// -- folder_browser.rs -- //
//
// Logique de l'explorateur de dossiers (sélection multiple par glisser).
// Le rendu est dans browser_window.rs.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::protected_folders::is_protected;

pub struct Row {
    pub text: String,
    pub path: PathBuf,
    /// Ligne "⬅ .." : sert à naviguer, jamais sélectionnable.
    pub is_parent: bool,
}

pub struct FolderExplorer {
    pub current_path: String,
    pub rows: Vec<Row>,
    pub selected: BTreeSet<usize>,
    pub dragging: bool,
    pub last_item: Option<usize>,
    /// `Some` quand la fenêtre est fermée (liste vide = annulation).
    pub result: Option<Vec<PathBuf>>,
}

/// Équivalent de os.path.normpath (purement lexical).
pub fn normpath(p: &str) -> String {
    if p.is_empty() {
        return String::new();
    }
    let mut out = PathBuf::new();
    for c in Path::new(p).components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() && out.as_os_str().is_empty() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        ".".to_string()
    } else {
        out.to_string_lossy().into_owned()
    }
}

pub fn normalize_path(path: &str) -> String {
    let p = path.trim();
    if p.is_empty() {
        return String::new();
    }
    let p = p.trim_matches('"').trim_matches('\'');
    #[cfg(windows)]
    let p = p.replace('/', "\\");
    #[cfg(not(windows))]
    let p = p.to_string();
    #[allow(unused_mut)]
    let mut n = normpath(p.trim());
    // "C:" seul désigne le dossier courant du lecteur : on veut la racine.
    #[cfg(windows)]
    if n.len() == 2 && n.ends_with(':') {
        n.push('\\');
    }
    n
}

#[cfg(windows)]
pub fn is_hidden_or_system(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    // 2 = HIDDEN, 4 = SYSTEM, 1024 = REPARSE_POINT (jonctions / liens)
    match fs::symlink_metadata(path) {
        Ok(m) => m.file_attributes() & (2 | 4 | 1024) != 0,
        Err(_) => false,
    }
}

#[cfg(not(windows))]
pub fn is_hidden_or_system(path: &Path) -> bool {
    path.file_name().map_or(false, |n| n.to_string_lossy().starts_with('.'))
}

pub fn get_start_path() -> String {
    for var in ["USERPROFILE", "HOME"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() && Path::new(&v).is_dir() {
                return v;
            }
        }
    }
    std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

impl FolderExplorer {
    pub fn new() -> Self {
        let start = get_start_path();
        let mut e = FolderExplorer {
            current_path: start.clone(),
            rows: Vec::new(),
            selected: BTreeSet::new(),
            dragging: false,
            last_item: None,
            result: None,
        };
        e.load_directory(&start);
        e
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    fn select(&mut self, item: usize) {
        if let Some(r) = self.rows.get(item) {
            if !r.is_parent {
                self.selected.insert(item);
            }
        }
    }

    /// Bouton enfoncé : sur une ligne -> nouvelle sélection ; démarre le glissement.
    pub fn press(&mut self, item: Option<usize>) {
        if item.is_some() {
            self.clear_selection();
        }
        self.dragging = true;
        if let Some(i) = item {
            self.last_item = Some(i);
            self.select(i);
        }
    }

    /// Glissement : ajoute toutes les lignes entre la précédente et l'actuelle
    /// (aucune ligne n'est sautée, même si la souris va vite).
    pub fn drag(&mut self, item: Option<usize>) {
        if !self.dragging {
            return;
        }
        if let Some(i) = item {
            if Some(i) != self.last_item {
                match self.last_item {
                    Some(l) => {
                        let (a, b) = if l <= i { (l, i) } else { (i, l) };
                        for k in a..=b {
                            self.select(k);
                        }
                    }
                    None => self.select(i),
                }
                self.last_item = Some(i);
            }
        }
    }

    pub fn release(&mut self) {
        self.dragging = false;
        self.last_item = None;
    }

    pub fn load_directory(&mut self, path: &str) {
        let path = normalize_path(path);
        let p = Path::new(&path);
        if path.is_empty() || !p.is_dir() || is_protected(&path) {
            return;
        }

        self.rows.clear();
        // La sélection appartient à la liste affichée : naviguer la réinitialise.
        self.selected.clear();
        self.current_path = path.clone();

        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() && parent != p {
                self.rows.push(Row {
                    text: "⬅ ..".to_string(),
                    path: parent.to_path_buf(),
                    is_parent: true,
                });
            }
        }

        if let Ok(rd) = fs::read_dir(p) {
            let mut dirs: Vec<(String, PathBuf)> = rd
                .flatten()
                .filter_map(|e| {
                    let full = e.path();
                    if full.is_dir() && !is_hidden_or_system(&full) && !is_protected(&full.to_string_lossy()) {
                        Some((e.file_name().to_string_lossy().into_owned(), full))
                    } else {
                        None
                    }
                })
                .collect();
            dirs.sort_by_key(|(n, _)| n.to_lowercase());
            for (name, full) in dirs {
                self.rows.push(Row { text: format!("📁 {name}"), path: full, is_parent: false });
            }
        }
    }

    pub fn open_row(&mut self, idx: usize) {
        if let Some(r) = self.rows.get(idx) {
            let p = r.path.to_string_lossy().into_owned();
            self.load_directory(&p);
        }
    }

    pub fn open_path(&mut self) {
        let path = normalize_path(&self.current_path);
        if Path::new(&path).is_dir() {
            self.load_directory(&path);
        }
    }

    pub fn refresh(&mut self) {
        let p = self.current_path.clone();
        self.load_directory(&p);
    }

    pub fn go_back(&mut self) {
        let cur = normalize_path(&self.current_path);
        if let Some(parent) = Path::new(&cur).parent() {
            if !parent.as_os_str().is_empty() {
                let p = parent.to_string_lossy().into_owned();
                self.load_directory(&p);
            }
        }
    }

    pub fn validate_selection(&mut self) {
        let list: Vec<PathBuf> = self
            .selected
            .iter()
            .filter_map(|&i| self.rows.get(i))
            .filter(|r| !r.is_parent)
            .map(|r| r.path.clone())
            .collect();
        self.result = Some(list);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normpath_works() {
        assert_eq!(normpath("/a/b/../c/./d"), "/a/c/d");
        assert_eq!(normpath("/.."), "/");
        assert_eq!(normpath("a/../.."), "..");
    }

    #[test]
    fn normalize_strips_quotes_and_spaces() {
        assert_eq!(normalize_path("  \"/tmp/x/../y\"  "), "/tmp/y");
        assert_eq!(normalize_path(""), "");
    }

    #[test]
    fn selection_and_navigation() {
        let base = std::env::temp_dir().join(format!("sonora_t_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        for d in ["Alpha", "beta", "Gamma", "delta", ".hidden"] {
            fs::create_dir_all(base.join(d)).unwrap();
        }
        fs::write(base.join("file.txt"), b"x").unwrap();

        let mut e = FolderExplorer::new();
        e.load_directory(base.to_str().unwrap());
        // ".." + 4 dossiers visibles (triés sans casse), fichier et caché exclus
        assert!(e.rows[0].is_parent);
        let names: Vec<_> = e.rows.iter().skip(1).map(|r| r.text.clone()).collect();
        assert_eq!(names, vec!["📁 Alpha", "📁 beta", "📁 delta", "📁 Gamma"]);

        // clic puis glissement rapide de la ligne 1 à la ligne 4 : aucune ligne sautée
        e.press(Some(1));
        e.drag(Some(4));
        e.release();
        assert_eq!(e.selected.iter().cloned().collect::<Vec<_>>(), vec![1, 2, 3, 4]);

        // ".." jamais sélectionnable
        e.press(Some(0));
        e.drag(Some(2));
        e.release();
        assert!(!e.selected.contains(&0));

        // naviguer vide la sélection (bug d'origine : sélection périmée)
        e.open_row(1);
        assert!(e.selected.is_empty());

        e.go_back();
        e.press(Some(2));
        e.release();
        e.validate_selection();
        assert_eq!(e.result.as_ref().unwrap(), &vec![base.join("beta")]);

        let _ = fs::remove_dir_all(&base);
    }
}
