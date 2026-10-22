//! The wide BP in the snapshot and the side-record copies (M10, Task 9, part C.2a): see [`super::state`] for the layout.
//!
//! Each copy routine moves a record's low 24 bits, and the hooks here move the high 16 bits (the block's bytes) along with
//! them. Every hook declines when the block is not live, so a run without the marker is the original's, and every hook
//! resumes at its own key, so the original code goes on after the copy:
//! - the snapshot save (`01:FE4B`, hooked at its RTL `01:FE7D`) keeps each present party record's high part in its
//!   character's snapshot, and the restore (`01:FE7E`, hooked at its RTL `01:FEB0`) puts it back;
//! - the copy of a party record to side A (`05:AE81`, hooked at its RTL `05:AE94`) and of an enemy record to side B
//!   (`05:AE16`, hooked at its RTL `05:AE29`) takes the high part along;
//! - the copies back, side A to a party record and side B to an enemy record, are hooked at the `PLA` after each loop
//!   (`05:9D68` and `05:9D86`).
//!
//! The side copies name their record by `$7E:14C3` (party) and `$7E:14C5` (enemy): the copy routines store their `Y` there
//! at entry, and the copies back read the same words. The slot is the word over the record stride.

use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, ReturnKind};

use super::state::{
    BLOCK, ENEMY_SLOTS, MARKER, MARKER_AT, PARTY_SLOTS, enemy_record, party_record, record_high,
    side_record, snapshot_high,
};

/// `01:FE7D`: the RTL of the snapshot save, after the loop has kept the low 24 bits of each present party record.
pub(super) const SNAPSHOT_SAVE_KEY: u32 = 0x01_FE7D;
/// `01:FEB0`: the RTL of the snapshot restore, after the loop has put the low 24 bits back into each present party record.
pub(super) const SNAPSHOT_RESTORE_KEY: u32 = 0x01_FEB0;
/// `05:AE94`: the RTL of the copy of a party record to side A.
pub(super) const SIDE_A_COPY_KEY: u32 = 0x05_AE94;
/// `05:AE29`: the RTL of the copy of an enemy record to side B.
pub(super) const SIDE_B_COPY_KEY: u32 = 0x05_AE29;
/// `05:9D68`: the `PLA` after the loop that copies side A to the party record named by `$7E:14C3`.
pub(super) const SIDE_A_COPY_BACK_KEY: u32 = 0x05_9D68;
/// `05:9D86`: the `PLA` after the loop that copies side B to the enemy record named by `$7E:14C5`.
pub(super) const SIDE_B_COPY_BACK_KEY: u32 = 0x05_9D86;

/// `$7E:14C3`: the offset of the party record the copy routines are at (a word; `battle_party_offset`).
const PARTY_OFFSET_AT: u32 = 0x7E_14C3;
/// `$7E:14C5`: the offset from `$7E:0E00` of the enemy record the copy routines are at (a word; `battle_enemy_offset`).
const ENEMY_OFFSET_AT: u32 = 0x7E_14C5;
/// The stride of a record table: 32 bytes per record.
const RECORD_STRIDE: u32 = 0x20;
/// The record's flags byte (`+00`): the record is present when it is `$C0` or more.
const PRESENT: u8 = 0xC0;
/// The record's character id (`+1C`).
const ID_AT: u32 = 0x1C;

/// Whether the block's marker is in place. A neutral block declines every hook.
fn live(ctx: &mut Ctx<'_>) -> bool {
    MARKER.iter().enumerate().all(|(k, &byte)| ctx.read8(MARKER_AT + k as u32) == byte)
}

/// The character id of the party record at `record`, or `None` when the record is absent.
fn present_id(ctx: &Ctx<'_>, record: u32) -> Option<u8> {
    (ctx.peek8(record) >= PRESENT).then(|| ctx.peek8(record + ID_AT))
}

/// The slot of the record at `offset`, the `Y` a copy routine holds: the record's offset from the first record of its table
/// (`$7E:0B00` for the party, `$7E:0E00` for the enemies). It must be a multiple of the stride and name one of the table's
/// `slots` records; any other offset gives `None`.
fn slot_of(offset: u16, slots: u32) -> Option<u32> {
    let offset = u32::from(offset);
    (offset.is_multiple_of(RECORD_STRIDE) && offset / RECORD_STRIDE < slots)
        .then_some(offset / RECORD_STRIDE)
}

/// Copies the two bytes of the high part at block offset `from` to block offset `to`.
fn copy_high(ctx: &mut Ctx<'_>, from: usize, to: usize) {
    let high = ctx.read16(BLOCK + from as u32);
    ctx.write16(BLOCK + to as u32, high);
}

/// Copies the high part of the record at bus address `from` to the record at `to`, when both records have one.
fn copy_record_high(ctx: &mut Ctx<'_>, from: u32, to: u32) {
    if let (Some(from), Some(to)) = (record_high(from), record_high(to)) {
        copy_high(ctx, from, to);
    }
}

/// `01:FE7D`: the RTL of the snapshot save. Each present party record's high part goes to its character's snapshot. The
/// records are visited in slot order, as the low bits are, so a character in two slots keeps the later slot's high part.
/// A character id past the twelve snapshots has no high part to keep, so its record is skipped.
fn snapshot_save(ctx: &mut Ctx<'_>) {
    if live(ctx) {
        for slot in 0..PARTY_SLOTS {
            let record = party_record(slot);
            if let Some(id) = present_id(ctx, record)
                && let Some(to) = snapshot_high(id)
                && let Some(from) = record_high(record)
            {
                copy_high(ctx, from, to);
            }
        }
    }
    ctx.resume_at(SNAPSHOT_SAVE_KEY);
}

/// `01:FEB0`: the RTL of the snapshot restore. Each present party record takes its character's snapshot high part; a
/// character id past the twelve snapshots keeps the record's own high part.
fn snapshot_restore(ctx: &mut Ctx<'_>) {
    if live(ctx) {
        for slot in 0..PARTY_SLOTS {
            let record = party_record(slot);
            if let Some(id) = present_id(ctx, record)
                && let Some(from) = snapshot_high(id)
                && let Some(to) = record_high(record)
            {
                copy_high(ctx, from, to);
            }
        }
    }
    ctx.resume_at(SNAPSHOT_RESTORE_KEY);
}

/// `05:AE94`: the RTL of the copy of the party record at `$7E:14C3` to side A.
fn side_a_copy(ctx: &mut Ctx<'_>) {
    if live(ctx)
        && let Some(slot) = slot_of(ctx.peek16(PARTY_OFFSET_AT), PARTY_SLOTS)
    {
        copy_record_high(ctx, party_record(slot), side_record(false));
    }
    ctx.resume_at(SIDE_A_COPY_KEY);
}

/// `05:AE29`: the RTL of the copy of the enemy record at `$7E:14C5` to side B.
fn side_b_copy(ctx: &mut Ctx<'_>) {
    if live(ctx)
        && let Some(slot) = slot_of(ctx.peek16(ENEMY_OFFSET_AT), ENEMY_SLOTS)
    {
        copy_record_high(ctx, enemy_record(slot), side_record(true));
    }
    ctx.resume_at(SIDE_B_COPY_KEY);
}

/// `05:9D68`: the `PLA` after the loop that copies side A to the party record at `$7E:14C3`.
fn side_a_copy_back(ctx: &mut Ctx<'_>) {
    if live(ctx)
        && let Some(slot) = slot_of(ctx.peek16(PARTY_OFFSET_AT), PARTY_SLOTS)
    {
        copy_record_high(ctx, side_record(false), party_record(slot));
    }
    ctx.resume_at(SIDE_A_COPY_BACK_KEY);
}

/// `05:9D86`: the `PLA` after the loop that copies side B to the enemy record at `$7E:14C5`.
fn side_b_copy_back(ctx: &mut Ctx<'_>) {
    if live(ctx)
        && let Some(slot) = slot_of(ctx.peek16(ENEMY_OFFSET_AT), ENEMY_SLOTS)
    {
        copy_record_high(ctx, side_record(true), enemy_record(slot));
    }
    ctx.resume_at(SIDE_B_COPY_BACK_KEY);
}

/// The hook entries of the snapshot and the side copies, all `Patch`: they cost nothing and never call the original.
pub(super) fn hook_entries() -> Vec<HookEntry> {
    let patch = |name: &str, key: u32, func: ssd_oracle::RoutineFn| HookEntry {
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
        patch("mod_snapshot_save", SNAPSHOT_SAVE_KEY, snapshot_save),
        patch("mod_snapshot_restore", SNAPSHOT_RESTORE_KEY, snapshot_restore),
        patch("mod_side_a_copy", SIDE_A_COPY_KEY, side_a_copy),
        patch("mod_side_b_copy", SIDE_B_COPY_KEY, side_b_copy),
        patch("mod_side_a_copy_back", SIDE_A_COPY_BACK_KEY, side_a_copy_back),
        patch("mod_side_b_copy_back", SIDE_B_COPY_BACK_KEY, side_b_copy_back),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_and_offset_words_are_the_instructions_named_in_the_documentation() {
        assert_eq!(SNAPSHOT_SAVE_KEY, 0x01_FE7D, "the RTL of the save at 01:FE4B");
        assert_eq!(SNAPSHOT_RESTORE_KEY, 0x01_FEB0, "the RTL of the restore at 01:FE7E");
        assert_eq!(SIDE_A_COPY_KEY, 0x05_AE94, "the RTL of the copy to side A at 05:AE81");
        assert_eq!(SIDE_B_COPY_KEY, 0x05_AE29, "the RTL of the copy to side B at 05:AE16");
        assert_eq!(SIDE_A_COPY_BACK_KEY, 0x05_9D68, "the PLA after the copy back to the party");
        assert_eq!(SIDE_B_COPY_BACK_KEY, 0x05_9D86, "the PLA after the copy back to the enemies");
        assert_eq!(PARTY_OFFSET_AT, 0x7E_14C3);
        assert_eq!(ENEMY_OFFSET_AT, 0x7E_14C5);
    }

    #[test]
    fn the_hooks_are_six_patches_in_the_order_of_the_copies_and_cost_nothing() {
        let entries = hook_entries();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "mod_snapshot_save",
                "mod_snapshot_restore",
                "mod_side_a_copy",
                "mod_side_b_copy",
                "mod_side_a_copy_back",
                "mod_side_b_copy_back",
            ]
        );
        let keys: Vec<u32> = entries.iter().map(|entry| entry.key).collect();
        assert_eq!(
            keys,
            [
                SNAPSHOT_SAVE_KEY,
                SNAPSHOT_RESTORE_KEY,
                SIDE_A_COPY_KEY,
                SIDE_B_COPY_KEY,
                SIDE_A_COPY_BACK_KEY,
                SIDE_B_COPY_BACK_KEY,
            ]
        );
        for entry in &entries {
            assert_eq!(entry.mode, HookMode::Patch, "{}", entry.name);
            assert_eq!(entry.cycles, Some(0), "{}", entry.name);
            assert!(matches!(entry.action, HookAction::Routine { .. }), "{}", entry.name);
        }
    }

    #[test]
    fn no_key_is_a_cheat_key_and_each_is_the_only_redirector_at_its_key() {
        let cheats = crate::cheats::entry_points();
        let redirectors = crate::mods::entry_points();
        for entry in hook_entries() {
            assert!(!cheats.contains(&entry.key), "{} is a cheat's key", entry.name);
            assert_eq!(
                redirectors.iter().filter(|&&key| key == entry.key).count(),
                1,
                "{} is not registered once",
                entry.name
            );
        }
    }

    #[test]
    fn a_party_offset_names_its_slot_and_only_the_nine_records_have_one() {
        assert_eq!(slot_of(0x00, PARTY_SLOTS), Some(0));
        assert_eq!(slot_of(0x20, PARTY_SLOTS), Some(1));
        assert_eq!(slot_of(0x100, PARTY_SLOTS), Some(8), "the ninth record starts past one byte");
        assert_eq!(slot_of(0x120, PARTY_SLOTS), None, "there is no tenth record");
        assert_eq!(slot_of(0x10, PARTY_SLOTS), None, "an offset inside a record is no record");
        assert_eq!(slot_of(0xFFFF, PARTY_SLOTS), None);
    }

    #[test]
    fn an_enemy_offset_names_its_slot_and_only_the_five_records_have_one() {
        assert_eq!(slot_of(0x00, ENEMY_SLOTS), Some(0));
        assert_eq!(slot_of(0x80, ENEMY_SLOTS), Some(4));
        assert_eq!(slot_of(0xA0, ENEMY_SLOTS), None, "there is no sixth enemy record");
        assert_eq!(slot_of(0x120, ENEMY_SLOTS), None, "a party offset is not an enemy record");
    }

    #[test]
    fn the_offset_of_each_record_address_gives_back_its_slot() {
        for slot in 0..PARTY_SLOTS {
            let offset = party_record(slot) - party_record(0);
            assert_eq!(slot_of(offset as u16, PARTY_SLOTS), Some(slot));
        }
        for slot in 0..ENEMY_SLOTS {
            let offset = enemy_record(slot) - enemy_record(0);
            assert_eq!(slot_of(offset as u16, ENEMY_SLOTS), Some(slot));
        }
    }
}

/// The hooks run headless on the batch test pack (`.work/m10/tables-pack`, as in the level check of `tables`): the snapshot
/// round trip (the save at `01:FE4B`, then the restore at `01:FE7E` after the BP changed), the copies to the sides, and a
/// neutral block that declines every hook.
#[cfg(test)]
mod round_trip_rom {
    use super::*;
    use ssd_oracle::{Joypad, Machine, MemorySaveStore, NullAudio, StatusRegister, Stopped, Stops};

    /// The call stub: a `JSL` to the routine is written here and run from `$7F:FE00`.
    const STUB: u32 = 0x7F_FE00;
    /// The address the stub returns to (after its four bytes).
    const BACK: u32 = STUB + 4;
    /// The snapshot save and the snapshot restore, by their `JSL` targets.
    const SAVE: u32 = 0x01_FE4B;
    const RESTORE: u32 = 0x01_FE7E;
    /// `$7E:4C00`: the snapshot of character 0 (3 bytes).
    const SNAPSHOT: u32 = 0x7E_4C00;
    /// `05:AE81` (party to side A) and `05:AE16` (enemy to side B), by their `JSL` targets.
    const PARTY_TO_SIDE_A: u32 = 0x05_AE81;
    const ENEMY_TO_SIDE_B: u32 = 0x05_AE16;

    fn boot() -> Machine {
        let raw = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let rom = ssd_rom::normalize(&raw, None).unwrap().bytes;
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.work/m10/tables-pack");
        let pack = ssd_mod::Pack::from_dir(&dir).unwrap();
        let image = ssd_mod::build(&rom, &[pack]).unwrap().bytes;
        let mut machine = Machine::with_image(
            image,
            ssd_rom::crc32(&rom),
            Box::new(MemorySaveStore::default()),
            ssd_oracle::DEFAULT_WRAM_SEED,
        )
        .unwrap();
        let mut table = ssd_oracle::HookTable::new();
        crate::mods::install(&mut table);
        machine.set_hook_table(Some(table));
        // Boot: the game runs its title with no input, so the work RAM is the game's own before the records are staged.
        let frame = Stops { frame_end: true, ..Stops::default() };
        for _ in 0..300 {
            machine.run(&frame, &mut |_| Joypad::default(), &mut NullAudio, 1).unwrap();
        }
        machine
    }

    /// Runs the bank-01 routine at `routine` from the stub, with `Y` = `y`, until the stub's return at `BACK`.
    fn call(machine: &mut Machine, routine: u32, y: u16) {
        let [low, high, bank, _] = routine.to_le_bytes();
        machine.write_wram(STUB, &[0x22, low, high, bank]).unwrap();
        let mut registers = machine.cpu_registers();
        registers.pbr = (STUB >> 16) as u8;
        registers.pc = (STUB & 0xFFFF) as u16;
        registers.dbr = 0;
        registers.d = 0;
        registers.y = y;
        registers.p = StatusRegister::from(0x24);
        machine.set_cpu_registers(registers);
        machine.set_breakpoint(BACK);
        let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
        for _ in 0..4 {
            match machine.run(&stops, &mut |_| Joypad::default(), &mut NullAudio, 2).unwrap() {
                Stopped::Breakpoint { key } if key == BACK => return,
                Stopped::FrameEnd => {}
                other => panic!("the routine did not return: {other:?}"),
            }
        }
        panic!("the routine did not return within the frames of the call");
    }

    /// The bytes at the bus address `address` (work RAM, `$7E` to `$7F`).
    fn bytes(machine: &Machine, address: u32, len: usize) -> Vec<u8> {
        let at = (address - 0x7E_0000) as usize;
        machine.wram()[at..at + len].to_vec()
    }

    #[test]
    #[ignore = "needs SSD_ROM_PATH and the test pack in .work/m10/tables-pack"]
    fn the_save_keeps_the_high_part_and_the_restore_puts_it_back() {
        let mut machine = boot();
        let record = party_record(0);
        // Party slot 0 is character 0, present (`$C0`), with the low 24 bits `$123456`; the other eight records are absent.
        machine.write_wram(record, &[0xC0]).unwrap();
        machine.write_wram(record + ID_AT, &[0]).unwrap();
        machine.write_wram(record + 0x07, &[0x56, 0x34, 0x12]).unwrap();
        for slot in 1..PARTY_SLOTS {
            machine.write_wram(party_record(slot), &[0]).unwrap();
        }
        // The live block: the marker, and slot 0's high part 2, so the BP is `2 << 24 | $123456`.
        machine.write_wram(MARKER_AT, &MARKER).unwrap();
        machine.write_wram(BLOCK, &[0x02, 0x00]).unwrap();

        call(&mut machine, SAVE, 0);
        assert_eq!(
            bytes(&machine, SNAPSHOT, 3),
            [0x56, 0x34, 0x12],
            "the save keeps the low 24 bits, as the game does"
        );
        let high_at = BLOCK + snapshot_high(0).unwrap() as u32;
        assert_eq!(
            bytes(&machine, high_at, 2),
            [0x02, 0x00],
            "the save keeps the high part of character 0"
        );

        // The BP changes after the save: the low bits and the high part are cleared.
        machine.write_wram(record + 0x07, &[0, 0, 0]).unwrap();
        machine.write_wram(BLOCK, &[0, 0]).unwrap();
        assert_eq!(
            bytes(&machine, record + 0x07, 3),
            [0, 0, 0],
            "the change of the low bits is in place"
        );
        assert_eq!(bytes(&machine, BLOCK, 2), [0, 0], "the change of the high part is in place");

        call(&mut machine, RESTORE, 0);
        assert_eq!(
            bytes(&machine, record + 0x07, 3),
            [0x56, 0x34, 0x12],
            "the restore puts the low bits back"
        );
        assert_eq!(bytes(&machine, BLOCK, 2), [0x02, 0x00], "the restore puts the high part back");
    }

    /// The bus address of the high part of the record at `record` (a party, enemy or side record).
    fn high_at(record: u32) -> u32 {
        BLOCK + record_high(record).unwrap() as u32
    }

    #[test]
    #[ignore = "needs SSD_ROM_PATH and the test pack in .work/m10/tables-pack"]
    fn the_copies_to_the_sides_take_the_high_parts_along() {
        let mut machine = boot();
        // Party slot 1 has the high part 5, enemy slot 1 the high part 7; both sides start with none, and the block is live.
        machine.write_wram(MARKER_AT, &MARKER).unwrap();
        machine.write_wram(high_at(party_record(1)), &[0x05, 0x00]).unwrap();
        machine.write_wram(high_at(enemy_record(1)), &[0x07, 0x00]).unwrap();
        machine.write_wram(high_at(side_record(false)), &[0, 0]).unwrap();
        machine.write_wram(high_at(side_record(true)), &[0, 0]).unwrap();

        // `05:AE81` copies party slot 1 (`Y` = `$20`) to side A, and `05:AE16` copies enemy slot 1 (`Y` = `$20`) to side B.
        call(&mut machine, PARTY_TO_SIDE_A, 0x20);
        assert_eq!(
            bytes(&machine, PARTY_OFFSET_AT, 2),
            [0x20, 0x00],
            "the copy stores its offset at `$14C3`"
        );
        assert_eq!(
            bytes(&machine, high_at(side_record(false)), 2),
            [0x05, 0x00],
            "side A takes slot 1's high part"
        );
        call(&mut machine, ENEMY_TO_SIDE_B, 0x20);
        assert_eq!(
            bytes(&machine, ENEMY_OFFSET_AT, 2),
            [0x20, 0x00],
            "the copy stores its offset at `$14C5`"
        );
        assert_eq!(
            bytes(&machine, high_at(side_record(true)), 2),
            [0x07, 0x00],
            "side B takes enemy 1's high part"
        );
    }

    #[test]
    #[ignore = "needs SSD_ROM_PATH and the test pack in .work/m10/tables-pack"]
    fn a_neutral_block_leaves_the_high_parts_alone() {
        let mut machine = boot();
        // Party slot 0 is present with the high part 2 and character 0's snapshot high part is `$EE`, but there is no marker.
        let record = party_record(0);
        machine.write_wram(record, &[0xC0]).unwrap();
        machine.write_wram(record + ID_AT, &[0]).unwrap();
        machine.write_wram(MARKER_AT, &[0, 0, 0, 0]).unwrap();
        machine.write_wram(high_at(record), &[0x02, 0x00]).unwrap();
        let snapshot = BLOCK + snapshot_high(0).unwrap() as u32;
        machine.write_wram(snapshot, &[0xEE, 0xEE]).unwrap();

        call(&mut machine, SAVE, 0);
        assert_eq!(
            bytes(&machine, snapshot, 2),
            [0xEE, 0xEE],
            "the save declines without the marker"
        );
        machine.write_wram(high_at(record), &[0x09, 0x00]).unwrap();
        call(&mut machine, RESTORE, 0);
        assert_eq!(
            bytes(&machine, high_at(record), 2),
            [0x09, 0x00],
            "the restore declines without the marker"
        );
    }
}
