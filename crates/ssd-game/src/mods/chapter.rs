//! The chapter wrappers of the packs (M10, Task 8): the ending that completes the game. The ending runs one of two mode inits,
//! `00:96A5` (mode `$11`) and `00:9730` (mode `$12`), once per ending (`docs/re/data/title-chapters.md`, section 7). Each wrapper
//! sets a four-byte marker of the mod state block (`$7F:FB38`, free in [`super::state`]) and declines, so the original init runs as it
//! would. The menu driver reads the marker in every frame and reports it as the application event [`COMPLETED_TAG`] (see
//! [`completion_events`]); the app writes the `completed` file of the saves folder, and `ssd_mod::chapter::unlocked` reads the
//! file. The wrappers are installed with the other redirectors, so they run in Enhanced runs only.

use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, HostEvent, ReturnKind};

/// The application event of a completed game. Its payload is empty: the event is the fact. The next free tag after
/// `state::MODS_TAG` (4).
pub const COMPLETED_TAG: u8 = 5;
/// The entries of the two endings' mode inits: the wrappers' keys.
pub const ENDING_11: u32 = 0x00_96A5;
pub const ENDING_12: u32 = 0x00_9730;
/// The marker: the four bytes `SSDc` at [`MARKER_AT`], written by an ending's init and cleared when the driver reports it. Four bytes,
/// because the power-on fill of work RAM is random (see `state`): one byte of it can be any value.
pub const MARKER_AT: u32 = 0x7F_FB38;
/// The four bytes of the marker.
pub const MARKER: [u8; 4] = *b"SSDc";

/// The index of the marker in work RAM (the first byte of work RAM, `$7E:0000`, is index 0).
fn marker_index() -> usize {
    (MARKER_AT - 0x7E_0000) as usize
}

/// The hook entries of the two wrappers: Patch mode, costing nothing of their own (as the other redirectors).
#[must_use]
pub fn hook_entries() -> Vec<HookEntry> {
    let entry = |name: &str, key: u32, func: ssd_oracle::RoutineFn| HookEntry {
        name: name.to_owned(),
        key,
        mode: HookMode::Patch,
        action: HookAction::Routine { func, ret: ReturnKind::Rtl },
        entry_m8: None,
        entry_x8: None,
        flags_mask: 0,
        verify_sample: 1,
        cycles: Some(0),
        bound: None,
        budget_frames: None,
    };
    vec![
        entry("mod_completed_11", ENDING_11, ending_11),
        entry("mod_completed_12", ENDING_12, ending_12),
    ]
}

/// Mode `$11`'s init: marks the game completed, then the original init runs.
fn ending_11(ctx: &mut Ctx<'_>) {
    for (offset, byte) in MARKER.into_iter().enumerate() {
        ctx.write8(MARKER_AT + offset as u32, byte);
    }
    ctx.resume_at(ENDING_11);
}

/// Mode `$12`'s init: marks the game completed, then the original init runs.
fn ending_12(ctx: &mut Ctx<'_>) {
    for (offset, byte) in MARKER.into_iter().enumerate() {
        ctx.write8(MARKER_AT + offset as u32, byte);
    }
    ctx.resume_at(ENDING_12);
}

/// The host events of a frame for the completion: when the marker is set, its clear (a write of the block) and the event
/// [`COMPLETED_TAG`]. The clear keeps the event to one frame, and a replay carries both.
#[must_use]
pub fn completion_events(wram: &[u8]) -> Vec<HostEvent> {
    if wram.get(marker_index()..marker_index() + MARKER.len()) != Some(&MARKER[..]) {
        return Vec::new();
    }
    vec![
        HostEvent::Wram { address: MARKER_AT, bytes: vec![0; MARKER.len()] },
        HostEvent::App { tag: COMPLETED_TAG, payload: Vec::new() },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_endings_are_patch_entries_that_cost_nothing_and_carry_no_cheat_key() {
        let entries = hook_entries();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["mod_completed_11", "mod_completed_12"]);
        for entry in &entries {
            assert_eq!(entry.mode, HookMode::Patch, "{}", entry.name);
            assert_eq!(entry.cycles, Some(0), "{}", entry.name);
            assert!(matches!(entry.action, HookAction::Routine { .. }), "{}", entry.name);
            assert!(
                crate::cheats::hook_entries().iter().all(|cheat| cheat.key != entry.key),
                "{}: a cheat key",
                entry.name
            );
        }
        assert_eq!(entries[0].key, ENDING_11);
        assert_eq!(entries[1].key, ENDING_12);
    }

    #[test]
    fn a_marked_frame_reports_the_completion_once_and_clears_the_marker() {
        let mut wram = vec![0_u8; 0x2_0000];
        assert!(completion_events(&wram).is_empty(), "no marker, no event");
        wram[marker_index()..marker_index() + MARKER.len()].copy_from_slice(&MARKER);
        let events = completion_events(&wram);
        assert_eq!(
            events,
            [
                HostEvent::Wram { address: MARKER_AT, bytes: vec![0; MARKER.len()] },
                HostEvent::App { tag: COMPLETED_TAG, payload: Vec::new() },
            ]
        );
        assert_eq!(COMPLETED_TAG, 5, "the next free tag after the mod state's (4)");
    }
}
