// -- auto_advance.rs -- //
//
// Passage automatique au morceau suivant, piloté par événements : le thread
// dort tant que le moteur ne signale ni fin de morceau ni erreur (aucune
// interrogation périodique, donc aucun coût CPU pendant la lecture).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::audio_engine::PlayState;
use crate::playlist_navigation;
use crate::player_state::PlayerState;

/// Délai minimal entre deux passages automatiques (anti-rebond).
const MINIMUM_DELAY_BETWEEN_ADVANCES: Duration = Duration::from_secs(1);

pub struct AutoAdvance {
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AutoAdvance {
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

impl Drop for AutoAdvance {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start(core: Arc<PlayerState>) -> AutoAdvance {
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();

    let handle = thread::Builder::new()
        .name("sonora-auto-advance".into())
        .spawn(move || {
            core.engine.set_event_waiter(thread::current());
            let mut seen_ended = core.engine.ended_count();
            let mut seen_errors = core.engine.error_count();
            let mut last_advance = Instant::now() - Duration::from_secs(10);

            while flag.load(Ordering::Acquire) {
                let mut retry_in: Option<Duration> = None;

                let errors = core.engine.error_count();
                let ended = core.engine.ended_count();

                if errors != seen_errors {
                    // Fichier illisible : on l'indique, puis on passe au suivant sans boucler à l'infini.
                    seen_errors = errors;
                    let message = core.engine.last_error();
                    let give_up = {
                        let mut state = core.lock();
                        state.failures += 1;
                        let name = if state.index >= 0 && (state.index as usize) < state.playlist.len() {
                            playlist_navigation::display_name(&state.playlist[state.index as usize])
                        } else {
                            String::new()
                        };
                        state.label = format!("{name} — erreur de lecture : {message}");
                        state.failures >= state.playlist.len().max(1)
                    };
                    core.notify();
                    if !give_up {
                        last_advance = Instant::now();
                        playlist_navigation::next_song(&core);
                    }
                } else if ended != seen_ended {
                    if core.engine.state() != PlayState::Ended {
                        // Fin périmée (déplacement ou nouveau morceau entre-temps) : ignorée.
                        seen_ended = ended;
                    } else if last_advance.elapsed() >= MINIMUM_DELAY_BETWEEN_ADVANCES {
                        seen_ended = ended;
                        last_advance = Instant::now();
                        core.lock().failures = 0; // un morceau a été lu jusqu'au bout
                        playlist_navigation::next_song(&core);
                    } else {
                        retry_in = Some(MINIMUM_DELAY_BETWEEN_ADVANCES.saturating_sub(last_advance.elapsed()));
                    }
                }

                if !flag.load(Ordering::Acquire) {
                    break;
                }
                match retry_in {
                    Some(d) => thread::park_timeout(d.max(Duration::from_millis(10))),
                    None => thread::park(),
                }
            }
        })
        .expect("thread de passage automatique");

    AutoAdvance { running, thread: Some(handle) }
}
