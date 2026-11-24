// -- memory_trimming.rs -- //
//
// Rend à Windows la mémoire de travail (working set) devenue inutile : quand la
// fenêtre est réduite et quand le flux audio inactif est fermé. Windows la
// reprendra uniquement si elle sert de nouveau ; la mémoire affichée tombe au
// strict nécessaire (comme le fait déjà le système pour une fenêtre minimisée).

#[cfg(windows)]
pub fn trim_working_set() {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
    unsafe {
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

#[cfg(not(windows))]
pub fn trim_working_set() {}
