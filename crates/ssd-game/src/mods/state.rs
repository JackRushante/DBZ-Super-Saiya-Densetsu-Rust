//! The mod state block (M10, Task 9): the work RAM the packs' wide BP and the extended flags need, outside the save image
//! and outside the cheat block. Its layout is decided here; the sidecar `ssd.mods` keeps it per save slot
//! (`ssd_menus::mods_state`), and a replay carries it as the application event [`MODS_TAG`].
//!
//! **A wide BP is 40 bits.** The record keeps the low 24 bits at `+07..+09` (the game's own bytes, read by every routine),
//! and the high 16 bits (bits 24 to 39) are kept here. A record without a high part is a 24-bit BP, as before.
//!
//! **Layout** (`$7F:FAFC` to `$7F:FBFF`; every field is valid only with the marker, else reads as zero):
//! - `$7F:FAFC`..`$7F:FAFF`: the marker `SSDm`. Without it the block is neutral whatever its bytes are (the power-on fill of
//!   work RAM is random, as for the cheat block).
//! - `$7F:FB00 + 0x00`: the high parts of the 9 party records (2 bytes each, little endian; party slot `i` at `+2i`).
//! - `+0x12`: the high parts of the 5 enemy records (enemy slot `j` at `+0x12 + 2j`).
//! - `+0x1C`: the high parts of the side records, `$7E:1400` (side A, `+0x1C`) and `$7E:1440` (side B, `+0x1E`).
//! - `+0x20`: the high parts of the BP snapshot `$7E:4C00 + 3 * id` for the 12 characters (character `id` at `+0x20 + 2id`).
//! - `+0x38` to `+0x3B`: the marker `SSDc` of a completed game (`chapter`).
//! - `+0x50`: the kind of the battle, for `enemy_scaling` with offsets: the marker `SSDe` (4 bytes), then one byte that is 1
//!   between a random encounter's group pick and the build of its records, and one that is 1 while the battle whose records
//!   were built last is a random encounter. Its own marker, so it is valid whether or not this block is live.
//! - `+0x60`: the wide levels of `free_progression`'s `level_cap`: the marker `SSDl` (4 bytes), then the level of each of the
//!   12 characters as 16 bits (character `id` at `+0x64 + 2id`), then the EXP's bits 24 to 31 of each (`+0x7C + id`). Its own
//!   marker, like the kind of the battle, so it is valid whether or not the block is live; the sidecar keeps it (`levels`).
//! - the rest to `+0xFF`: free, zero.
//!
//! **Decision: the extended flags are not copied into the block.** The 256 flag bytes do not fit beside the high parts in
//! the block's 256 bytes, and the flags already sit at `$7F:FD00` outside the save image. The sidecar keeps them with their
//! marker `SSDf` (`$7F:FCFC`, Batch 2a), and a replay carries them in the same application event as the high parts.
//!
//! The reference replays never write the range (`scene_ext`'s scan, which covers `$7F:FAFC`..`$7F:FBFF` too).

use ssd_menus::mods_state::{FLAG_BYTES, HIGH_BYTES, LEVEL_BYTES, ModState};
use ssd_oracle::HostEvent;

/// The application event that carries the mod state of a run in a format 2 replay: its payload is
/// [`ModState::to_bytes`]. The next free tag after the bag (`BIND_TAG`, 1), the cheats (2) and the spawner (3).
pub const MODS_TAG: u8 = 4;

/// The bus address of the marker of the block (`SSDm`).
pub const MARKER_AT: u32 = 0x7F_FAFC;
/// The marker of the block: the block is valid only when these four bytes are at [`MARKER_AT`].
pub const MARKER: [u8; 4] = *b"SSDm";
/// The bus address of the block (256 bytes).
pub const BLOCK: u32 = 0x7F_FB00;
/// The bytes of the block the layout uses ([`HIGH_BYTES`] of them, from the block's start).
pub const USED: usize = HIGH_BYTES;

/// The block offsets of the high parts (2 bytes each).
pub const PARTY_HIGH: usize = 0x00;
pub const ENEMY_HIGH: usize = 0x12;
pub const SIDE_HIGH: usize = 0x1C;
pub const SNAPSHOT_HIGH: usize = 0x20;

/// The number of party records, enemy records, side records and characters with a BP snapshot.
pub const PARTY_SLOTS: u32 = 9;
pub const ENEMY_SLOTS: u32 = 5;
pub const SIDE_SLOTS: u32 = 2;
pub const CHARACTERS: u8 = 12;

/// The bus addresses of the records the high parts belong to: the first party record (stride `$20`), the first enemy
/// record (stride `$20`), the side records, and the BP snapshot (stride 3).
const PARTY_RECORDS: u32 = 0x7E_0B00;
const ENEMY_RECORDS: u32 = 0x7E_0E00;
const SIDE_A: u32 = 0x7E_1400;
const SIDE_B: u32 = 0x7E_1440;
const RECORD_STRIDE: u32 = 0x20;
const SNAPSHOT: u32 = 0x7E_4C00;

/// Scratch of the wide-BP hooks: free bytes of the block (`+0x40` to `+0x4F`), outside [`HIGH_BYTES`], so no save keeps
/// them. A hook writes them when a routine starts (the damage shift, a boost) and reads them back when it ends; a
/// restore that runs without a boost of its own reads what the last boost left, as the original reads its stale
/// `$01EB..$01ED`.
/// The damage shift applied to both sides (`0` = none), one byte.
pub const SCRATCH_SHIFT: usize = 0x40;
/// Side A's low 24 BP before a damage shift (3 bytes, little endian).
pub const SCRATCH_SIDE_A_LOW: usize = 0x41;
/// Side B's low 24 BP before a damage shift (3 bytes, little endian).
pub const SCRATCH_SIDE_B_LOW: usize = 0x44;
/// The high part of the side record before a Kaioken boost (2 bytes), restored with the boost's low 24 bits.
pub const SCRATCH_KAIOKEN_HIGH: usize = 0x48;
/// The end (exclusive) of the scratch: the bytes from [`SCRATCH_SHIFT`] to here are zeroed when a state is applied.
pub const SCRATCH_END: usize = SCRATCH_KAIOKEN_HIGH + 2;

/// The kind of the battle (`scaling`): the block offset of its marker. Six bytes (the marker, the pending mark and the kind),
/// none of them saved with the block or zeroed with the scratch. The marker is its own, because the power-on fill of work RAM
/// is random.
pub const BATTLE_KIND: usize = 0x50;
/// The marker of the kind-of-battle bytes.
pub const KIND_MARKER: [u8; 4] = *b"SSDe";

/// The member pick of `party_boosts` (`party_boosts`, the HD driver): six bytes at `+0x58`, none of them saved with the block or
/// zeroed with the scratch. The marker `SSDq` (the power-on fill of work RAM is random), the object the pick is for and the row of
/// the party list of the first fighter the original accepts it for (`0xFF`: it accepts none). The hooks write them while an object
/// of the pack is used with its switch on; the driver answers the original's own fighter choice with them.
pub const PICK: usize = 0x58;
/// The marker of the pick.
pub const PICK_MARKER: [u8; 4] = *b"SSDq";
/// The bus address of the pick's marker, its object and its row.
pub const PICK_AT: u32 = BLOCK + PICK as u32;
pub const PICK_ITEM_AT: u32 = PICK_AT + 4;
pub const PICK_ROW_AT: u32 = PICK_AT + 5;

/// The wide levels (`levels`): the block offset of their marker, the marker, and the bus address of the 36 bytes behind it
/// (twelve levels of 16 bits, then twelve EXP high bytes).
pub const LEVELS_MARKER: usize = 0x60;
pub const LEVELS_MARK: [u8; 4] = *b"SSDl";
pub const LEVELS_MARKER_AT: u32 = BLOCK + LEVELS_MARKER as u32;
pub const LEVELS_AT: u32 = LEVELS_MARKER_AT + 4;
/// The bus address of the wide level of character `id` (16 bits) and of its EXP's high byte.
#[must_use]
pub fn wide_level_at(id: u8) -> u32 {
    LEVELS_AT + 2 * u32::from(id)
}
#[must_use]
pub fn exp_high_at(id: u8) -> u32 {
    LEVELS_AT + 2 * u32::from(CHARACTERS) + u32::from(id)
}

/// The shift a damage pair needs: the smallest `s` such that the larger of the two BPs, plus a quarter of it (the class
/// bonus of `05:B6F2`, which can add 25 percent before the damage), fits 24 bits. Both BPs are shifted by it.
#[must_use]
pub fn damage_shift(a: u64, d: u64) -> u32 {
    let larger = a.max(d);
    let mut shift = 0;
    while (larger >> shift) + ((larger >> shift) / 4) > 0xFF_FFFF {
        shift += 1;
    }
    shift
}

/// The bus address of the extended flags (256 bytes) and of their marker (`SSDf`).
pub const FLAGS: u32 = 0x7F_FD00;
pub const FLAGS_MARKER_AT: u32 = 0x7F_FCFC;
pub const FLAGS_MARKER: [u8; 4] = *b"SSDf";

/// The first byte of work RAM in the bus (`$7E:0000`): the index of an address in a work RAM slice is `address - WRAM`.
const WRAM: u32 = 0x7E_0000;

/// The index of a bus address in a work RAM slice.
fn at(address: u32) -> usize {
    (address - WRAM) as usize
}

/// The block offset of the high part of the record at the bus address `record` (a party, enemy or side record's
/// first byte), or `None` for any other address.
#[must_use]
pub fn record_high(record: u32) -> Option<usize> {
    let party = record.checked_sub(PARTY_RECORDS).filter(|k| k % RECORD_STRIDE == 0);
    if let Some(k) = party.map(|k| k / RECORD_STRIDE).filter(|&k| k < PARTY_SLOTS) {
        return Some(PARTY_HIGH + 2 * k as usize);
    }
    let enemy = record.checked_sub(ENEMY_RECORDS).filter(|k| k % RECORD_STRIDE == 0);
    if let Some(k) = enemy.map(|k| k / RECORD_STRIDE).filter(|&k| k < ENEMY_SLOTS) {
        return Some(ENEMY_HIGH + 2 * k as usize);
    }
    match record {
        SIDE_A => Some(SIDE_HIGH),
        SIDE_B => Some(SIDE_HIGH + 2),
        _ => None,
    }
}

/// The block offset of the high part of character `id`'s BP snapshot (`$7E:4C00 + 3 * id`), or `None` for an id past 11.
#[must_use]
pub fn snapshot_high(id: u8) -> Option<usize> {
    (id < CHARACTERS).then_some(SNAPSHOT_HIGH + 2 * usize::from(id))
}

/// The bus address of the snapshot's first byte for character `id` (`$7E:4C00 + 3 * id`), for the wrappers that copy it.
#[must_use]
pub fn snapshot_at(id: u8) -> u32 {
    SNAPSHOT + 3 * u32::from(id)
}

/// The bus address of the record that a party or enemy slot is at (the record stride is `$20`).
#[must_use]
pub fn party_record(slot: u32) -> u32 {
    PARTY_RECORDS + RECORD_STRIDE * slot
}

/// The bus address of enemy slot `slot`'s record.
#[must_use]
pub fn enemy_record(slot: u32) -> u32 {
    ENEMY_RECORDS + RECORD_STRIDE * slot
}

/// The bus address of the side record `$7E:1400` (A) or `$7E:1440` (B).
#[must_use]
pub fn side_record(b: bool) -> u32 {
    if b { SIDE_B } else { SIDE_A }
}

/// Whether the block's marker is in `wram` (a work RAM slice, index `address - $7E:0000`).
#[must_use]
pub fn block_live(wram: &[u8]) -> bool {
    wram[at(MARKER_AT)..at(MARKER_AT) + 4] == MARKER
}

/// Whether the extended flags' marker is in `wram`.
#[must_use]
pub fn flags_live(wram: &[u8]) -> bool {
    wram[at(FLAGS_MARKER_AT)..at(FLAGS_MARKER_AT) + 4] == FLAGS_MARKER
}

/// Whether the wide levels' marker is in `wram`.
#[must_use]
pub fn levels_live(wram: &[u8]) -> bool {
    wram[at(LEVELS_MARKER_AT)..at(LEVELS_MARKER_AT) + 4] == LEVELS_MARK
}

/// The level a record's byte and the wide level of its character make together: the wide level while the byte agrees with it
/// (`min(wide, 99) = byte`, the byte being the wide level saturated at 99), else the byte.
#[must_use]
pub fn level_agreeing(wide: u16, byte: u8) -> u16 {
    let cap = u16::from(ssd_menus::levels::RECORD_LEVEL_MAX);
    if wide > cap && wide.min(cap) == u16::from(byte) { wide } else { u16::from(byte) }
}

/// The level of character `id` whose record's level byte is `byte`, read from the mod state in `wram` (a snapshot of work RAM
/// from `$7E:0000`, at least 128 KiB): the wide level when the wide levels' marker is there and the byte agrees with it, else the
/// byte. This is the reader the HD screens use; the game's own is `levels::effective_level`, with the same rule.
#[must_use]
pub fn level_in(wram: &[u8], id: u8, byte: u8) -> u16 {
    if id < CHARACTERS && levels_live(wram) {
        let at = at(wide_level_at(id));
        return level_agreeing(u16::from_le_bytes([wram[at], wram[at + 1]]), byte);
    }
    u16::from(byte)
}

/// The EXP's bits 24 to 31 of character `id` in `wram`, or 0 without the wide levels' marker.
#[must_use]
pub fn exp_high_in(wram: &[u8], id: u8) -> u8 {
    if id < CHARACTERS && levels_live(wram) { wram[at(exp_high_at(id))] } else { 0 }
}

/// The high 16 bits at block offset `offset` of `wram`, or zero when the block is not live (a neutral block).
#[must_use]
pub fn high_in(wram: &[u8], offset: usize) -> u16 {
    if !block_live(wram) {
        return 0;
    }
    let at = at(BLOCK) + offset;
    u16::from_le_bytes([wram[at], wram[at + 1]])
}

/// The 40-bit BP of a record: its 24 low bits (`low`) and the high part at block offset `offset`.
#[must_use]
pub fn wide(low: u32, high: u16) -> u64 {
    u64::from(high) << 24 | u64::from(low & 0xFF_FFFF)
}

/// Splits a BP into the 24 low bits the record keeps and the 16 high bits the block keeps. Saturates at 40 bits.
#[must_use]
pub fn split(bp: u64) -> (u32, u16) {
    let bp = bp.min((1 << 40) - 1);
    ((bp & 0xFF_FFFF) as u32, (bp >> 24) as u16)
}

/// The mod state of a save made now: each part with its marker, the bytes as they are in `wram`.
#[must_use]
pub fn capture(wram: &[u8]) -> ModState {
    let flags = flags_live(wram).then(|| {
        let at = at(FLAGS);
        wram[at..at + FLAG_BYTES].to_vec()
    });
    let high = block_live(wram).then(|| {
        let at = at(BLOCK);
        wram[at..at + HIGH_BYTES].to_vec()
    });
    let levels = levels_live(wram).then(|| {
        let at = at(LEVELS_AT);
        wram[at..at + LEVEL_BYTES].to_vec()
    });
    ModState { flags, high, levels }
}

/// The host events that put `state` into work RAM: the markers with the bytes of each part that has one. A part without
/// a state is not written, so a run keeps what it has there.
#[must_use]
pub fn apply_events(state: &ModState) -> Vec<HostEvent> {
    let mut events = Vec::new();
    if let Some(high) = &state.high {
        events.push(HostEvent::Wram { address: BLOCK, bytes: high.clone() });
        // The scratch of the wide-BP hooks starts at zero, so the console's power-on fill is never read as a value.
        events.push(HostEvent::Wram {
            address: BLOCK + SCRATCH_SHIFT as u32,
            bytes: vec![0; SCRATCH_END - SCRATCH_SHIFT],
        });
        events.push(HostEvent::Wram { address: MARKER_AT, bytes: MARKER.to_vec() });
    }
    if let Some(levels) = &state.levels {
        events.push(HostEvent::Wram { address: LEVELS_AT, bytes: levels.clone() });
        events.push(HostEvent::Wram { address: LEVELS_MARKER_AT, bytes: LEVELS_MARK.to_vec() });
    }
    if let Some(flags) = &state.flags {
        events.push(HostEvent::Wram { address: FLAGS, bytes: flags.clone() });
        events.push(HostEvent::Wram { address: FLAGS_MARKER_AT, bytes: FLAGS_MARKER.to_vec() });
    }
    events
}

/// The host events that make the run's mod state neutral when `wram` has a marker: the markers are cleared (the bytes
/// behind them are then ignored). Nothing is written when there is no marker, so a fresh run is not touched.
#[must_use]
pub fn clear_events(wram: &[u8]) -> Vec<HostEvent> {
    let mut events = Vec::new();
    if block_live(wram) {
        events.push(HostEvent::Wram { address: MARKER_AT, bytes: vec![0; 4] });
    }
    if levels_live(wram) {
        events.push(HostEvent::Wram { address: LEVELS_MARKER_AT, bytes: vec![0; 4] });
    }
    if flags_live(wram) {
        events.push(HostEvent::Wram { address: FLAGS_MARKER_AT, bytes: vec![0; 4] });
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Vec<u8> {
        vec![0xA5; 0x2_0000]
    }

    #[test]
    fn the_block_is_outside_the_save_image_and_the_other_blocks_and_holds_its_parts() {
        const _: () = assert!(MARKER_AT + 4 == BLOCK, "the marker lies just before the block");
        let block = BLOCK..BLOCK + 0x100;
        assert!(block.start >= 0x7F_F001, "above the save image and the lower upper RAM");
        assert!(!(BLOCK..BLOCK + 0x100).contains(&FLAGS), "the flags are another bank");
        assert!(!(BLOCK..BLOCK + 0x100).contains(&0x7F_FF00), "the cheat block is another range");
        let used = [
            PARTY_HIGH + 2 * PARTY_SLOTS as usize,
            ENEMY_HIGH + 2 * ENEMY_SLOTS as usize,
            SIDE_HIGH + 2 * SIDE_SLOTS as usize,
            SNAPSHOT_HIGH + 2 * CHARACTERS as usize,
        ];
        assert_eq!(used, [0x12, 0x1C, 0x20, 0x38]);
        assert_eq!(*used.last().unwrap(), USED, "the used part ends where the sidecar's ends");
    }

    #[test]
    fn each_record_and_character_has_its_own_high_part() {
        assert_eq!(record_high(party_record(0)), Some(0x00));
        assert_eq!(record_high(party_record(8)), Some(0x10));
        assert_eq!(record_high(party_record(9)), None, "nine party records");
        assert_eq!(record_high(enemy_record(0)), Some(0x12));
        assert_eq!(record_high(enemy_record(4)), Some(0x1A));
        assert_eq!(record_high(enemy_record(5)), None, "five enemy records");
        assert_eq!(record_high(side_record(false)), Some(0x1C));
        assert_eq!(record_high(side_record(true)), Some(0x1E));
        assert_eq!(record_high(0x7E_0B01), None, "a field inside a record is not a record");
        assert_eq!(snapshot_high(0), Some(0x20));
        assert_eq!(snapshot_high(11), Some(0x36));
        assert_eq!(snapshot_high(12), None);
        assert_eq!(snapshot_at(2), 0x7E_4C06, "the snapshot is 3 bytes a character");
    }

    #[test]
    fn a_wide_bp_is_its_low_24_bits_and_its_high_16_and_splits_back() {
        assert_eq!(wide(0x12_3456, 0), 0x12_3456);
        assert_eq!(wide(0xFF_FFFF, 1), 0x1FF_FFFF);
        // 50 000 000 = 0x2FAF080 = high 2 (bits 24..) and low 0xFAF080.
        assert_eq!(split(50_000_000), (0xFAF080, 2));
        assert_eq!(wide(0xFAF080, 2), 50_000_000);
        assert_eq!(split(u64::MAX), (0xFF_FFFF, 0xFFFF), "saturates at 40 bits");
    }

    #[test]
    fn a_neutral_block_reads_as_zero_high_parts_whatever_its_bytes_are() {
        let mut wram = blank();
        let at = (BLOCK - WRAM) as usize;
        wram[at] = 7;
        assert!(!block_live(&wram));
        assert_eq!(high_in(&wram, 0x00), 0, "no marker, no high part");
        assert!(capture(&wram).is_empty(), "nothing is kept without the markers");
        assert!(clear_events(&wram).is_empty(), "nothing is written to a fresh run");
    }

    #[test]
    fn a_live_block_reads_its_high_parts_and_is_captured_with_its_bytes() {
        let mut wram = blank();
        let marker = (MARKER_AT - WRAM) as usize;
        wram[marker..marker + 4].copy_from_slice(&MARKER);
        let at = (BLOCK - WRAM) as usize;
        wram[at + 0x20..at + 0x22].copy_from_slice(&[0x34, 0x12]);
        assert!(block_live(&wram));
        assert_eq!(high_in(&wram, SNAPSHOT_HIGH), 0x1234);
        let state = capture(&wram);
        assert_eq!(state.high.as_ref().map(Vec::len), Some(HIGH_BYTES));
        assert_eq!(state.high.as_ref().unwrap()[0x20], 0x34);
        assert_eq!(state.flags, None, "the flags have no marker here");
    }

    #[test]
    fn the_damage_shift_keeps_the_larger_pair_within_24_bits_with_the_class_bonus_and_the_ratio() {
        assert_eq!(damage_shift(10_000_000, 10), 0, "12.5 M with the bonus still fits");
        assert_eq!(damage_shift(50_000_000, 1_000), 2, "50 M is 12.5 M after two shifts");
        assert_eq!(
            damage_shift(0x00FF_FFFF, 0),
            1,
            "a full 24-bit BP needs one shift for its bonus"
        );
        // Equal ratios before and after: 50 M : 4 M = 12.5 : 1 = 12.5 M : 1 M after two shifts.
        let (a, d) = (50_000_000_u64, 4_000_000_u64);
        let shift = damage_shift(a, d);
        assert_eq!((a >> shift) / (d >> shift), a / d);
        assert_eq!((a >> shift, d >> shift), (12_500_000, 1_000_000));
    }

    #[test]
    fn a_state_is_applied_as_markers_and_bytes_and_cleared_as_markers() {
        let state = ModState {
            flags: Some(vec![0x55; FLAG_BYTES]),
            high: Some(vec![0x01; HIGH_BYTES]),
            levels: None,
        };
        let events = apply_events(&state);
        assert_eq!(events.len(), 5);
        assert!(
            matches!(&events[0], HostEvent::Wram { address, bytes } if *address == BLOCK && bytes.len() == HIGH_BYTES)
        );
        assert!(
            matches!(&events[1], HostEvent::Wram { address, bytes } if *address == BLOCK + SCRATCH_SHIFT as u32 && bytes.iter().all(|&byte| byte == 0) && bytes.len() == SCRATCH_END - SCRATCH_SHIFT)
        );
        assert!(
            matches!(&events[2], HostEvent::Wram { address, bytes } if *address == MARKER_AT && bytes == b"SSDm")
        );
        assert!(
            matches!(&events[3], HostEvent::Wram { address, bytes } if *address == FLAGS && bytes.len() == FLAG_BYTES)
        );
        assert!(
            matches!(&events[4], HostEvent::Wram { address, bytes } if *address == FLAGS_MARKER_AT && bytes == b"SSDf")
        );
        let mut wram = blank();
        for event in &events {
            if let HostEvent::Wram { address, bytes } = event {
                let at = (*address - WRAM) as usize;
                wram[at..at + bytes.len()].copy_from_slice(bytes);
            }
        }
        assert_eq!(capture(&wram), state, "what is applied is what is captured");
        let cleared = clear_events(&wram);
        assert_eq!(cleared.len(), 2, "both markers are cleared");
        assert!(
            matches!(&cleared[0], HostEvent::Wram { address, bytes } if *address == MARKER_AT && bytes == &vec![0; 4])
        );
    }

    #[test]
    fn the_wide_level_counts_only_while_the_records_byte_agrees_with_it() {
        assert_eq!(level_agreeing(150, 99), 150);
        assert_eq!(level_agreeing(150, 60), 60, "a routine wrote the byte by itself");
        assert_eq!(level_agreeing(99, 99), 99, "a wide level of 99 or less is no wide level");
        assert_eq!(level_agreeing(0, 47), 47);
        let mut wram = vec![0_u8; 0x2_0000];
        assert_eq!((level_in(&wram, 1, 99), exp_high_in(&wram, 1)), (99, 0), "no marker");
        let put = |wram: &mut Vec<u8>, address: u32, bytes: &[u8]| {
            wram[at(address)..at(address) + bytes.len()].copy_from_slice(bytes);
        };
        put(&mut wram, LEVELS_MARKER_AT, &LEVELS_MARK);
        put(&mut wram, wide_level_at(1), &321_u16.to_le_bytes());
        put(&mut wram, exp_high_at(1), &[7]);
        assert_eq!((level_in(&wram, 1, 99), exp_high_in(&wram, 1)), (321, 7));
        assert_eq!(level_in(&wram, 1, 98), 98);
        assert_eq!(
            (level_in(&wram, 12, 99), exp_high_in(&wram, 12)),
            (99, 0),
            "past the characters"
        );
    }
}
