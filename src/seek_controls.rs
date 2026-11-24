// -- seek_controls.rs -- //  (+10 s / -10 s)

use crate::player_state::PlayerState;

pub fn add_10s(core: &PlayerState) {
    let length = core.engine.length_ms();
    if length <= 0 {
        return;
    }
    let new_time = (core.engine.position_ms() + 10_000).min(length);
    core.engine.seek_ms(new_time);
}

pub fn sub_10s(core: &PlayerState) {
    let new_time = (core.engine.position_ms() - 10_000).max(0);
    core.engine.seek_ms(new_time);
}
