//! The level cap of `free_progression` (`level_cap`, 48 to 999; `docs/mods/free-progression.md`, "The level cap"): the party's
//! levels, EXP, BP, HP and KI go on past the game's own table, on the curves of [`ssd_menus::levels`].
//!
//! **Storage.** A record keeps its level in one byte (`+02`), which every routine of the game reads. A level above 99 is kept
//! as a *wide level* in the mod state block (`state::wide_level_at`, 16 bits a character, with its own marker), and the record's
//! byte then saturates at 99 (two digits: the original's screens read "99"). The wide level is the character's level only while
//! the byte agrees with it (`min(wide, 99) == byte`); a routine that wrote the byte by itself leaves the byte as the truth. The
//! EXP is 24 bits in the record (`+0F..+11`) and its bits 24 to 31 are in the state (`state::exp_high_at`): the EXP of level 999
//! is 63 million. The BP is the wide BP of `state` (24 bits in the record, 16 in the block).
//!
//! **Hooks** (all `Patch`, all decline without the pack parameter, and for the characters that have no level table):
//! - `03:841E`, the level-up check after a fight: replaced; it raises the character while its EXP reaches the next level's
//!   start, with the rows of the game up to the character's own cap (47 or 48) and the extension beyond;
//! - `03:851B`, the raise by a number of levels (floors, gifts, the Porunga card): replaced, with the original's rule
//!   (`target + 1`, at least one level);
//! - `05:8FDD`, after the 24-bit EXP add of a fight: the carry goes to the EXP's high byte;
//! - `03:867A`, the end of the Super Saiyan's builder (`03:85F0`): the BP, HP and KI the form is built with follow the level.

use super::progression::level_cap;
use super::state::{
    BLOCK, CHARACTERS, LEVELS_MARK, LEVELS_MARKER_AT, MARKER, MARKER_AT, PARTY_HIGH, PARTY_SLOTS,
    SCRATCH_END, exp_high_at, level_agreeing, party_record, wide_level_at,
};
use ssd_menus::levels::{Anchor, HP_KI_MAX, RECORD_LEVEL_MAX};
use ssd_menus::mods_state::LEVEL_BYTES;
use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, ReturnKind, RoutineFn};

/// `03:841E`: the level-up check, entered with `Y` the party record's offset.
pub(super) const LEVEL_UP_KEY: u32 = 0x03_841E;
/// `03:851B`: the raise by `$10` levels (the target rule below), entered with `X` the party record's offset.
pub(super) const RAISE_KEY: u32 = 0x03_851B;
/// `05:8FDD`: the `JSL $03841E` of the award loop, after the 24-bit EXP add; the carry is the add's.
pub(super) const EXP_CARRY_KEY: u32 = 0x05_8FDD;
/// `03:867A`: the `RTL` of `03:85F0`, the builder of Goku's Super Saiyan form.
pub(super) const SUPER_SAIYAN_KEY: u32 = 0x03_867A;

/// `03:850E`: the game's cap of each character (47 or 48 for the eight that have a table).
const GAME_CAPS: u32 = 0x03_850E;
/// `02:C13B`: a word per character, the pointer of its level table in bank 2; the table is 48 rows of 11 bytes.
const LEVEL_POINTERS: u32 = 0x02_C13B;
const ROW_LEN: u32 = 11;
/// The characters with a level table of 48 rows: Goku to Vegeta.
const WITH_TABLE: u8 = 8;

/// The offsets of a party record.
const FLAGS: u32 = 0x00;
const LEVEL: u32 = 0x02;
const HP: u32 = 0x03;
const KI: u32 = 0x05;
const BP: u32 = 0x07;
const MAX_HP: u32 = 0x0A;
const MAX_KI: u32 = 0x0C;
const BYTE_0E: u32 = 0x0E;
const EXP: u32 = 0x0F;
const CHARACTER: u32 = 0x1C;
/// Bit 6 of the flags: a member of the party that the level-up check serves.
const IN_PARTY: u8 = 0x40;
/// `$1F`: the award loop's "a level was gained" byte (bit 7).
const GAINED: u16 = 0x1F;
/// `$10`: the number of levels the raise is given.
const RAISE_PARAMETER: u16 = 0x10;

/// The Super Saiyan's own level in the original: the form is built for a Goku of 47 (`03:8683`), with the BP of the row of 48.
const SUPER_SAIYAN_LEVEL: u16 = 47;
/// The BP, HP and KI of the form's row (the numbers of the row of level 48 are assigned, not added).
const SUPER_SAIYAN_ID: u8 = 0;

/// One row of a character's table at a level: the BP the level gives, the EXP it begins at, its maximum HP, the KI it gives and
/// byte `$0E`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub(super) bp_gain: u64,
    pub(super) exp_start: u64,
    pub(super) max_hp: u16,
    pub(super) ki_gain: u32,
    pub(super) byte_0e: u8,
}

/// The game's row of `id` at `level` (1 to 48), read from the ROM.
fn game_row(ctx: &Ctx<'_>, id: u8, level: u16) -> Row {
    let pointer = ctx.peek16(LEVEL_POINTERS + 2 * u32::from(id));
    let at = 0x02_0000 | u32::from(pointer);
    let row = at + ROW_LEN * (u32::from(level) - 1);
    let u24 = |at: u32| u64::from(ctx.peek16(at)) | u64::from(ctx.peek8(at + 2)) << 16;
    Row {
        bp_gain: u24(row),
        exp_start: u24(row + 3),
        max_hp: ctx.peek16(row + 6),
        ki_gain: u32::from(ctx.peek16(row + 8)),
        byte_0e: ctx.peek8(row + 10),
    }
}

/// The curve of `id` past its game cap: the totals of its rows up to the cap, and the values of the cap's row.
fn anchor(ctx: &Ctx<'_>, id: u8) -> Anchor {
    let cap = u16::from(ctx.peek8(GAME_CAPS + u32::from(id)));
    let (mut bp, mut ki) = (0_u64, 0_u64);
    for level in 1..=cap {
        let row = game_row(ctx, id, level);
        bp += row.bp_gain;
        ki += u64::from(row.ki_gain);
    }
    let last = game_row(ctx, id, cap);
    Anchor {
        level: cap,
        bp,
        ki,
        max_hp: u64::from(last.max_hp),
        exp: last.exp_start,
        byte_0e: u64::from(last.byte_0e),
    }
}

/// The row of `id` at `level`: the game's own up to its cap, the extension beyond it.
pub(super) fn row_at(ctx: &Ctx<'_>, id: u8, level: u16) -> Row {
    let cap = u16::from(ctx.peek8(GAME_CAPS + u32::from(id)));
    if level <= cap {
        return game_row(ctx, id, level);
    }
    let anchor = anchor(ctx, id);
    let point = anchor.at(level);
    Row {
        bp_gain: anchor.bp_gain(level),
        exp_start: point.exp,
        max_hp: point.max_hp,
        ki_gain: anchor.ki_gain(level) as u32,
        byte_0e: point.byte_0e,
    }
}

/// The 24 bits at `at`.
fn peek24(ctx: &Ctx<'_>, at: u32) -> u32 {
    u32::from(ctx.peek16(at)) | u32::from(ctx.peek8(at + 2)) << 16
}

fn write24(ctx: &mut Ctx<'_>, at: u32, value: u32) {
    ctx.write16(at, value as u16);
    ctx.write8(at + 2, (value >> 16) as u8);
}

/// Whether the wide levels' marker is in place.
fn levels_live(ctx: &Ctx<'_>) -> bool {
    LEVELS_MARK.iter().enumerate().all(|(k, &byte)| ctx.peek8(LEVELS_MARKER_AT + k as u32) == byte)
}

/// Puts the wide levels in place (the marker, and every byte zero) when they are not.
fn make_levels_live(ctx: &mut Ctx<'_>) {
    if !levels_live(ctx) {
        for k in 0..LEVEL_BYTES as u32 {
            ctx.write8(wide_level_at(0) + k, 0);
        }
        for (k, &byte) in LEVELS_MARK.iter().enumerate() {
            ctx.write8(LEVELS_MARKER_AT + k as u32, byte);
        }
    }
}

/// The level of the party record `record` of character `id`: the wide level while the byte agrees with it, else the byte.
#[must_use]
pub(super) fn effective_level(ctx: &Ctx<'_>, record: u32) -> u16 {
    let byte = ctx.peek8(record + LEVEL);
    let id = ctx.peek8(record + CHARACTER);
    if id < CHARACTERS && levels_live(ctx) {
        return level_agreeing(ctx.peek16(wide_level_at(id)), byte);
    }
    u16::from(byte)
}

/// Sets the level of the record: the byte saturates at 99 and the wide level is kept when the level is above it.
fn set_level(ctx: &mut Ctx<'_>, record: u32, id: u8, level: u16) {
    ctx.write8(record + LEVEL, level.min(u16::from(RECORD_LEVEL_MAX)) as u8);
    if id < CHARACTERS && (level > u16::from(RECORD_LEVEL_MAX) || levels_live(ctx)) {
        make_levels_live(ctx);
        ctx.write16(wide_level_at(id), if level > u16::from(RECORD_LEVEL_MAX) { level } else { 0 });
    }
}

/// The EXP of the record: 24 bits and the high byte of the state.
fn exp_of(ctx: &Ctx<'_>, record: u32, id: u8) -> u64 {
    let high = if id < CHARACTERS && levels_live(ctx) { ctx.peek8(exp_high_at(id)) } else { 0 };
    u64::from(peek24(ctx, record + EXP)) | u64::from(high) << 24
}

fn set_exp(ctx: &mut Ctx<'_>, record: u32, id: u8, exp: u64) {
    write24(ctx, record + EXP, (exp & 0xFF_FFFF) as u32);
    let high = (exp >> 24).min(0xFF) as u8;
    if id < CHARACTERS && (high != 0 || levels_live(ctx)) {
        make_levels_live(ctx);
        ctx.write8(exp_high_at(id), high);
    }
}

/// The party slot of a record's bus address.
fn slot_of(record: u32) -> u32 {
    (record - party_record(0)) / 0x20
}

/// The 40-bit BP of the record.
fn bp_of(ctx: &Ctx<'_>, record: u32) -> u64 {
    let high = if block_live_ctx(ctx) {
        u64::from(ctx.peek16(BLOCK + (PARTY_HIGH as u32 + 2 * slot_of(record))))
    } else {
        0
    };
    u64::from(peek24(ctx, record + BP)) | high << 24
}

fn block_live_ctx(ctx: &Ctx<'_>) -> bool {
    MARKER.iter().enumerate().all(|(k, &byte)| ctx.peek8(MARKER_AT + k as u32) == byte)
}

/// Writes a 40-bit BP: the low 24 bits in the record and the high 16 in the party slot's part of the block, which is made live
/// when a part needs it (the used bytes and the scratch zeroed, then the marker, as the enemy scaling does).
fn set_bp(ctx: &mut Ctx<'_>, record: u32, bp: u64) {
    let (low, high) = super::state::split(bp);
    write24(ctx, record + BP, low);
    if high != 0 && !block_live_ctx(ctx) {
        for offset in 0..SCRATCH_END as u32 {
            ctx.write8(BLOCK + offset, 0);
        }
        for (k, &byte) in MARKER.iter().enumerate() {
            ctx.write8(MARKER_AT + k as u32, byte);
        }
    }
    if block_live_ctx(ctx) {
        ctx.write16(BLOCK + (PARTY_HIGH as u32 + 2 * slot_of(record)), high);
    }
}

/// The bus address of the party record at offset `offset`, for a character with a table (`None` for any other record).
fn record_of(ctx: &Ctx<'_>, offset: u16) -> Option<(u32, u8)> {
    if u32::from(offset) >= 0x20 * PARTY_SLOTS || !offset.is_multiple_of(0x20) {
        return None;
    }
    let record = party_record(0) + u32::from(offset);
    let id = ctx.peek8(record + CHARACTER);
    (id < WITH_TABLE).then_some((record, id))
}

/// The gains of one level on the record: BP added, maximum HP set, KI added to the maximum, byte `$0E` set.
fn apply_row(ctx: &mut Ctx<'_>, record: u32, row: &Row) {
    let bp = bp_of(ctx, record) + row.bp_gain;
    set_bp(ctx, record, bp);
    ctx.write16(record + MAX_HP, row.max_hp);
    let max_ki = (u64::from(ctx.peek16(record + MAX_KI)) + u64::from(row.ki_gain)).min(HP_KI_MAX);
    ctx.write16(record + MAX_KI, max_ki as u16);
    ctx.write8(record + BYTE_0E, row.byte_0e);
}

/// `03:841E`: the level-up check of one party member. With the cap, for a character that has a table: the original's rules (a
/// member in the party, not at its cap) and its loop (the next level's start reached: the level, the row's BP, HP, KI and byte
/// `$0E`, HP and KI full, bit 7 of `$1F`), with the rows of [`row_at`]. Anything else is the original's.
fn level_up_check(ctx: &mut Ctx<'_>) {
    let Some(cap) = level_cap(ctx) else {
        ctx.resume_at(LEVEL_UP_KEY);
        return;
    };
    let y = ctx.y();
    let Some((record, id)) = record_of(ctx, y) else {
        ctx.resume_at(LEVEL_UP_KEY);
        return;
    };
    let mut level = effective_level(ctx, record);
    if level >= cap {
        ctx.set_cost(0);
        return;
    }
    ctx.write_dp8(GAINED, 0);
    if ctx.peek8(record + FLAGS) & IN_PARTY != 0 {
        let exp = exp_of(ctx, record, id);
        while level < cap && exp >= row_at(ctx, id, level + 1).exp_start {
            level += 1;
            set_level(ctx, record, id, level);
            apply_row(ctx, record, &row_at(ctx, id, level));
            let (max_hp, max_ki) = (ctx.peek16(record + MAX_HP), ctx.peek16(record + MAX_KI));
            ctx.write16(record + HP, max_hp);
            ctx.write16(record + KI, max_ki);
            ctx.write_dp8(GAINED, 0x80);
        }
    }
    ctx.set_x(y);
    ctx.set_cost(0);
}

/// The level a raise reaches: the original sets `$10 = byte + delta` in 8 bits and raises to `$10 + 1` (at most the cap), and
/// at least one level. A level above 99 has its byte at 99, which keeps the rule: a floor's target (at most 99) is a no-op that
/// gives one level, a gift of `delta` levels gives `delta + 1`.
#[must_use]
fn raised_level(byte: u8, wide: u16, delta: u8, cap: u16) -> u16 {
    let target = u16::from(byte.wrapping_add(delta)) + 1;
    let gained = if target > u16::from(byte) { target - u16::from(byte) } else { 1 };
    (wide + gained).min(cap)
}

/// `03:851B`: the raise of a party member by `$10` levels (`X` its record's offset). Replaced with the cap, for a character that
/// has a table: the rows of every level reached add their BP and KI, the EXP is set to the start of the level reached, and the
/// maximum HP and byte `$0E` are the last row's.
fn raise(ctx: &mut Ctx<'_>) {
    let Some(cap) = level_cap(ctx) else {
        ctx.resume_at(RAISE_KEY);
        return;
    };
    let x = ctx.x();
    let Some((record, id)) = record_of(ctx, x) else {
        ctx.resume_at(RAISE_KEY);
        return;
    };
    let before = effective_level(ctx, record);
    if before < cap {
        let delta = ctx.peek_dp8(RAISE_PARAMETER);
        let after = raised_level(ctx.peek8(record + LEVEL), before, delta, cap);
        for level in before + 1..=after {
            let row = row_at(ctx, id, level);
            let bp = bp_of(ctx, record) + row.bp_gain;
            set_bp(ctx, record, bp);
            let max_ki =
                (u64::from(ctx.peek16(record + MAX_KI)) + u64::from(row.ki_gain)).min(HP_KI_MAX);
            ctx.write16(record + MAX_KI, max_ki as u16);
            ctx.write16(record + KI, max_ki as u16);
        }
        set_level(ctx, record, id, after);
        let last = row_at(ctx, id, after);
        set_exp(ctx, record, id, last.exp_start);
        ctx.write16(record + MAX_HP, last.max_hp);
        ctx.write16(record + HP, last.max_hp);
        ctx.write8(record + BYTE_0E, last.byte_0e);
    }
    ctx.set_x(x);
    ctx.set_cost(0);
}

/// `05:8FDD`: a member's 24-bit EXP has just been added to; the carry of the add goes to the EXP's high byte. Y is the record's
/// offset.
fn exp_carry(ctx: &mut Ctx<'_>) {
    if level_cap(ctx).is_some()
        && ctx.c()
        && let Some((_, id)) = record_of(ctx, ctx.y())
    {
        make_levels_live(ctx);
        let high = ctx.peek8(exp_high_at(id)).saturating_add(1);
        ctx.write8(exp_high_at(id), high);
    }
    ctx.resume_at(EXP_CARRY_KEY);
}

/// The factor of a level over the form's own level, `((level + 5) / 52)^power`, applied to `value` (rounded, halves up).
#[must_use]
pub(super) fn form_scaled(value: u64, level: u16, power: u32) -> u64 {
    let u = u128::from(level.max(SUPER_SAIYAN_LEVEL)) + 5;
    let u0 = u128::from(SUPER_SAIYAN_LEVEL) + 5;
    let (num, den) = (u.pow(power), u0.pow(power));
    u64::try_from((u128::from(value) * num + den / 2) / den).unwrap_or(u64::MAX)
}

/// `03:867A`: Goku's Super Saiyan form has been built (BP, HP and KI assigned from the row of level 48: 4 999 999 BP). With the
/// cap, the form follows Goku's level `L` instead of the fixed row: BP times `((L + 5) / 52)^3`, HP and KI times
/// `(L + 5) / 52` (at most four digits), and the record's level byte is the one it had (not 48). A Goku of 47 or less gets the
/// row's own numbers.
fn super_saiyan(ctx: &mut Ctx<'_>) {
    if level_cap(ctx).is_some() {
        let record = party_record(0) + u32::from(ctx.y());
        if u32::from(ctx.y()) < 0x20 * PARTY_SLOTS
            && ctx.y().is_multiple_of(0x20)
            && ctx.peek8(record + CHARACTER) == SUPER_SAIYAN_ID
        {
            let wide =
                if levels_live(ctx) { ctx.peek16(wide_level_at(SUPER_SAIYAN_ID)) } else { 0 };
            if wide > u16::from(RECORD_LEVEL_MAX) {
                let low = u64::from(peek24(ctx, record + BP));
                set_bp(ctx, record, form_scaled(low, wide, 3));
                let hp = form_scaled(u64::from(ctx.peek16(record + MAX_HP)), wide, 1).min(HP_KI_MAX)
                    as u16;
                let ki = form_scaled(u64::from(ctx.peek16(record + MAX_KI)), wide, 1).min(HP_KI_MAX)
                    as u16;
                for at in [MAX_HP, HP] {
                    ctx.write16(record + at, hp);
                }
                for at in [MAX_KI, KI] {
                    ctx.write16(record + at, ki);
                }
                ctx.write8(record + LEVEL, RECORD_LEVEL_MAX);
            }
        }
    }
    ctx.resume_at(SUPER_SAIYAN_KEY);
}

/// One hook, a `Patch` that costs nothing. `m8` is the width of the accumulator at the key.
fn patch(name: &str, key: u32, func: RoutineFn) -> HookEntry {
    HookEntry {
        name: name.to_owned(),
        key,
        mode: HookMode::Patch,
        action: HookAction::Routine { func, ret: ReturnKind::Rtl },
        entry_m8: Some(true),
        entry_x8: Some(false),
        flags_mask: 0,
        verify_sample: 1,
        cycles: Some(0),
        bound: None,
        budget_frames: None,
    }
}

/// The hooks of the level cap.
pub(super) fn hooks() -> Vec<HookEntry> {
    vec![
        patch("mod_fp_level_up", LEVEL_UP_KEY, level_up_check),
        patch("mod_fp_level_raise", RAISE_KEY, raise),
        patch("mod_fp_exp_carry", EXP_CARRY_KEY, exp_carry),
        patch("mod_fp_super_saiyan_form", SUPER_SAIYAN_KEY, super_saiyan),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raise_follows_the_originals_rule_and_the_wide_level_keeps_it() {
        // The measured floors: target 8 on levels 1, 4, 7, 8, 9, 12 and 30 gives 9, 9, 9, 9, 10, 13 and 31 (the delta is the
        // target minus the level, in 8 bits; the raise is to the target plus one and at least one level).
        for (level, expected) in
            [(1_u8, 9_u16), (4, 9), (7, 9), (8, 9), (9, 10), (12, 13), (30, 31)]
        {
            let delta = 8_u8.wrapping_sub(level);
            assert_eq!(
                raised_level(level, u16::from(level), delta, 999),
                expected,
                "level {level}"
            );
        }
        // A gift of 3 levels is delta 3: four levels.
        assert_eq!(raised_level(20, 20, 3, 999), 24);
        // At the top of the byte the floor is one level and the gift its full size.
        assert_eq!(raised_level(99, 300, 8_u8.wrapping_sub(99), 999), 301, "a floor at level 300");
        assert_eq!(raised_level(99, 300, 20, 999), 321, "a gift of 20 at level 300");
        assert_eq!(raised_level(99, 998, 20, 999), 999, "the cap");
    }

    #[test]
    fn the_super_saiyan_form_follows_the_level_and_is_the_rows_own_at_47_or_less() {
        assert_eq!(form_scaled(4_999_999, 47, 3), 4_999_999);
        assert_eq!(form_scaled(4_999_999, 20, 3), 4_999_999, "a lower level is raised to 47");
        let at_300 = form_scaled(4_999_999, 300, 3);
        // (305 / 52)^3 = 201.8 times 4 999 999.
        assert!((1_005_000_000..1_012_000_000).contains(&at_300), "{at_300}");
        assert!(form_scaled(4_999_999, 999, 3) < 1 << 40, "fits 40 bits");
    }

    #[test]
    fn the_hooks_are_at_their_addresses_and_none_is_another_hooks_key() {
        let entries = hooks();
        let keys: Vec<u32> = entries.iter().map(|entry| entry.key).collect();
        assert_eq!(keys, [0x03_841E, 0x03_851B, 0x05_8FDD, 0x03_867A]);
        let cheats = crate::cheats::entry_points();
        for entry in &entries {
            assert!(!cheats.contains(&entry.key), "{} is a cheat's key", entry.name);
            assert_eq!(entry.mode, HookMode::Patch);
            assert!(entry.name.starts_with("mod_fp_"));
        }
    }
}
