// -- protected_folders.rs -- //
//
// Dossiers système que l'explorateur ne doit ni lister ni ouvrir.
//
// Règle : un dossier placé directement à la racine d'un lecteur et portant un
// nom de la liste ci-dessous est protégé, ainsi que tout son contenu.
// "Program Files" et "Program Files (x86)" restent volontairement accessibles.
// Le dossier système réel (%SystemRoot%) est protégé même s'il porte un autre nom.

/// Noms (en minuscules) des dossiers système / techniques de la racine d'un lecteur.
const BLOCKED_ROOT_FOLDERS: &[&str] = &[
    "windows",
    "windows.old",
    "system.sav",
    "inetpub",
    "temp",
    "tmp",
    "perflogs",
    "$recycle.bin",
    "recycler",
    "system volume information",
    "recovery",
    "programdata",
    "config.msi",
    "documents and settings",
    "msocache",
    "$windows.~bt",
    "$windows.~ws",
    "$winreagent",
    "$sysreset",
    "$getcurrent",
    "boot",
    "efi",
    "swsetup",
    "drivers",
    "intel",
    "amd",
    "nvidia",
    "onedrivetemp",
    "found.000",
    "windowsapps",
];

/// Découpe un chemin Windows en (lecteur, composants), séparateurs `/` et `\` acceptés.
fn split_windows_path(path: &str) -> Vec<String> {
    let p = path.replace('/', "\\");
    let p = p.strip_prefix("\\\\?\\").unwrap_or(&p);
    p.split('\\').filter(|s| !s.is_empty()).map(|s| s.to_lowercase()).collect()
}

fn system_root_components() -> Option<Vec<String>> {
    let root = std::env::var("SystemRoot").or_else(|_| std::env::var("windir")).ok()?;
    let parts = split_windows_path(&root);
    if parts.len() >= 2 { Some(parts) } else { None }
}

pub fn is_protected_with_system_root(path: &str, system_root: Option<&[String]>) -> bool {
    let parts = split_windows_path(path);
    if parts.is_empty() {
        return false;
    }
    // Chemin UNC (\\serveur\partage) : jamais filtré.
    if path.starts_with("\\\\") && !path.starts_with("\\\\?\\") {
        return false;
    }
    // Seuls les chemins à lettre de lecteur (C:\...) ont une « racine de lecteur ».
    let has_drive = parts[0].len() == 2 && parts[0].ends_with(':');
    if has_drive {
        if let Some(first) = parts.get(1) {
            if BLOCKED_ROOT_FOLDERS.contains(&first.as_str()) {
                return true;
            }
        }
    }
    if let Some(root) = system_root {
        if parts.len() >= root.len() && parts[..root.len()] == *root {
            return true;
        }
    }
    false
}

pub fn is_protected(path: &str) -> bool {
    is_protected_with_system_root(path, system_root_components().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_system_folders_and_everything_below() {
        for p in [
            "C:\\Windows",
            "C:\\windows\\System32",
            "c:/WINDOWS/Fonts",
            "C:\\SYSTEM.SAV",
            "C:\\inetpub\\wwwroot",
            "C:\\Temp",
            "D:\\Temp\\x",
            "C:\\PerfLogs",
            "C:\\$Recycle.Bin\\S-1",
            "C:\\ProgramData\\x",
            "\\\\?\\C:\\Windows",
        ] {
            assert!(is_protected_with_system_root(p, None), "{p}");
        }
    }

    #[test]
    fn keeps_program_files_and_user_folders() {
        for p in [
            "C:\\",
            "C:\\Program Files",
            "C:\\Program Files (x86)\\App",
            "C:\\Users",
            "C:\\Users\\vleda\\Music",
            "C:\\corex",
            "C:\\MEGA",
            "C:\\lib",
            "D:\\Music\\Temp",
            "E:\\Windows Media",
            "\\\\serveur\\partage\\Windows",
        ] {
            assert!(!is_protected_with_system_root(p, None), "{p}");
        }
    }

    #[test]
    fn protects_real_system_root_even_with_other_name() {
        let root = split_windows_path("D:\\WinNT");
        assert!(is_protected_with_system_root("D:\\WinNT\\system32", Some(&root)));
        assert!(!is_protected_with_system_root("D:\\WinNTX", Some(&root)));
    }
}
