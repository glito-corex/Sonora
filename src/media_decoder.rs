// -- media_decoder.rs -- //
//
// Ouverture et décodage d'un fichier média avec Symphonia (100 % Rust, sans ffmpeg).
//
// Extraction du son d'un .mp4 : le conteneur ISO-MP4 peut contenir une piste
// vidéo (ignorée), une ou plusieurs pistes audio, des sous-titres... On
// sélectionne donc la première piste dont le codec est réellement décodable,
// et on ignore tous les paquets qui n'appartiennent pas à cette piste.

use std::fs::File;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use symphonia::core::audio::{Channels, SampleBuffer};
use symphonia::core::codecs::{
    CodecParameters, CodecType, Decoder, DecoderOptions, CODEC_TYPE_NULL,
};
use symphonia::core::errors::{Error as SymError, SeekErrorKind};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

use crate::codec_registry::codec_registry;

/// Nombre maximal d'erreurs de décodage consécutives tolérées (paquets corrompus ignorés).
const MAX_CONSECUTIVE_DECODE_ERRORS: u32 = 200;
/// Amorce décodée avant une position de seek puis écartée.
const PRE_ROLL_SECONDS: f64 = 0.15;

/// Description d'un bloc d'échantillons décodés (les données restent dans `Media::samples`).
#[derive(Clone, Copy, Debug)]
pub struct DecodedChunk {
    pub rate: u32,
    pub channels: usize,
    pub layout: Channels,
    /// Première trame valide du tampon (rognage gapless / saut après seek).
    pub start_frame: usize,
    /// Trame de fin (exclue).
    pub end_frame: usize,
}

pub enum SeekOutcome {
    Done,
    /// La position demandée dépasse la fin du flux.
    PastEnd,
}

pub struct MediaDecoder {
    path: PathBuf,
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    nominal_rate: u32,
    duration_ms: i64,
    #[allow(dead_code)]
    codec_name: String,
    sample_buf: Option<SampleBuffer<f32>>,
    buf_rate: u32,
    buf_channels: usize,
    /// Après un seek précis : instant (en secondes) avant lequel on jette l'audio décodé.
    discard_before: Option<f64>,
    decode_errors: u32,
}

fn codec_label(codec: CodecType) -> String {
    use symphonia::core::codecs::*;
    let known: &[(CodecType, &str)] = &[
        (CODEC_TYPE_OPUS, "Opus"),
        (CODEC_TYPE_AAC, "AAC"),
        (CODEC_TYPE_MP3, "MP3"),
        (CODEC_TYPE_MP2, "MP2"),
        (CODEC_TYPE_MP1, "MP1"),
        (CODEC_TYPE_VORBIS, "Vorbis"),
        (CODEC_TYPE_FLAC, "FLAC"),
        (CODEC_TYPE_ALAC, "ALAC"),
        (CODEC_TYPE_EAC3, "E-AC-3"),
        (CODEC_TYPE_AC4, "AC-4"),
        (CODEC_TYPE_SPEEX, "Speex"),
        (CODEC_TYPE_DCA, "DTS"),
        (CODEC_TYPE_WAVPACK, "WavPack"),
        (CODEC_TYPE_TTA, "TTA"),
        (CODEC_TYPE_ATRAC1, "ATRAC1"),
        (CODEC_TYPE_MUSEPACK, "Musepack"),
        (CODEC_TYPE_MONKEYS_AUDIO, "Monkey's Audio"),
        (CODEC_TYPE_WMA, "WMA"),
        (CODEC_TYPE_ATRAC3, "ATRAC3"),
    ];
    if let Some((_, name)) = known.iter().find(|(c, _)| *c == codec) {
        return (*name).to_string();
    }
    if let Some(d) = codec_registry().get_codec(codec) {
        return d.long_name.to_string();
    }
    format!("{codec:?}")
}

fn secs_of(time: Time) -> f64 {
    time.seconds as f64 + time.frac
}

/// Choisit la piste audio à décoder et construit son décodeur.
/// La piste par défaut est essayée en premier, puis les autres, dans l'ordre.
fn select_track(format: &dyn FormatReader) -> Result<(u32, CodecParameters, Box<dyn Decoder>), String> {
    let codecs = codec_registry();
    let default_id = format.default_track().map(|t| t.id);

    let mut tracks: Vec<_> = format.tracks().iter().collect();
    tracks.sort_by_key(|t| Some(t.id) != default_id); // tri stable : piste par défaut d'abord

    let mut unsupported: Vec<String> = Vec::new();
    let mut had_candidate = false;

    for t in tracks {
        let p = &t.codec_params;
        if p.codec == CODEC_TYPE_NULL {
            // Piste vidéo / sous-titres / données : jamais décodée, simplement ignorée.
            continue;
        }
        had_candidate = true;
        match codecs.make(p, &DecoderOptions::default()) {
            Ok(dec) => return Ok((t.id, p.clone(), dec)),
            Err(e) => unsupported.push(format!("{} ({e})", codec_label(p.codec))),
        }
    }

    if !had_candidate {
        Err("aucune piste audio dans ce fichier".to_string())
    } else {
        Err(format!(
            "codec audio non pris en charge : {}",
            unsupported.join(", ")
        ))
    }
}

impl MediaDecoder {
    pub fn open(path: &Path) -> Result<MediaDecoder, String> {
        let file = File::open(path).map_err(|e| format!("ouverture impossible : {e}"))?;
        let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());

        // Indice d'extension. ".corex" n'est pas un format connu : on laisse
        // Symphonia reconnaître le conteneur par son contenu.
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext = ext.to_ascii_lowercase();
            if ext != "corex" {
                hint.with_extension(&ext);
            }
        }

        let fmt_opts = FormatOptions { enable_gapless: true, ..Default::default() };
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &fmt_opts, &MetadataOptions::default())
            .map_err(|e| match e {
                SymError::Unsupported(_) => "format de fichier non reconnu".to_string(),
                other => format!("lecture impossible : {other}"),
            })?;

        let format = probed.format;
        let (track_id, params, decoder) = select_track(format.as_ref())?;

        let time_base = params.time_base;
        let nominal_rate = params.sample_rate.unwrap_or(0);
        let duration_ms = match (params.n_frames, time_base, params.sample_rate) {
            (Some(n), Some(tb), _) => (secs_of(tb.calc_time(n)) * 1000.0).round() as i64,
            (Some(n), None, Some(sr)) if sr > 0 => ((n as f64 / sr as f64) * 1000.0).round() as i64,
            _ => 0,
        };

        Ok(MediaDecoder {
            path: path.to_path_buf(),
            format,
            decoder,
            track_id,
            time_base,
            nominal_rate,
            duration_ms,
            codec_name: codec_label(params.codec),
            sample_buf: None,
            buf_rate: 0,
            buf_channels: 0,
            discard_before: None,
            decode_errors: 0,
        })
    }

    /// Durée en millisecondes, 0 si inconnue.
    pub fn duration_ms(&self) -> i64 {
        self.duration_ms
    }

    #[allow(dead_code)]
    pub fn codec_name(&self) -> &str {
        &self.codec_name
    }

    /// Échantillons entrelacés du dernier bloc décodé.
    pub fn samples(&self) -> &[f32] {
        self.sample_buf.as_ref().map_or(&[], |b| b.samples())
    }

    fn ts_to_secs(&self, ts: u64) -> f64 {
        match self.time_base {
            Some(tb) => secs_of(tb.calc_time(ts)),
            None if self.nominal_rate > 0 => ts as f64 / self.nominal_rate as f64,
            None => 0.0,
        }
    }

    /// Décode le prochain bloc audio de la piste sélectionnée.
    /// `Ok(None)` = fin du flux.
    pub fn next_chunk(&mut self) -> Result<Option<DecodedChunk>, String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => return Ok(None),
                Err(SymError::ResetRequired) => {
                    // La liste des pistes a changé (flux chaînés) : on réinitialise.
                    let (id, params, dec) = select_track(self.format.as_ref())?;
                    self.track_id = id;
                    self.time_base = params.time_base;
                    self.nominal_rate = params.sample_rate.unwrap_or(0);
                    self.decoder = dec;
                    self.discard_before = None;
                    continue;
                }
                Err(e) => return Err(format!("lecture du flux : {e}")),
            };

            // Ignore tout ce qui n'est pas la piste audio choisie (vidéo, sous-titres, autre piste audio).
            if packet.track_id() != self.track_id {
                continue;
            }

            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => {
                    // Paquet corrompu : on l'ignore et on continue.
                    self.decode_errors += 1;
                    if self.decode_errors > MAX_CONSECUTIVE_DECODE_ERRORS {
                        return Err("trop d'erreurs de décodage consécutives".to_string());
                    }
                    continue;
                }
                Err(SymError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(format!("décodage : {e}")),
            };
            self.decode_errors = 0;

            let spec = *decoded.spec();
            let channels = spec.channels.count();
            let frames = decoded.frames();
            if frames == 0 || channels == 0 {
                continue;
            }

            // (Ré)allocation du tampon si le format ou la taille ont changé.
            let needs_new = match &self.sample_buf {
                None => true,
                Some(b) => {
                    self.buf_rate != spec.rate
                        || self.buf_channels != channels
                        || b.capacity() < decoded.capacity() * channels
                }
            };
            if needs_new {
                self.sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
                self.buf_rate = spec.rate;
                self.buf_channels = channels;
            }
            let buf = self.sample_buf.as_mut().expect("tampon alloué");
            buf.copy_interleaved_ref(decoded);

            // Rognage gapless (amorce / remplissage de l'encodeur).
            let mut start = (packet.trim_start as usize).min(frames);
            let end = frames.saturating_sub(packet.trim_end as usize).max(start);

            // Seek précis : on décode depuis le point d'accès précédent, puis on
            // jette tout ce qui précède l'instant demandé.
            if let Some(target) = self.discard_before {
                let pkt_start = self.ts_to_secs(packet.ts());
                let rate = spec.rate as f64;
                let pkt_end = pkt_start + frames as f64 / rate;
                if pkt_end <= target {
                    continue;
                }
                if pkt_start < target {
                    let skip = ((target - pkt_start) * rate).round() as usize;
                    start = start.max(skip.min(frames));
                }
                self.discard_before = None;
            }

            if start >= end {
                continue;
            }

            return Ok(Some(DecodedChunk {
                rate: spec.rate,
                channels,
                layout: spec.channels,
                start_frame: start,
                end_frame: end,
            }));
        }
    }

    /// Déplace la lecture à `ms` millisecondes (seek précis à l'échantillon près).
    pub fn seek_ms(&mut self, ms: i64) -> Result<SeekOutcome, String> {
        let ms = ms.max(0);
        let secs = ms as f64 / 1000.0;
        // On se place un peu avant la cible : les codecs à recouvrement (AAC, MP3,
        // Vorbis, Opus) ont besoin de quelques trames d'amorce pour être exacts.
        let seek_secs = (secs - PRE_ROLL_SECONDS).max(0.0);
        let time = Time::new(seek_secs.trunc() as u64, seek_secs.fract());

        let res = self.format.seek(
            SeekMode::Accurate,
            SeekTo::Time { time, track_id: Some(self.track_id) },
        );

        match res {
            Ok(_) => {
                self.decoder.reset();
                self.discard_before = if ms > 0 { Some(secs) } else { None };
                self.decode_errors = 0;
                Ok(SeekOutcome::Done)
            }
            Err(SymError::SeekError(SeekErrorKind::OutOfRange)) => Ok(SeekOutcome::PastEnd),
            // Certains démuxeurs (mp4) signalent un seek après la fin par un EOF.
            Err(SymError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => Ok(SeekOutcome::PastEnd),
            Err(SymError::SeekError(SeekErrorKind::Unseekable))
            | Err(SymError::SeekError(SeekErrorKind::ForwardOnly))
            | Err(SymError::Unsupported(_)) => {
                // Repli : on rouvre le fichier et on décode jusqu'à la position voulue.
                let mut fresh = MediaDecoder::open(&self.path)?;
                fresh.discard_before = if ms > 0 { Some(secs) } else { None };
                *self = fresh;
                Ok(SeekOutcome::Done)
            }
            Err(e) => Err(format!("déplacement impossible : {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_dir() -> Option<PathBuf> {
        std::env::var_os("SONORA_TESTDATA").map(PathBuf::from)
    }

    /// Décode tout le fichier ; renvoie (trames, fréquence, canaux, crête).
    fn decode_all(path: &Path) -> (usize, u32, usize, f32) {
        let mut m = MediaDecoder::open(path).expect("ouverture");
        let (mut frames, mut rate, mut ch, mut peak) = (0usize, 0u32, 0usize, 0.0f32);
        while let Some(c) = m.next_chunk().expect("décodage") {
            frames += c.end_frame - c.start_frame;
            rate = c.rate;
            ch = c.channels;
            let s = &m.samples()[c.start_frame * c.channels..c.end_frame * c.channels];
            peak = s.iter().fold(peak, |a, b| a.max(b.abs()));
        }
        (frames, rate, ch, peak)
    }

    #[test]
    fn mp4_with_video_track_yields_audio() {
        let Some(dir) = data_dir() else { return };
        for (name, rate, ch, secs) in [
            ("video_aac_44k.mp4", 44100u32, 2usize, 3.0f64),
            ("video_aac_48k_faststart.mp4", 48000, 2, 3.0),
            ("mono_aac_22k.mp4", 22050, 1, 3.0),
            ("video_aac_48k.mp4", 48000, 2, 5.0),
        ] {
            let (frames, r, c, peak) = decode_all(&dir.join(name));
            assert_eq!(r, rate, "{name}");
            assert_eq!(c, ch, "{name}");
            let dur = frames as f64 / r as f64;
            assert!((dur - secs).abs() < 0.15, "{name}: durée {dur}");
            assert!(peak > 0.4 && peak < 0.6, "{name}: crête {peak}");
            let m = MediaDecoder::open(&dir.join(name)).unwrap();
            assert!((m.duration_ms() as f64 / 1000.0 - secs).abs() < 0.15, "{name}: durée annoncée {}", m.duration_ms());
            assert_eq!(m.codec_name(), "AAC");
        }
    }

    #[test]
    fn wav_decodes_exactly() {
        let Some(dir) = data_dir() else { return };
        let (frames, r, c, _) = decode_all(&dir.join("tone.wav"));
        assert_eq!((frames, r, c), (88200, 44100, 2));
    }

    #[test]
    fn seek_is_sample_accurate_enough() {
        let Some(dir) = data_dir() else { return };
        let total = decode_all(&dir.join("video_aac_48k.mp4")).0;
        let mut m = MediaDecoder::open(&dir.join("video_aac_48k.mp4")).unwrap();
        assert!(matches!(m.seek_ms(2000).unwrap(), SeekOutcome::Done));
        let mut after = 0usize;
        while let Some(c) = m.next_chunk().unwrap() {
            after += c.end_frame - c.start_frame;
        }
        let expected = total as i64 - 2 * 48000;
        assert!((after as i64 - expected).abs() < 48, "après seek: {after} attendu ~{expected}");

        // seek hors limites
        assert!(matches!(m.seek_ms(60_000).unwrap(), SeekOutcome::PastEnd));
        // seek en arrière après la fin
        assert!(matches!(m.seek_ms(0).unwrap(), SeekOutcome::Done));
        assert!(m.next_chunk().unwrap().is_some());
    }

    #[test]
    fn unknown_or_missing_files_report_errors() {
        assert!(MediaDecoder::open(Path::new("/nonexistent/zzz.mp3")).is_err());
        let p = std::env::temp_dir().join("sonora_garbage.mp4");
        std::fs::write(&p, vec![7u8; 5000]).unwrap();
        assert!(MediaDecoder::open(&p).is_err());
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod opus_tests {
    use super::*;

    fn data_dir() -> Option<PathBuf> {
        std::env::var_os("SONORA_TESTDATA").map(PathBuf::from)
    }

    fn decode_all(path: &Path) -> (usize, u32, usize, f32, f32) {
        let mut m = MediaDecoder::open(path).expect("ouverture");
        let (mut frames, mut rate, mut ch, mut peak, mut first_peak) = (0usize, 0u32, 0usize, 0.0f32, -1.0f32);
        while let Some(c) = m.next_chunk().expect("décodage") {
            frames += c.end_frame - c.start_frame;
            rate = c.rate;
            ch = c.channels;
            let s = &m.samples()[c.start_frame * c.channels..c.end_frame * c.channels];
            let p = s.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            if first_peak < 0.0 {
                first_peak = p;
            }
            peak = peak.max(p);
        }
        (frames, rate, ch, peak, first_peak)
    }

    #[test]
    fn opus_in_every_container_decodes() {
        let Some(dir) = data_dir() else { return };
        for (name, channels) in [
            ("opus_stereo.ogg", 2usize),
            ("opus_stereo.webm", 2),
            ("opus_stereo.mkv", 2),
            ("opus_stereo.mp4", 2),
            ("opus_mono.ogg", 1),
            ("opus_51.ogg", 6),
        ] {
            let (frames, rate, ch, peak, _) = decode_all(&dir.join(name));
            let dur = frames as f64 / rate as f64;
            eprintln!("{name}: {frames} trames, {rate} Hz, {ch} ch, durée {dur:.4}s, crête {peak:.3}");
            assert_eq!(rate, 48000, "{name}");
            assert_eq!(ch, channels, "{name}");
            assert!((dur - 3.0).abs() < 0.05, "{name}: durée {dur}");
            assert!(peak > 0.35 && peak < 0.65, "{name}: crête {peak}");
            let m = MediaDecoder::open(&dir.join(name)).unwrap();
            assert_eq!(m.codec_name(), "Opus", "{name}");
        }
    }

    #[test]
    fn opus_seek_is_accurate() {
        let Some(dir) = data_dir() else { return };
        for name in ["opus_stereo.ogg", "opus_stereo.webm", "opus_stereo.mp4"] {
            let total = decode_all(&dir.join(name)).0;
            let mut m = MediaDecoder::open(&dir.join(name)).unwrap();
            assert!(matches!(m.seek_ms(1500).unwrap(), SeekOutcome::Done), "{name}");
            let mut after = 0usize;
            while let Some(c) = m.next_chunk().unwrap() {
                after += c.end_frame - c.start_frame;
            }
            let expected = total as i64 - 1500 * 48;
            assert!((after as i64 - expected).abs() < 800, "{name}: après seek {after}, attendu ~{expected}");
        }
    }
}

#[cfg(test)]
mod decode_benchmark {
    use super::*;

    /// Coût processeur du décodage seul (à lancer en release) :
    /// SONORA_TESTDATA=<dossier> cargo test --release decoding_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn decoding_cost() {
        let Some(dir) = std::env::var_os("SONORA_TESTDATA").map(PathBuf::from) else { return };
        for (name, seconds) in [("video_aac_48k.mp4", 5.0f64), ("opus_stereo.ogg", 3.0), ("tone.wav", 2.0)] {
            let path = dir.join(name);
            let runs = 20;
            let start = std::time::Instant::now();
            for _ in 0..runs {
                let mut m = MediaDecoder::open(&path).unwrap();
                while m.next_chunk().unwrap().is_some() {}
            }
            let elapsed = start.elapsed().as_secs_f64();
            println!("{name} : {:.2} % d'un cœur (x{:.0} temps réel)", 100.0 * elapsed / (seconds * runs as f64), seconds * runs as f64 / elapsed);
        }
    }
}
