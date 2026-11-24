// -- media_selection.rs -- //
//
// Choix des médias : extensions acceptées, listage / tri des dossiers, et
// boîtes de dialogue natives Windows (mode dossier/fichier, sélection de fichiers).

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

pub const MEDIA_EXTENSIONS: [&str; 6] = [".webm", ".mp3", ".wav", ".mp4", ".mkv", ".corex"];

pub fn has_media_extension(name: &str) -> bool {
    let lower = name.to_lowercase();
    MEDIA_EXTENSIONS.iter().any(|e| lower.ends_with(e))
}

/// Tri « naturel » insensible à la casse : « piste2 » avant « piste10 ».
pub fn natural_compare(a: &str, b: &str) -> Ordering {
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let start_a = i;
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            let start_b = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let number_a: String = a[start_a..i].iter().collect::<String>().trim_start_matches('0').to_string();
            let number_b: String = b[start_b..j].iter().collect::<String>().trim_start_matches('0').to_string();
            let order = number_a.len().cmp(&number_b.len()).then_with(|| number_a.cmp(&number_b));
            if order != Ordering::Equal {
                return order;
            }
        } else {
            let order = a[i].cmp(&b[j]);
            if order != Ordering::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

/// Fichiers média d'un dossier (les sous-dossiers nommés « x.mp3 » sont exclus), triés.
pub fn list_media_files(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = match fs::read_dir(folder) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.file_name().to_str().map_or(false, has_media_extension) && e.path().is_file())
            .map(|e| e.path())
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort_by(|a, b| {
        natural_compare(
            &a.file_name().unwrap_or_default().to_string_lossy(),
            &b.file_name().unwrap_or_default().to_string_lossy(),
        )
    });
    files
}

/// Playlist issue de plusieurs dossiers : dossiers dans un ordre stable, sans doublon.
pub fn playlist_from_folders(folders: &[PathBuf]) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = folders.to_vec();
    folders.sort_by(|a, b| natural_compare(&a.to_string_lossy(), &b.to_string_lossy()));
    folders.dedup();
    let mut playlist: Vec<PathBuf> = Vec::new();
    for folder in &folders {
        for file in list_media_files(folder) {
            if !playlist.contains(&file) {
                playlist.push(file);
            }
        }
    }
    playlist
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["piste10.mp3", "Piste2.mp3", "piste1.mp3", "a.mp3"];
        v.sort_by(|a, b| natural_compare(a, b));
        assert_eq!(v, vec!["a.mp3", "piste1.mp3", "Piste2.mp3", "piste10.mp3"]);
    }

    #[test]
    fn extension_filter() {
        assert!(has_media_extension("X.MP4"));
        assert!(has_media_extension("a.corex"));
        assert!(!has_media_extension("a.txt"));
    }

    #[test]
    fn folder_listing_excludes_directories_named_like_media() {
        let base = std::env::temp_dir().join(format!("sonora_l_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("fake.mp3")).unwrap();
        fs::write(base.join("b2.mp3"), b"").unwrap();
        fs::write(base.join("b10.wav"), b"").unwrap();
        fs::write(base.join("notes.txt"), b"").unwrap();
        let files = list_media_files(&base);
        assert_eq!(files, vec![base.join("b2.mp3"), base.join("b10.wav")]);
        let _ = fs::remove_dir_all(&base);
    }
}
