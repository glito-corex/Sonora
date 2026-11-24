// -- playback_commands.rs -- //
//
// Commandes de lecture partagées par les boutons, les touches multimédia du
// clavier et les commandes d'application (casques, télécommandes) : un seul
// endroit définit ce que font lecture, pause, arrêt, piste suivante, volume...

use crate::audio_engine::PlayState;
use crate::player_state::PlayerState;
use crate::{playlist_navigation, volume_control};

/// Pas des touches « Volume + / - » (en points de volume, sur 100).
pub const VOLUME_STEP: i32 = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlaybackCommand {
    /// Reprend la lecture (ou relance le morceau terminé / arrêté).
    Play,
    /// Met en pause si le morceau est en cours de lecture.
    Pause,
    /// Bascule lecture / pause.
    TogglePlayPause,
    /// Arrête la lecture ; « Lecture » repart ensuite du début du morceau.
    Stop,
    NextTrack,
    PreviousTrack,
    VolumeUp,
    VolumeDown,
    ToggleMute,
}

fn play(core: &PlayerState) {
    if core.lock().playlist.is_empty() || core.engine.is_playing() {
        return;
    }
    match core.engine.state() {
        PlayState::Ended | PlayState::Error | PlayState::Idle => playlist_navigation::replay(core),
        _ => core.engine.set_paused(false),
    }
}

fn pause(core: &PlayerState) {
    if core.engine.is_playing() {
        core.engine.set_paused(true);
    }
}

pub fn execute(core: &PlayerState, command: PlaybackCommand) {
    match command {
        PlaybackCommand::Play => play(core),
        PlaybackCommand::Pause => pause(core),
        PlaybackCommand::TogglePlayPause => {
            if core.engine.is_playing() {
                pause(core);
            } else {
                play(core);
            }
        }
        PlaybackCommand::Stop => {
            if !core.lock().playlist.is_empty() {
                core.engine.stop();
            }
        }
        PlaybackCommand::NextTrack => playlist_navigation::next_requested_by_user(core),
        PlaybackCommand::PreviousTrack => playlist_navigation::previous_requested_by_user(core),
        PlaybackCommand::VolumeUp => {
            let volume = core.lock().user_volume;
            volume_control::set_volume(core, (volume + VOLUME_STEP).min(100) as f64);
        }
        PlaybackCommand::VolumeDown => {
            let volume = core.lock().user_volume;
            volume_control::set_volume(core, (volume - VOLUME_STEP).max(0) as f64);
        }
        PlaybackCommand::ToggleMute => volume_control::toggle_mute(core),
    }
    core.notify();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_keys_step_and_clamp() {
        let core = PlayerState::new();
        volume_control::set_volume(&core, 80.0);
        execute(&core, PlaybackCommand::VolumeUp);
        assert_eq!(core.lock().user_volume, 85);
        for _ in 0..10 {
            execute(&core, PlaybackCommand::VolumeUp);
        }
        assert_eq!(core.lock().user_volume, 100);
        for _ in 0..30 {
            execute(&core, PlaybackCommand::VolumeDown);
        }
        assert_eq!(core.lock().user_volume, 0);
    }

    #[test]
    fn mute_key_toggles_and_volume_key_unmutes() {
        let core = PlayerState::new();
        execute(&core, PlaybackCommand::ToggleMute);
        assert!(core.lock().muted);
        execute(&core, PlaybackCommand::VolumeUp);
        assert!(!core.lock().muted);
        execute(&core, PlaybackCommand::ToggleMute);
        execute(&core, PlaybackCommand::ToggleMute);
        assert!(!core.lock().muted);
    }

    #[test]
    fn transport_commands_do_nothing_without_playlist() {
        let core = PlayerState::new();
        for c in [
            PlaybackCommand::Play,
            PlaybackCommand::Pause,
            PlaybackCommand::TogglePlayPause,
            PlaybackCommand::Stop,
            PlaybackCommand::NextTrack,
            PlaybackCommand::PreviousTrack,
        ] {
            execute(&core, c);
        }
        assert!(!core.engine.is_playing());
        assert_eq!(core.lock().index, -1);
    }
}
