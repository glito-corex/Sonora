// -- playlist_navigation.rs -- //  (lecture d'un index, suivant, précédent, rejouer)

use std::path::Path;

use crate::player_state::PlayerState;

fn safe_index(index: isize, len: usize) -> Option<usize> {
    if len == 0 {
        None
    } else {
        Some(index.rem_euclid(len as isize) as usize)
    }
}

pub fn display_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub fn play(core: &PlayerState, index: isize) {
    let path = {
        let mut g = core.lock();
        let Some(idx) = safe_index(index, g.playlist.len()) else { return };
        let path = g.playlist[idx].clone();
        g.index = idx as isize;
        g.label = display_name(&path);
        path
    };
    core.engine.open(path);
    core.notify();
}

pub fn next_song(core: &PlayerState) {
    let idx = {
        let g = core.lock();
        if g.playlist.is_empty() {
            return;
        }
        g.index + 1
    };
    play(core, idx);
}

pub fn prev_song(core: &PlayerState) {
    let idx = {
        let g = core.lock();
        if g.playlist.is_empty() {
            return;
        }
        g.index - 1
    };
    play(core, idx);
}

pub fn replay(core: &PlayerState) {
    let idx = core.lock().index;
    if idx >= 0 {
        play(core, idx);
    }
}

/// Lecture demandée par l'utilisateur (boutons, nouvelle playlist) : repart d'un compteur d'échecs à zéro.
pub fn play_requested_by_user(core: &PlayerState, index: isize) {
    core.lock().failures = 0;
    play(core, index);
}

pub fn next_requested_by_user(core: &PlayerState) {
    core.lock().failures = 0;
    next_song(core);
}

pub fn previous_requested_by_user(core: &PlayerState) {
    core.lock().failures = 0;
    prev_song(core);
}
