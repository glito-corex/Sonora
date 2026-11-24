// -- media_keys.rs -- //
//
// Touches multimédia du clavier et commandes d'application Windows.
//
// * Touches (WM_HOTKEY) : enregistrées globalement, elles fonctionnent même quand
//   Sonora n'a pas le focus (comme tout lecteur de musique).
// * Commandes d'application (WM_APPCOMMAND) : envoyées par certains claviers,
//   télécommandes et casques ; elles couvrent aussi « Lecture » et « Pause » séparés.
//
// La correspondance touche -> commande est pure (testable sur toute plateforme).

use crate::playback_commands::PlaybackCommand;

// Codes de touches virtuelles Windows.
pub const VK_VOLUME_MUTE: u32 = 0xAD;
pub const VK_VOLUME_DOWN: u32 = 0xAE;
pub const VK_VOLUME_UP: u32 = 0xAF;
pub const VK_MEDIA_NEXT_TRACK: u32 = 0xB0;
pub const VK_MEDIA_PREV_TRACK: u32 = 0xB1;
pub const VK_MEDIA_STOP: u32 = 0xB2;
pub const VK_MEDIA_PLAY_PAUSE: u32 = 0xB3;
/// Touche « Play » dédiée (VK_PLAY).
pub const VK_PLAY: u32 = 0xFA;

// Commandes d'application (APPCOMMAND_*).
pub const APPCOMMAND_VOLUME_MUTE: u32 = 8;
pub const APPCOMMAND_VOLUME_DOWN: u32 = 9;
pub const APPCOMMAND_VOLUME_UP: u32 = 10;
pub const APPCOMMAND_MEDIA_NEXTTRACK: u32 = 11;
pub const APPCOMMAND_MEDIA_PREVIOUSTRACK: u32 = 12;
pub const APPCOMMAND_MEDIA_STOP: u32 = 13;
pub const APPCOMMAND_MEDIA_PLAY_PAUSE: u32 = 14;
pub const APPCOMMAND_MEDIA_PLAY: u32 = 46;
pub const APPCOMMAND_MEDIA_PAUSE: u32 = 47;

/// Touches enregistrées : (identifiant, touche virtuelle, répétition automatique autorisée, commande).
/// La répétition n'est utile qu'aux touches de volume (maintenir pour monter / descendre).
pub const HOTKEYS: [(i32, u32, bool, PlaybackCommand); 8] = [
    (1, VK_MEDIA_PLAY_PAUSE, false, PlaybackCommand::TogglePlayPause),
    (2, VK_PLAY, false, PlaybackCommand::Play),
    (3, VK_MEDIA_NEXT_TRACK, false, PlaybackCommand::NextTrack),
    (4, VK_MEDIA_PREV_TRACK, false, PlaybackCommand::PreviousTrack),
    (5, VK_MEDIA_STOP, false, PlaybackCommand::Stop),
    (6, VK_VOLUME_UP, true, PlaybackCommand::VolumeUp),
    (7, VK_VOLUME_DOWN, true, PlaybackCommand::VolumeDown),
    (8, VK_VOLUME_MUTE, false, PlaybackCommand::ToggleMute),
];

pub fn command_for_hotkey(id: i32) -> Option<PlaybackCommand> {
    HOTKEYS.iter().find(|h| h.0 == id).map(|h| h.3)
}

pub fn command_for_app_command(command: u32) -> Option<PlaybackCommand> {
    Some(match command {
        APPCOMMAND_MEDIA_PLAY_PAUSE => PlaybackCommand::TogglePlayPause,
        APPCOMMAND_MEDIA_PLAY => PlaybackCommand::Play,
        APPCOMMAND_MEDIA_PAUSE => PlaybackCommand::Pause,
        APPCOMMAND_MEDIA_STOP => PlaybackCommand::Stop,
        APPCOMMAND_MEDIA_NEXTTRACK => PlaybackCommand::NextTrack,
        APPCOMMAND_MEDIA_PREVIOUSTRACK => PlaybackCommand::PreviousTrack,
        APPCOMMAND_VOLUME_UP => PlaybackCommand::VolumeUp,
        APPCOMMAND_VOLUME_DOWN => PlaybackCommand::VolumeDown,
        APPCOMMAND_VOLUME_MUTE => PlaybackCommand::ToggleMute,
        _ => return None,
    })
}

/// Numéro de commande d'application contenu dans le `lparam` de WM_APPCOMMAND
/// (équivalent de GET_APPCOMMAND_LPARAM : mot de poids fort sans les bits de périphérique).
pub fn app_command_from_lparam(lparam: isize) -> u32 {
    (((lparam >> 16) & 0xFFFF) as u32) & !0xF000
}

/// Enregistre les touches globales ; renvoie le nombre de touches obtenues.
/// Une touche déjà prise par un autre programme est simplement ignorée.
#[cfg(windows)]
pub fn register(hwnd: windows_sys::Win32::Foundation::HWND) -> usize {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_NOREPEAT};
    HOTKEYS
        .iter()
        .filter(|(id, vk, repeat, _)| {
            let modifiers = if *repeat { 0 } else { MOD_NOREPEAT };
            unsafe { RegisterHotKey(hwnd, *id, modifiers, *vk) != 0 }
        })
        .count()
}

#[cfg(windows)]
pub fn unregister(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;
    for (id, _, _, _) in HOTKEYS.iter() {
        unsafe { UnregisterHotKey(hwnd, *id) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hotkey_maps_to_its_command() {
        assert_eq!(command_for_hotkey(1), Some(PlaybackCommand::TogglePlayPause));
        assert_eq!(command_for_hotkey(3), Some(PlaybackCommand::NextTrack));
        assert_eq!(command_for_hotkey(8), Some(PlaybackCommand::ToggleMute));
        assert_eq!(command_for_hotkey(99), None);
        // identifiants et touches uniques
        let mut ids: Vec<_> = HOTKEYS.iter().map(|h| h.0).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), HOTKEYS.len());
        let mut keys: Vec<_> = HOTKEYS.iter().map(|h| h.1).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), HOTKEYS.len());
    }

    #[test]
    fn app_commands_cover_play_and_pause_separately() {
        assert_eq!(command_for_app_command(APPCOMMAND_MEDIA_PLAY), Some(PlaybackCommand::Play));
        assert_eq!(command_for_app_command(APPCOMMAND_MEDIA_PAUSE), Some(PlaybackCommand::Pause));
        assert_eq!(command_for_app_command(APPCOMMAND_MEDIA_STOP), Some(PlaybackCommand::Stop));
        assert_eq!(command_for_app_command(APPCOMMAND_VOLUME_UP), Some(PlaybackCommand::VolumeUp));
        assert_eq!(command_for_app_command(1), None); // APPCOMMAND_BROWSER_FORWARD : non géré
    }

    #[test]
    fn app_command_is_extracted_from_lparam() {
        // WM_APPCOMMAND : MAKELPARAM(keys, MAKELONG(command | device << 12))
        let lparam = ((APPCOMMAND_MEDIA_PLAY_PAUSE | (2 << 12)) as isize) << 16;
        assert_eq!(app_command_from_lparam(lparam), APPCOMMAND_MEDIA_PLAY_PAUSE);
    }
}
