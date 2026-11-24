// -- main.rs -- //
//
// Sonora : lecteur audio léger en Rust pur.
//   décodage : Symphonia (mp3, wav, mp4/AAC, mkv/webm, ogg, vorbis) + libopus (décodeur seul, intégrée)
//   sortie   : cpal (WASAPI)       interface : Win32 / GDI (aucune bibliothèque graphique)

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![cfg_attr(not(windows), allow(dead_code))]

mod audio_engine;
mod auto_advance;
mod codec_registry;
mod folder_browser;
mod format_conversion;
mod libopus_bindings;
mod media_decoder;
mod media_keys;
mod media_selection;
mod memory_trimming;
mod opus_decoder;
mod playback_commands;
mod player_state;
mod playlist_navigation;
mod protected_folders;
mod seek_controls;
mod sinc_resampler;
mod volume_control;
mod window_layout;

#[cfg(windows)]
mod browser_window;
#[cfg(windows)]
mod main_window;
#[cfg(windows)]
mod win32_dialogs;
#[cfg(windows)]
mod win32_support;

use std::path::PathBuf;

fn executable_directory() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Même comportement que l'original : error_log.txt réécrit à chaque erreur.
fn write_error_log(text: &str) {
    let _ = std::fs::write(executable_directory().join("error_log.txt"), text);
}

#[cfg(windows)]
fn main() {
    use std::sync::Arc;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, PostMessageW, TranslateMessage, MSG};

    // Équivalent de os.chdir(dossier du script).
    let _ = std::env::set_current_dir(executable_directory());

    // Équivalent du try/except + traceback -> error_log.txt
    // (message et position suffisent : l'exécutable est strippé, une pile d'appels n'apporterait rien).
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        write_error_log(&format!("Erreur dans le thread « {} » :\n{info}\n", thread.name().unwrap_or("?")));
    }));

    unsafe {
        // Sans effet si le manifeste (build.rs) a déjà déclaré la prise en charge du DPI.
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(-4isize as _);
    }

    let core = Arc::new(player_state::PlayerState::new());
    volume_control::set_volume(&core, 80.0); // volume initial
    let mut auto_advance = auto_advance::start(core.clone());

    let hwnd = main_window::create(core.clone());
    if hwnd.is_null() {
        write_error_log("Impossible de créer la fenêtre principale.");
        return;
    }

    // Réveil de l'interface depuis les threads de lecture (aucun minuteur).
    let hwnd_value = hwnd as usize;
    core.set_notifier(Box::new(move || unsafe {
        PostMessageW(hwnd_value as HWND, main_window::WM_STATE_CHANGED, 0, 0);
    }));

    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    auto_advance.stop();
    core.engine.stop();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Sonora utilise l'interface Win32 : exécution sous Windows uniquement.");
}
