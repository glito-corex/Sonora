// -- audio_engine.rs -- //
//
// Moteur de lecture (remplace python-vlc) :
//
//   fichier --Symphonia--> PCM f32 --mixage/rééchantillonnage--> tampon circulaire
//                                                                     |
//                                          cpal (WASAPI) <--- callback audio
//
// * un thread « décodeur » (lecture, seek, conversion de format),
// * un thread « sortie » qui possède le flux cpal et le reconstruit si le
//   périphérique disparaît ou si le périphérique par défaut change,
// * le callback audio ne fait aucune allocation après son premier appel et
//   ne prend aucun verrou contesté.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

use crate::format_conversion::Converter;
use crate::media_decoder::{MediaDecoder, SeekOutcome};

/// Capacité du tampon circulaire, en échantillons f32 (mémoire virtuelle :
/// seule la partie réellement remplie est touchée, ~400 ms d'audio).
const RING_CAPACITY: usize = 1 << 19;
/// Durée d'audio gardée d'avance dans le tampon.
const TARGET_BUFFER_MS: u64 = 400;
/// Après la dernière activité, le flux audio reste actif ce temps (fondu de pause).
const IDLE_GRACE_MS: u64 = 700;
/// Un flux inutilisé aussi longtemps est fermé (libère le périphérique et la mémoire).
const IDLE_CLOSE_SECONDS: u64 = 10;
/// Durée des fondus (anti-clic) en secondes.
const RAMP_SECONDS: f32 = 0.015;
/// Constante de temps du lissage de volume en secondes.
const GAIN_SMOOTH_SECONDS: f32 = 0.02;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum PlayState {
    Idle = 0,
    Opening = 1,
    Playing = 2,
    Ended = 3,
    Error = 4,
}

impl PlayState {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => PlayState::Opening,
            2 => PlayState::Playing,
            3 => PlayState::Ended,
            4 => PlayState::Error,
            _ => PlayState::Idle,
        }
    }
}

enum Cmd {
    Open(PathBuf),
    Seek(i64),
    Stop,
    Quit,
}

struct Shared {
    state: AtomicU8,
    paused: AtomicBool,
    gain_bits: AtomicU32,

    /// Position de lecture en trames de sortie (écrit uniquement par le callback audio).
    pos_frames: AtomicU64,
    /// Position (ms) à laquelle le callback doit se recaler lors du prochain vidage.
    pending_base_ms: AtomicI64,
    /// Position demandée par un seek en attente de prise en compte (-1 = aucune).
    seek_hint_ms: AtomicI64,
    flush_gen: AtomicU64,
    flush_ack: AtomicU64,
    /// Le décodeur a fini d'envoyer tout le flux dans le tampon.
    eof: AtomicBool,
    /// Compteurs d'événements (le moniteur compare à la dernière valeur vue).
    ended_count: AtomicU64,
    error_count: AtomicU64,
    length_ms: AtomicI64,

    out_rate: AtomicU32,
    out_channels: AtomicU32,
    out_epoch: AtomicU64,
    out_ready: AtomicBool,
    stream_failed: AtomicBool,

    quit: AtomicBool,
    last_error: Mutex<String>,

    /// Dernière activité utilisateur/moteur (ms depuis `epoch`) : garde le flux
    /// audio actif le temps des fondus, puis il est mis en pause.
    last_activity_ms: AtomicU64,
    epoch: Instant,
    audio_thread: Mutex<Option<Thread>>,
    event_waiter: Mutex<Option<Thread>>,
}

impl Shared {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(PlayState::Idle as u8),
            paused: AtomicBool::new(false),
            gain_bits: AtomicU32::new(0.8f32.to_bits()),
            pos_frames: AtomicU64::new(0),
            pending_base_ms: AtomicI64::new(0),
            seek_hint_ms: AtomicI64::new(-1),
            flush_gen: AtomicU64::new(0),
            flush_ack: AtomicU64::new(0),
            eof: AtomicBool::new(false),
            ended_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            length_ms: AtomicI64::new(0),
            out_rate: AtomicU32::new(48_000),
            out_channels: AtomicU32::new(2),
            out_epoch: AtomicU64::new(0),
            out_ready: AtomicBool::new(false),
            stream_failed: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            last_error: Mutex::new(String::new()),
            last_activity_ms: AtomicU64::new(0),
            epoch: Instant::now(),
            audio_thread: Mutex::new(None),
            event_waiter: Mutex::new(None),
        }
    }

    fn touch(&self) {
        self.last_activity_ms.store(self.epoch.elapsed().as_millis() as u64, Ordering::Release);
    }

    fn recently_active(&self, within_ms: u64) -> bool {
        let now = self.epoch.elapsed().as_millis() as u64;
        now.saturating_sub(self.last_activity_ms.load(Ordering::Acquire)) < within_ms
    }

    /// Le flux audio doit-il tourner ? (lecture en cours, ou fondu récent)
    fn needs_audio(&self) -> bool {
        let playing = matches!(self.state(), PlayState::Opening | PlayState::Playing)
            && !self.paused.load(Ordering::Acquire);
        playing || self.recently_active(IDLE_GRACE_MS)
    }

    fn wake_audio(&self) {
        if let Ok(g) = self.audio_thread.try_lock() {
            if let Some(t) = g.as_ref() {
                t.unpark();
            }
        }
    }

    /// Réveille le thread qui attend les événements (fin de morceau, erreur).
    fn wake_events(&self) {
        if let Ok(g) = self.event_waiter.try_lock() {
            if let Some(t) = g.as_ref() {
                t.unpark();
            }
        }
    }

    fn set_state(&self, s: PlayState) {
        self.state.store(s as u8, Ordering::Release);
    }

    fn state(&self) -> PlayState {
        PlayState::from_u8(self.state.load(Ordering::Acquire))
    }

    fn fail(&self, msg: impl Into<String>) {
        if let Ok(mut g) = self.last_error.lock() {
            *g = msg.into();
        }
        self.set_state(PlayState::Error);
        self.error_count.fetch_add(1, Ordering::AcqRel);
        self.wake_events();
    }
}

pub struct Engine {
    shared: Arc<Shared>,
    tx: Mutex<Sender<Cmd>>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl Engine {
    /// Démarre les threads audio. Ne échoue jamais : sans périphérique de sortie,
    /// l'ouverture d'un média renverra une erreur explicite.
    pub fn new() -> Engine {
        let shared = Arc::new(Shared::new());
        let rb = HeapRb::<f32>::new(RING_CAPACITY);
        let (prod, cons) = rb.split();
        let cons = Arc::new(Mutex::new(cons));

        let (tx, rx) = mpsc::channel::<Cmd>();

        let audio = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("sonora-audio".into())
                .spawn(move || audio_thread(shared, cons))
                .expect("thread audio")
        };
        if let Ok(mut g) = shared.audio_thread.lock() {
            *g = Some(audio.thread().clone());
        }

        let decoder = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("sonora-decoder".into())
                .spawn(move || decoder_thread(shared, rx, prod))
                .expect("thread décodeur")
        };

        Engine {
            shared,
            tx: Mutex::new(tx),
            threads: Mutex::new(vec![audio, decoder]),
        }
    }

    /// Le thread donné est réveillé à chaque fin de morceau ou erreur.
    pub fn set_event_waiter(&self, thread: Thread) {
        if let Ok(mut g) = self.shared.event_waiter.lock() {
            *g = Some(thread);
        }
    }

    fn send(&self, cmd: Cmd) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(cmd);
        }
    }

    /// Ouvre et démarre la lecture d'un fichier (équivalent set_media + play).
    pub fn open(&self, path: PathBuf) {
        self.shared.paused.store(false, Ordering::Release);
        self.shared.set_state(PlayState::Opening);
        self.shared.seek_hint_ms.store(0, Ordering::Release);
        self.shared.touch();
        self.shared.wake_audio();
        self.send(Cmd::Open(path));
    }

    pub fn stop(&self) {
        self.shared.touch();
        self.send(Cmd::Stop);
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Release);
        self.shared.touch();
        self.shared.wake_audio();
    }

    pub fn is_paused(&self) -> bool {
        self.shared.paused.load(Ordering::Acquire)
    }

    pub fn state(&self) -> PlayState {
        self.shared.state()
    }

    /// Équivalent de `player.is_playing()`.
    pub fn is_playing(&self) -> bool {
        matches!(self.state(), PlayState::Opening | PlayState::Playing) && !self.is_paused()
    }

    /// Gain linéaire (1.0 = 100 %).
    pub fn set_gain(&self, gain: f32) {
        let g = if gain.is_finite() { gain.clamp(0.0, 4.0) } else { 0.0 };
        self.shared.gain_bits.store(g.to_bits(), Ordering::Release);
    }

    /// Position de lecture en ms.
    pub fn position_ms(&self) -> i64 {
        let hint = self.shared.seek_hint_ms.load(Ordering::Acquire);
        if hint >= 0 {
            return hint;
        }
        let rate = self.shared.out_rate.load(Ordering::Acquire).max(1) as u64;
        (self.shared.pos_frames.load(Ordering::Acquire) * 1000 / rate) as i64
    }

    /// Durée en ms, 0 si inconnue.
    pub fn length_ms(&self) -> i64 {
        self.shared.length_ms.load(Ordering::Acquire)
    }

    pub fn seek_ms(&self, ms: i64) {
        let ms = ms.max(0);
        self.shared.seek_hint_ms.store(ms, Ordering::Release);
        self.shared.touch();
        self.shared.wake_audio();
        self.send(Cmd::Seek(ms));
    }

    pub fn ended_count(&self) -> u64 {
        self.shared.ended_count.load(Ordering::Acquire)
    }

    pub fn error_count(&self) -> u64 {
        self.shared.error_count.load(Ordering::Acquire)
    }

    pub fn last_error(&self) -> String {
        self.shared.last_error.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shared.quit.store(true, Ordering::Release);
        self.shared.wake_audio();
        self.send(Cmd::Quit);
        if let Ok(mut t) = self.threads.lock() {
            for h in t.drain(..) {
                let _ = h.join();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Thread de sortie audio (possède le flux cpal)
// ---------------------------------------------------------------------------

fn audio_thread(shared: Arc<Shared>, cons: Arc<Mutex<HeapCons<f32>>>) {
    let host = cpal::default_host();
    let mut stream: Option<Stream> = None;
    let mut running = false;
    let mut device_name: Option<String> = None;
    let mut last_attempt: Option<Instant> = None;
    let mut last_poll = Instant::now();
    let mut idle_since = Instant::now();

    while !shared.quit.load(Ordering::Acquire) {
        if shared.stream_failed.swap(false, Ordering::AcqRel) {
            stream = None; // périphérique perdu : l'ancien flux est détruit
            running = false;
            shared.out_ready.store(false, Ordering::Release);
        }

        let need = shared.needs_audio();
        let mut retry_soon = false;

        if need {
            // Création paresseuse : le périphérique n'est ouvert qu'à la première lecture.
            if stream.is_none() && last_attempt.map_or(true, |t| t.elapsed() >= Duration::from_millis(750)) {
                last_attempt = Some(Instant::now());
                match build_stream(&host, &shared, cons.clone()) {
                    Ok((s, name)) => {
                        stream = Some(s);
                        device_name = Some(name);
                        shared.out_ready.store(true, Ordering::Release);
                        shared.out_epoch.fetch_add(1, Ordering::AcqRel);
                        last_poll = Instant::now();
                    }
                    Err(e) => {
                        if let Ok(mut g) = shared.last_error.lock() {
                            *g = format!("sortie audio : {e}");
                        }
                    }
                }
            }
            retry_soon = stream.is_none();

            if let Some(s) = stream.as_ref() {
                if !running {
                    if s.play().is_ok() {
                        running = true;
                    } else {
                        stream = None;
                        shared.out_ready.store(false, Ordering::Release);
                    }
                }
            }

            // Le périphérique par défaut a-t-il changé (casque branché...) ?
            if running && last_poll.elapsed() >= Duration::from_secs(1) {
                last_poll = Instant::now();
                let now = host.default_output_device().and_then(|d| d.name().ok());
                if now.is_some() && now != device_name {
                    stream = None;
                    running = false;
                    shared.out_ready.store(false, Ordering::Release);
                    last_attempt = None;
                    retry_soon = true;
                }
            }
            idle_since = Instant::now();
        } else {
            if running {
                // Plus rien à jouer : on arrête les callbacks (zéro CPU en pause).
                if let Some(s) = stream.as_ref() {
                    let _ = s.pause();
                }
                running = false;
                idle_since = Instant::now();
            }
            // Inutilisé depuis longtemps : on rend le périphérique.
            if stream.is_some() && idle_since.elapsed() >= Duration::from_secs(IDLE_CLOSE_SECONDS) {
                stream = None;
                shared.out_ready.store(false, Ordering::Release);
                crate::memory_trimming::trim_working_set();
            }
        }

        // Attente : réveillé par l'API (ouverture, seek, pause) ou par une erreur de flux.
        if need && running {
            thread::park_timeout(Duration::from_millis(500));
        } else if retry_soon {
            thread::park_timeout(Duration::from_millis(250));
        } else if need {
            thread::park_timeout(Duration::from_millis(IDLE_GRACE_MS));
        } else if stream.is_some() {
            let left = Duration::from_secs(IDLE_CLOSE_SECONDS).saturating_sub(idle_since.elapsed());
            thread::park_timeout(left.max(Duration::from_millis(50)));
        } else {
            thread::park();
        }
    }
    drop(stream);
}

fn build_stream(
    host: &cpal::Host,
    shared: &Arc<Shared>,
    cons: Arc<Mutex<HeapCons<f32>>>,
) -> Result<(Stream, String), String> {
    let device = host
        .default_output_device()
        .ok_or_else(|| "aucun périphérique de sortie audio".to_string())?;
    let name = device.name().unwrap_or_default();
    let supported = device
        .default_output_config()
        .map_err(|e| format!("configuration du périphérique : {e}"))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.config();

    shared.out_rate.store(config.sample_rate.0, Ordering::Release);
    shared.out_channels.store(config.channels as u32, Ordering::Release);

    let stream = match format {
        SampleFormat::F32 => make_stream::<f32>(&device, &config, shared.clone(), cons),
        SampleFormat::I16 => make_stream::<i16>(&device, &config, shared.clone(), cons),
        SampleFormat::U16 => make_stream::<u16>(&device, &config, shared.clone(), cons),
        SampleFormat::I32 => make_stream::<i32>(&device, &config, shared.clone(), cons),
        SampleFormat::I8 => make_stream::<i8>(&device, &config, shared.clone(), cons),
        SampleFormat::U8 => make_stream::<u8>(&device, &config, shared.clone(), cons),
        SampleFormat::F64 => make_stream::<f64>(&device, &config, shared.clone(), cons),
        other => Err(format!("format d'échantillon non géré : {other:?}")),
    }?;
    Ok((stream, name))
}

fn make_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    shared: Arc<Shared>,
    cons: Arc<Mutex<HeapCons<f32>>>,
) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = (config.channels as usize).max(1);
    let rate = config.sample_rate.0 as f32;
    let env_step = 1.0 / (RAMP_SECONDS * rate);
    let gain_coef = 1.0 - (-1.0 / (GAIN_SMOOTH_SECONDS * rate)).exp();

    let mut local_gen = shared.flush_gen.load(Ordering::Acquire);
    let mut env = 0.0f32; // fondu (0 = silence, 1 = plein)
    let mut gain_cur = f32::from_bits(shared.gain_bits.load(Ordering::Acquire));
    let mut scratch: Vec<f32> = Vec::with_capacity(16_384);
    let mut eof_reported = false;

    let shared_err = shared.clone();
    let err_fn = move |_e: cpal::StreamError| {
        shared_err.stream_failed.store(true, Ordering::Release);
        shared_err.wake_audio();
    };

    let cb = move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        let silence = T::from_sample(0.0f32);

        let mut cons = match cons.try_lock() {
            Ok(c) => c,
            Err(_) => {
                data.fill(silence);
                return;
            }
        };

        // Demande de vidage (seek / nouveau média).
        let g = shared.flush_gen.load(Ordering::Acquire);
        if g != local_gen {
            cons.clear();
            let base_ms = shared.pending_base_ms.load(Ordering::Acquire);
            let rate_now = shared.out_rate.load(Ordering::Acquire).max(1) as i64;
            shared.pos_frames.store((base_ms.max(0) * rate_now / 1000) as u64, Ordering::Release);
            let _ = shared.seek_hint_ms.compare_exchange(base_ms, -1, Ordering::AcqRel, Ordering::Acquire);
            env = 0.0;
            eof_reported = false;
            local_gen = g;
            shared.flush_ack.store(g, Ordering::Release);
        }

        let total_frames = data.len() / channels;
        let paused = shared.paused.load(Ordering::Acquire);
        let target_env = if paused { 0.0f32 } else { 1.0f32 };
        let gain_target = f32::from_bits(shared.gain_bits.load(Ordering::Acquire));

        // En pause : on ne consomme que le temps nécessaire au fondu de sortie.
        let frames_wanted = if paused {
            ((env / env_step).ceil() as usize).min(total_frames)
        } else {
            total_frames
        };

        let mut frames_done = 0usize;
        if frames_wanted > 0 {
            scratch.resize(frames_wanted * channels, 0.0);
            let got = cons.pop_slice(&mut scratch[..]);
            frames_done = got / channels;

            for f in 0..frames_done {
                if env < target_env {
                    env = (env + env_step).min(target_env);
                } else if env > target_env {
                    env = (env - env_step).max(target_env);
                }
                gain_cur += (gain_target - gain_cur) * gain_coef;
                let k = env * gain_cur;
                let base = f * channels;
                for c in 0..channels {
                    let v = (scratch[base + c] * k).clamp(-1.0, 1.0);
                    data[base + c] = T::from_sample(v);
                }
            }
            if frames_done > 0 {
                shared.pos_frames.fetch_add(frames_done as u64, Ordering::AcqRel);
            }
        }

        // Le reste du tampon de sortie : silence.
        for s in data[frames_done * channels..].iter_mut() {
            *s = silence;
        }
        if frames_done == 0 {
            gain_cur += (gain_target - gain_cur) * gain_coef;
            if paused {
                env = 0.0;
            }
        }

        // Fin de flux : tout a été envoyé et le tampon est vide.
        if shared.eof.load(Ordering::Acquire) {
            if cons.is_empty() && !eof_reported {
                eof_reported = true;
                shared.set_state(PlayState::Ended);
                shared.ended_count.fetch_add(1, Ordering::AcqRel);
                shared.wake_events();
            }
        } else {
            eof_reported = false;
        }
    };

    device
        .build_output_stream(config, cb, err_fn, None)
        .map_err(|e| format!("création du flux audio : {e}"))
}

// ---------------------------------------------------------------------------
// Thread décodeur
// ---------------------------------------------------------------------------

struct Decoding {
    media: MediaDecoder,
    conv: Option<Converter>,
    /// Échantillons convertis pas encore poussés dans le tampon circulaire.
    pending: Vec<f32>,
    pending_off: usize,
    /// Le média est entièrement décodé (reste éventuellement `pending`).
    finished: bool,
    /// `eof` publié (tout est dans le tampon circulaire).
    eof_sent: bool,
}

fn flush(shared: &Shared, base_ms: i64) {
    shared.pending_base_ms.store(base_ms, Ordering::Release);
    let g = shared.flush_gen.fetch_add(1, Ordering::AcqRel) + 1;
    if !shared.out_ready.load(Ordering::Acquire) {
        return;
    }
    let t0 = Instant::now();
    while shared.flush_ack.load(Ordering::Acquire) != g
        && t0.elapsed() < Duration::from_millis(500)
        && !shared.quit.load(Ordering::Acquire)
    {
        thread::sleep(Duration::from_millis(1));
    }
}

/// Ce que le thread décodeur doit faire avant son prochain tour de boucle.
enum Wait {
    /// Du travail est prêt : on enchaîne.
    Nothing,
    /// Rien à faire avant un moment (tampon plein...) : sommeil interrompu par toute commande.
    For(Duration),
    /// Aucun travail : on dort jusqu'à la prochaine commande (zéro CPU).
    Forever,
}

fn decoder_thread(shared: Arc<Shared>, rx: Receiver<Cmd>, mut prod: HeapProd<f32>) {
    let mut cur: Option<Decoding> = None;
    let mut seen_epoch = shared.out_epoch.load(Ordering::Acquire);
    let mut wait = Wait::Forever;

    'main: loop {
        if shared.quit.load(Ordering::Acquire) {
            break;
        }

        // ---- commandes -------------------------------------------------------
        let first = match wait {
            Wait::Nothing => None,
            Wait::For(d) => match rx.recv_timeout(d) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            Wait::Forever => match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => break,
            },
        };
        let cmds: Vec<Cmd> = first.into_iter().chain(rx.try_iter()).collect();

        let n = cmds.len();
        for (i, cmd) in cmds.into_iter().enumerate() {
            // Un seek est inutile si une commande suivante le remplace.
            if let Cmd::Seek(_) = cmd {
                if i + 1 < n {
                    continue;
                }
            }
            match cmd {
                Cmd::Quit => break 'main,
                Cmd::Stop => {
                    cur = None;
                    shared.eof.store(false, Ordering::Release);
                    shared.length_ms.store(0, Ordering::Release);
                    flush(&shared, 0);
                    shared.set_state(PlayState::Idle);
                }
                Cmd::Open(path) => {
                    cur = None;
                    shared.eof.store(false, Ordering::Release);
                    shared.length_ms.store(0, Ordering::Release);
                    shared.set_state(PlayState::Opening);

                    // Le flux audio est créé à la demande : on lui laisse le temps de démarrer.
                    let t0 = Instant::now();
                    while !shared.out_ready.load(Ordering::Acquire)
                        && t0.elapsed() < Duration::from_millis(1500)
                        && !shared.quit.load(Ordering::Acquire)
                    {
                        shared.wake_audio();
                        thread::sleep(Duration::from_millis(10));
                    }
                    if !shared.out_ready.load(Ordering::Acquire) {
                        let detail = shared.last_error.lock().map(|g| g.clone()).unwrap_or_default();
                        let msg = if detail.is_empty() {
                            "aucun périphérique de sortie audio disponible".to_string()
                        } else {
                            detail
                        };
                        shared.fail(msg);
                        continue;
                    }

                    match MediaDecoder::open(&path) {
                        Ok(media) => {
                            shared.length_ms.store(media.duration_ms(), Ordering::Release);
                            flush(&shared, 0);
                            seen_epoch = shared.out_epoch.load(Ordering::Acquire);
                            shared.set_state(PlayState::Playing);
                            cur = Some(Decoding {
                                media,
                                conv: None,
                                pending: Vec::new(),
                                pending_off: 0,
                                finished: false,
                                eof_sent: false,
                            });
                        }
                        Err(e) => shared.fail(e),
                    }
                }
                Cmd::Seek(ms) => {
                    if let Some(d) = cur.as_mut() {
                        do_seek(&shared, d, ms);
                    } else {
                        shared.seek_hint_ms.store(-1, Ordering::Release);
                    }
                }
            }
        }

        // ---- un pas de travail ------------------------------------------------
        wait = match cur.as_mut() {
            None => Wait::Forever,
            Some(d) => {
                let r = decode_step(&shared, d, &mut prod, &mut seen_epoch);
                if let Err(e) = r {
                    shared.fail(e);
                    cur = None;
                    Wait::Forever
                } else {
                    r.unwrap_or(Wait::Forever)
                }
            }
        };
    }
}

fn decode_step(
    shared: &Shared,
    d: &mut Decoding,
    prod: &mut HeapProd<f32>,
    seen_epoch: &mut u64,
) -> Result<Wait, String> {
    // Changement de périphérique de sortie : on reprend à la position courante.
    let epoch = shared.out_epoch.load(Ordering::Acquire);
    if epoch != *seen_epoch && shared.out_ready.load(Ordering::Acquire) {
        *seen_epoch = epoch;
        let rate = shared.out_rate.load(Ordering::Acquire).max(1) as u64;
        let pos = (shared.pos_frames.load(Ordering::Acquire) * 1000 / rate) as i64;
        do_seek(shared, d, pos);
        return Ok(Wait::Nothing);
    }

    if d.eof_sent {
        return Ok(Wait::Forever);
    }

    // Envoi du tampon en attente.
    if d.pending_off < d.pending.len() {
        let pushed = prod.push_slice(&d.pending[d.pending_off..]);
        d.pending_off += pushed;
        if d.pending_off < d.pending.len() {
            return Ok(Wait::For(Duration::from_millis(10)));
        }
        d.pending.clear();
        d.pending_off = 0;
    }

    if d.finished {
        shared.eof.store(true, Ordering::Release);
        d.eof_sent = true;
        shared.wake_audio();
        return Ok(Wait::Forever);
    }

    // Régulation : on garde ~TARGET_BUFFER_MS d'audio d'avance, puis on dort.
    let rate = shared.out_rate.load(Ordering::Acquire).max(1) as u64;
    let ch = shared.out_channels.load(Ordering::Acquire).max(1) as u64;
    let target = ((rate * ch * TARGET_BUFFER_MS / 1000) as usize).min(RING_CAPACITY / 2).max(4096);
    let filled = prod.occupied_len();
    if filled >= target {
        let excess_ms = ((filled - target) as u64 * 1000) / (rate * ch);
        let sleep_ms = (excess_ms + 5).clamp(5, 100);
        return Ok(Wait::For(Duration::from_millis(sleep_ms)));
    }

    // Décodage d'un bloc.
    match d.media.next_chunk()? {
        Some(chunk) => {
            let out_rate = rate as u32;
            let out_ch = ch as usize;
            let rebuild = d
                .conv
                .as_ref()
                .map_or(true, |c| !c.matches(chunk.rate, chunk.layout, chunk.channels, out_rate, out_ch));
            if rebuild {
                if let Some(old) = d.conv.as_mut() {
                    old.finish(&mut d.pending);
                }
                d.conv = Some(Converter::new(chunk.rate, chunk.layout, chunk.channels, out_rate, out_ch)?);
            }
            let samples = d.media.samples();
            let slice = &samples[chunk.start_frame * chunk.channels..chunk.end_frame * chunk.channels];
            if let Some(conv) = d.conv.as_mut() {
                conv.process(slice, &mut d.pending);
            }
        }
        None => {
            if let Some(conv) = d.conv.as_mut() {
                conv.finish(&mut d.pending);
            }
            d.finished = true;
        }
    }
    Ok(Wait::Nothing)
}

fn do_seek(shared: &Shared, d: &mut Decoding, ms: i64) {
    let length = shared.length_ms.load(Ordering::Acquire);
    let ms = ms.max(0);

    let past_end = length > 0 && ms >= length;
    let outcome = if past_end { Ok(SeekOutcome::PastEnd) } else { d.media.seek_ms(ms) };

    d.pending.clear();
    d.pending_off = 0;
    d.conv = None;

    match outcome {
        Ok(SeekOutcome::Done) => {
            d.finished = false;
            d.eof_sent = false;
            shared.eof.store(false, Ordering::Release);
            flush(shared, ms);
            shared.set_state(PlayState::Playing);
        }
        Ok(SeekOutcome::PastEnd) => {
            // Position au-delà de la fin : le morceau est considéré terminé.
            d.finished = true;
            d.eof_sent = true;
            let end = if length > 0 { length } else { ms };
            flush(shared, end);
            shared.eof.store(true, Ordering::Release);
        }
        Err(e) => {
            shared.seek_hint_ms.store(-1, Ordering::Release);
            shared.fail(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test d'intégration : nécessite un périphérique de sortie (réel ou virtuel)
    /// et SONORA_TESTDATA. Ignoré sinon.
    #[test]
    fn plays_mp4_to_the_end_and_seeks() {
        let Some(dir) = std::env::var_os("SONORA_TESTDATA").map(PathBuf::from) else { return };
        let eng = Engine::new();
        if !eng.shared.out_ready.load(Ordering::Acquire) {
            eprintln!("pas de périphérique audio : test ignoré");
            return;
        }
        eng.set_gain(0.5);
        let ended0 = eng.ended_count();
        eng.open(dir.join("video_aac_44k.mp4"));

        // la durée doit être connue rapidement
        let t0 = Instant::now();
        while eng.length_ms() == 0 && t0.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(10));
        }
        let len = eng.length_ms();
        assert!((len - 3000).abs() < 150, "durée {len}");

        // seek au milieu puis fin
        eng.seek_ms(1500);
        assert_eq!(eng.position_ms().max(1500) >= 1500, true);

        let t0 = Instant::now();
        while eng.ended_count() == ended0 && t0.elapsed() < Duration::from_secs(20) {
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(eng.ended_count(), ended0 + 1, "fin de morceau non signalée (état {:?})", eng.state());
        assert_eq!(eng.state(), PlayState::Ended);
        assert!(!eng.is_playing());

        // seek en arrière après la fin : rejoue puis se termine à nouveau
        eng.seek_ms(2500);
        let t0 = Instant::now();
        while eng.ended_count() == ended0 + 1 && t0.elapsed() < Duration::from_secs(20) {
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(eng.ended_count(), ended0 + 2);

        // seek au-delà de la fin => terminé aussi
        eng.seek_ms(10_000_000);
        let t0 = Instant::now();
        while eng.ended_count() == ended0 + 2 && t0.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(eng.ended_count(), ended0 + 3);

        // fichier illisible : erreur signalée, pas de blocage
        let e0 = eng.error_count();
        eng.open(dir.join("does_not_exist.mp3"));
        let t0 = Instant::now();
        while eng.error_count() == e0 && t0.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(eng.error_count(), e0 + 1);
        assert_eq!(eng.state(), PlayState::Error);

        // on peut relire ensuite
        eng.open(dir.join("tone.wav"));
        let t0 = Instant::now();
        while eng.state() != PlayState::Ended && t0.elapsed() < Duration::from_secs(20) {
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(eng.state(), PlayState::Ended);
    }
}
