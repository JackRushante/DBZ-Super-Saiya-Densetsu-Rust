//! The wide BP in the Kaioken boost and its restores (M10, Task 9, part C.2b): see [`super::state`] for the layout.
//!
//! The boost (`05:AB60`, documented in `docs/re/data/kaioken-bp.md`, section 4) saves a side record's 24-bit BP in
//! `$01EB..$01ED`, multiplies it by the multiplier `m` in `$01EF`, and adds the halved product to the record's low 24 bits;
//! the carry out is lost. A wide BP keeps its high 16 bits in the block, so the hooks here save and restore those with the
//! low bits, and the add works on the whole 40-bit BP ([`boosted`]):
//! - [`BOOST_START_KEY`] (`05:AB75`, after the optional restore of a previous boost): the side's high part goes back to its
//!   pre-boost value when a boost is in use, and is saved as the pre-boost value otherwise.
//! - [`BOOST_END_KEY`] (`05:ABE3`, the return after the add): the record gets the 40-bit sum, the low 24 bits in the record
//!   and the high 16 in the block.
//! - [`KI_RESTORE_KEY`] (`05:9D3D`) and [`STOP_RESTORE_KEY`] (`01:9FE0`): the two restores of the pre-boost BP put back the
//!   high part too.
//!
//! The pre-boost high part is kept in [`SCRATCH_KAIOKEN_HIGH`] (the block's `+0x48`), which no save keeps. Each hook declines
//! when the block is not live, and each resumes at its own key.
//!
//! The original writes the party record of the stop through the data bank. The hook addresses the same record in work RAM
//! (`$7E:0B00 + Y`), which is the same byte whenever the data bank is `$00` or `$7E`.

use super::state::{
    BLOCK, MARKER, MARKER_AT, PARTY_SLOTS, SCRATCH_KAIOKEN_HIGH, party_record, record_high,
    side_record, split, wide,
};
use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, ReturnKind, RoutineFn};

/// `05:AB75`: the `LDA $0014,Y` after the optional restore of a previous boost (`05:AB60..AB72`). The code reaches it from the
/// branch at `05:AB63` (no boost in use) and by falling through the restore. Y is the side record the boost works on: the
/// caller at `05:AB04` takes it from the active side, and the add at `05:ABE3` writes the same record.
pub(super) const BOOST_START_KEY: u32 = 0x05_AB75;
/// `05:ABE3`: the `RTS` after the boost's add (`05:ABBF..ABE0`). Y is the side record the add wrote: the routine takes it again
/// at `05:ABC3`, after the multiplication, and it is the record of entry.
pub(super) const BOOST_END_KEY: u32 = 0x05_ABE3;
/// `05:9D3D`: the 16-bit store that puts back the low word of the pre-boost BP when the Kaioken stops by KI (`05:9D35..9D3D`).
/// The key is this store and not the instruction after it (`05:9D40`): that one is also the join of the paths that do not
/// restore, the branches at `05:9D1E` and `05:9D22`. Y is the side record (`05:9CFE` takes it from the active side).
pub(super) const KI_RESTORE_KEY: u32 = 0x05_9D3D;
/// `01:9FE0`: the instruction after the byte store that puts back the pre-boost BP into the party record at `$7E:0B00 + Y`, when
/// the Kaioken stops at the second answer of the prompt (`01:9FD2..9FDD`). Y is the offset `01:9FBC` loads from `$0C20`.
pub(super) const STOP_RESTORE_KEY: u32 = 0x01_9FE0;

/// `$7E:01EB..$01ED`: the pre-boost low 24 bits of the side record (3 bytes, little endian).
const PRE_BOOST_LOW: u32 = 0x7E_01EB;
/// `$7E:01EF`: the multiplier of the boost in use, zero when none is.
const MULTIPLIER: u32 = 0x7E_01EF;
/// The pre-boost high part: the block's [`SCRATCH_KAIOKEN_HIGH`] (2 bytes, little endian).
const PRE_BOOST_HIGH: u32 = BLOCK + SCRATCH_KAIOKEN_HIGH as u32;
/// The largest BP the 40-bit record holds.
const MAX_BP: u64 = (1 << 40) - 1;

/// Whether the block's marker is in place: a neutral block declines every hook.
fn live(ctx: &mut Ctx<'_>) -> bool {
    MARKER.iter().enumerate().all(|(k, &byte)| ctx.read8(MARKER_AT + k as u32) == byte)
}

/// The bus address of side record A or B, from its base `y` (`$1400` or `$1440`).
fn side_at(y: u16) -> Option<u32> {
    match y {
        0x1400 => Some(side_record(false)),
        0x1440 => Some(side_record(true)),
        _ => None,
    }
}

/// The bus address of the high part of the record at the bus address `record`, when the record has one.
fn high_at(record: u32) -> Option<u32> {
    record_high(record).map(|offset| BLOCK + offset as u32)
}

/// The bus address of the high part of the party record at offset `y` from `$7E:0B00`, when `y` is a party record's offset.
fn party_high(y: u16) -> Option<u32> {
    (u32::from(y) < 0x20 * PARTY_SLOTS).then_some(party_record(0) + u32::from(y)).and_then(high_at)
}

/// Reads the 24 bits at the bus address `at`, low byte first.
fn read24(ctx: &mut Ctx<'_>, at: u32) -> u32 {
    u32::from(ctx.read16(at)) | (u32::from(ctx.read8(at + 2)) << 16)
}

/// Writes the low 24 bits of `value` at the bus address `at`, low byte first.
fn write24(ctx: &mut Ctx<'_>, at: u32, value: u32) {
    let [low, middle, high, _] = value.to_le_bytes();
    ctx.write8(at, low);
    ctx.write8(at + 1, middle);
    ctx.write8(at + 2, high);
}

/// `05:AB75`: the side's high part. A boost in use (`$01EF` not zero) has just had its low 24 bits put back to the pre-boost
/// value, so its high part goes back to the pre-boost one. Otherwise the side's high part is the pre-boost one, which the
/// boost saves with its low part (`05:ABAB..ABB9`), so the scratch takes it.
fn boost_start_wrapper(ctx: &mut Ctx<'_>) {
    if live(ctx)
        && let Some(record) = side_at(ctx.y())
        && let Some(high) = high_at(record)
    {
        if ctx.read8(MULTIPLIER) != 0 {
            let pre_boost = ctx.read16(PRE_BOOST_HIGH);
            ctx.write16(high, pre_boost);
        } else {
            let current = ctx.read16(high);
            ctx.write16(PRE_BOOST_HIGH, current);
        }
    }
    ctx.resume_at(BOOST_START_KEY);
}

/// `05:ABE3`: the boost's add on the whole BP. The original adds the halved product of the pre-boost low 24 bits to the
/// record's low 24 bits and drops the carry (`05:ABBF..ABE0`). Here the pre-boost BP is the 40 bits of `$01EB..$01ED` and the
/// scratch, the add is [`boosted`], and the record takes the low 24 bits of the sum and the block the high 16.
fn boost_end_wrapper(ctx: &mut Ctx<'_>) {
    let multiplier = ctx.read8(MULTIPLIER);
    if multiplier != 0
        && live(ctx)
        && let Some(record) = side_at(ctx.y())
        && let Some(high) = high_at(record)
    {
        let old = wide(read24(ctx, PRE_BOOST_LOW), ctx.read16(PRE_BOOST_HIGH));
        let (low, top) = split(boosted(old, multiplier));
        write24(ctx, record + 0x07, low);
        ctx.write16(high, top);
    }
    ctx.resume_at(BOOST_END_KEY);
}

/// `05:9D3D`: the store of the restore's low word into the side record Y names (`05:9D35..9D3D`). The high part goes back just
/// before it: the store does not read the high part, and only the restore path reaches this key.
fn ki_restore_wrapper(ctx: &mut Ctx<'_>) {
    if live(ctx)
        && let Some(record) = side_at(ctx.y())
        && let Some(high) = high_at(record)
    {
        let pre_boost = ctx.read16(PRE_BOOST_HIGH);
        ctx.write16(high, pre_boost);
    }
    ctx.resume_at(KI_RESTORE_KEY);
}

/// `01:9FE0`: the pre-boost high part goes back into the party record at the offset Y names, once the restore's byte store has
/// put back its low 24 bits (`01:9FD5..9FDD`). The routine has no other path into this instruction.
fn stop_restore_wrapper(ctx: &mut Ctx<'_>) {
    if live(ctx)
        && let Some(high) = party_high(ctx.y())
    {
        let pre_boost = ctx.read16(PRE_BOOST_HIGH);
        ctx.write16(high, pre_boost);
    }
    ctx.resume_at(STOP_RESTORE_KEY);
}

/// The BP after a boost with multiplier `m`: `old` plus the halved product of `old` and `m`, saturated at 40 bits. On a BP
/// without a high part the low 24 bits of the sum are the original's, and the carry is kept in the high part. `m` of zero is
/// the identity.
#[must_use]
fn boosted(old: u64, m: u8) -> u64 {
    old.saturating_add(old.saturating_mul(u64::from(m)) >> 1).min(MAX_BP)
}

/// The hook entries of the Kaioken boost and its restores, all `Patch`: they cost nothing and never call the original.
pub(super) fn hook_entries() -> Vec<HookEntry> {
    let patch = |name: &str, key: u32, func: RoutineFn| HookEntry {
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
        patch("mod_kaioken_boost_start", BOOST_START_KEY, boost_start_wrapper),
        patch("mod_kaioken_boost_end", BOOST_END_KEY, boost_end_wrapper),
        patch("mod_kaioken_ki_restore", KI_RESTORE_KEY, ki_restore_wrapper),
        patch("mod_kaioken_stop_restore", STOP_RESTORE_KEY, stop_restore_wrapper),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boost_adds_the_halved_product_and_saturates_at_40_bits() {
        assert_eq!(boosted(100, 1), 150, "100 plus half of 100 times 1");
        assert_eq!(boosted(100, 10), 600, "100 plus half of 100 times 10");
        assert_eq!(boosted(MAX_BP, 20), MAX_BP, "saturated at 40 bits");
        assert_eq!(boosted(u64::MAX, 255), MAX_BP, "a value past 40 bits saturates too");
    }

    #[test]
    fn a_wide_bp_keeps_the_carry_that_the_24_bit_boost_loses() {
        let old = (1_u64 << 24) + 4;
        let new = boosted(old, 10);
        assert_eq!(new, old + old * 10 / 2, "the whole 40-bit BP is multiplied");
        assert_eq!(new, 100_663_320);
        // On the low 24 bits alone, 4 plus 20 is 24 and its carry is lost: the record keeps 24, the block keeps the carry, 6.
        assert_eq!(
            split(new),
            (24, 6),
            "the low 24 bits agree with the 24-bit boost, the carry is in the high part"
        );
    }

    #[test]
    fn a_zero_multiplier_is_the_identity() {
        for old in [0, 100, (1 << 24) + 4, MAX_BP] {
            assert_eq!(boosted(old, 0), old);
        }
    }

    #[test]
    fn the_hooks_are_at_the_four_keys_of_the_listing_and_no_key_is_a_cheat_key() {
        assert_eq!(BOOST_START_KEY, 0x05_AB75);
        assert_eq!(BOOST_END_KEY, 0x05_ABE3);
        assert_eq!(KI_RESTORE_KEY, 0x05_9D3D);
        assert_eq!(STOP_RESTORE_KEY, 0x01_9FE0);
        let cheats = crate::cheats::entry_points();
        let redirectors = crate::mods::entry_points();
        for entry in hook_entries() {
            assert!(!cheats.contains(&entry.key), "{} is a cheat's key", entry.name);
            assert_eq!(
                redirectors.iter().filter(|&&key| key == entry.key).count(),
                1,
                "{} is registered once",
                entry.name
            );
        }
    }

    #[test]
    fn the_entries_are_four_patches_that_cost_nothing_in_their_order() {
        let entries = hook_entries();
        let keys: Vec<u32> = entries.iter().map(|entry| entry.key).collect();
        assert_eq!(keys, [BOOST_START_KEY, BOOST_END_KEY, KI_RESTORE_KEY, STOP_RESTORE_KEY]);
        for entry in &entries {
            assert_eq!(entry.mode, HookMode::Patch, "{}", entry.name);
            assert_eq!(entry.cycles, Some(0), "{}", entry.name);
        }
    }

    #[test]
    fn a_party_offset_names_its_record_and_only_the_nine_records_have_one() {
        assert_eq!(party_high(0x00), Some(BLOCK));
        assert_eq!(party_high(0x20), Some(BLOCK + 0x02));
        assert_eq!(party_high(0x100), Some(BLOCK + 0x10), "the ninth record starts past one byte");
        assert_eq!(party_high(0x120), None, "there is no tenth party record");
        assert_eq!(party_high(0x10), None, "an offset inside a record names no record");
        assert_eq!(party_high(0x0E00), None, "an enemy record is not a party record");
    }

    #[test]
    fn the_side_bases_name_their_records_and_nothing_else_does() {
        assert_eq!(side_at(0x1400), Some(side_record(false)));
        assert_eq!(side_at(0x1440), Some(side_record(true)));
        assert_eq!(side_at(0x1420), None);
        assert_eq!(high_at(side_record(false)), Some(BLOCK + 0x1C));
        assert_eq!(high_at(side_record(true)), Some(BLOCK + 0x1E));
    }
}
