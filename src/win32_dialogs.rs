// -- win32_dialogs.rs -- //
//
// Boîtes de dialogue natives Windows : question « dossiers ou fichier ? » et
// sélection de fichiers (équivalents de messagebox.askquestion / filedialog.askopenfilenames).

use std::path::PathBuf;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST,
    OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONQUESTION, MB_YESNO};

use crate::win32_support::wide;

const FNERR_BUFFERTOOSMALL: u32 = 0x3003;

/// « Choisir un ou plusieurs dossiers = oui / un fichier = non » : `true` si l'utilisateur répond Oui.
pub fn ask_choose_folders(owner: HWND) -> bool {
    let text = wide("Choisir un ou plusieurs dossiers = oui / un fichier = non");
    let title = wide("Mode");
    unsafe { MessageBoxW(owner, text.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONQUESTION) == IDYES }
}

/// Analyse le tampon renvoyé par GetOpenFileNameW (mode sélection multiple, style Explorer).
pub fn parse_selection(buffer: &[u16]) -> Vec<PathBuf> {
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0usize;
    for (i, &c) in buffer.iter().enumerate() {
        if c == 0 {
            if i == start {
                break; // double zéro final
            }
            parts.push(String::from_utf16_lossy(&buffer[start..i]));
            start = i + 1;
        }
    }
    match parts.len() {
        0 => Vec::new(),
        1 => vec![PathBuf::from(&parts[0])], // un seul fichier : chemin complet
        _ => {
            let folder = PathBuf::from(&parts[0]);
            parts[1..].iter().map(|name| folder.join(name)).collect()
        }
    }
}

pub fn pick_files(owner: HWND) -> Vec<PathBuf> {
    let title = wide("Choisir un ou plusieurs fichiers");
    // Filtre : paires « libellé\0motifs\0 », terminées par un double zéro.
    let filter: Vec<u16> = "Media\0*.webm;*.mp3;*.wav;*.mp4;*.mkv;*.corex\0\0".encode_utf16().collect();

    // Tampon temporaire (libéré au retour) ; agrandi si la sélection est très longue.
    let mut capacity = 32 * 1024usize;
    for _ in 0..2 {
        let mut buffer = vec![0u16; capacity];
        let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = owner;
        ofn.lpstrFilter = filter.as_ptr();
        ofn.nFilterIndex = 1;
        ofn.lpstrFile = buffer.as_mut_ptr();
        ofn.nMaxFile = capacity as u32;
        ofn.lpstrTitle = title.as_ptr();
        ofn.Flags = OFN_ALLOWMULTISELECT
            | OFN_EXPLORER
            | OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | OFN_HIDEREADONLY;
        ofn.hInstance = null_mut();

        let ok = unsafe { GetOpenFileNameW(&mut ofn) };
        if ok != 0 {
            return parse_selection(&buffer);
        }
        let error = unsafe { CommDlgExtendedError() };
        if error == FNERR_BUFFERTOOSMALL {
            // Les deux premiers octets contiennent la taille nécessaire (en caractères).
            capacity = (buffer[0] as usize).max(capacity * 4);
            continue;
        }
        break; // annulation ou autre erreur
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(parts: &[&str]) -> Vec<u16> {
        let mut v = Vec::new();
        for p in parts {
            v.extend(p.encode_utf16());
            v.push(0);
        }
        v.push(0);
        v.extend([0u16; 8]);
        v
    }

    #[test]
    fn single_file_is_full_path() {
        assert_eq!(parse_selection(&w(&["C:\\Music\\a.mp3"])), vec![PathBuf::from("C:\\Music\\a.mp3")]);
    }

    #[test]
    fn several_files_are_joined_to_folder() {
        let r = parse_selection(&w(&["C:\\Music", "a.mp3", "b.mp4"]));
        assert_eq!(r, vec![PathBuf::from("C:\\Music").join("a.mp3"), PathBuf::from("C:\\Music").join("b.mp4")]);
    }

    #[test]
    fn empty_buffer_is_empty_selection() {
        assert!(parse_selection(&[0u16; 4]).is_empty());
    }
}
