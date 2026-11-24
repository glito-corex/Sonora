// -- volume_control.rs -- //  (volume / mute)

use crate::player_state::PlayerState;

/// Volume effectif envoyé au moteur (0 si muet). 100 = gain 1.0 (comme VLC).
pub fn apply_volume(core: &PlayerState, volume: i32) {
    core.engine.set_gain(volume.clamp(0, 100) as f32 / 100.0);
}

pub fn set_volume(core: &PlayerState, val: f64) {
    if !val.is_finite() {
        return;
    }
    let volume = (val as i32).clamp(0, 100);
    {
        let mut g = core.lock();
        g.user_volume = volume;
        // Déplacer le curseur rend le son : le bouton doit refléter l'état réel.
        g.muted = false;
    }
    apply_volume(core, volume);
}

pub fn toggle_mute(core: &PlayerState) {
    let (muted, vol) = {
        let mut g = core.lock();
        g.muted = !g.muted;
        (g.muted, g.user_volume)
    };
    if muted {
        apply_volume(core, 0);
    } else {
        apply_volume(core, vol);
    }
}

/// Texte du bouton (même style que le libellé initial "MUTE").
pub fn mute_label(core: &PlayerState) -> &'static str {
    if core.lock().muted { "UNMUTE" } else { "MUTE" }
}
