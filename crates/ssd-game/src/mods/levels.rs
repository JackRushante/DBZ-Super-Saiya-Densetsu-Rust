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
//! - `03:85F0`, the builder of a form (entry), for Goku's Super Saiyan and Piccolo's fusion with Dende: the level the character had
//!   is kept (the builder sets 48) and the BP, HP and KI the form is built with follow it, never below what he had;
//! - `03:C64A`, the level routine of the story's gifts (`$164C` the character, `$164D` the levels): the gifts of Nail's and Dende's
//!   fusions and of Guru's unlocking are worth the same in BP at a high level as at the level the story gives them, and the raise
//!   Goku gets after the medical pod follows the party (see [`gift_context`]); every other caller (the floors) is the original's;
//! - `01:E3FC`, the commit of Porunga's level wish on the chosen member (the Item effect): a member at the game's cap (47, 48) gets a
//!   level up to the pack's cap, on the extended rows (and the party-wide wish of `party_boosts` the same for the others);
//! - `03:C845`, the end of the load of the last fight (effect `$3C`): Goku, who joins from the roster, is raised to the party's level.
//! - `05:8E3F`, the bonus of a won gravity trial (+2 levels): the level and EXP of the next two levels on the extended rows;
//! - `03:CD87`, the EXP the gravity trials' last stage pays: its carry past 24 bits goes to the EXP's high byte;
//! - `05:C8D5`, the Porunga card: its own cap (47, 48 for Gohan) gives way to the level cap, and a level comes with the EXP of the
//!   next level on the extended rows and the level-up check.

use super::progression::level_cap;
use super::scaling::{Fighter, party_sixths};
use super::state::{
    BLOCK, CHARACTERS, LEVELS_MARK, LEVELS_MARKER_AT, MARKER, MARKER_AT, PARTY_HIGH, PARTY_SLOTS,
    SCRATCH_END, exp_high_at, level_agreeing, party_record, snapshot_at, snapshot_high,
    wide_level_at,
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
/// `03:85F0`: the builder of a form (entry, `A` the character id: the record is the party's of that character). It sets the unit
/// from a table, the level (48 for Piccolo, the character's cap plus one for Goku) and BP, HP and KI from the character's row 48.
pub(super) const FORM_KEY: u32 = 0x03_85F0;
/// `03:C64A`: the level routine of the story (`$164C` the character id, `$164D` the levels to add), the scene effect `$38`.
pub(super) const GIFT_KEY: u32 = 0x03_C64A;
/// `01:E3FC`: the commit of Porunga's level wish on the chosen member (step 7 of the Item effect `01:E2A5`, after the A button).
pub(super) const WISH_KEY: u32 = 0x01_E3FC;
/// `03:C845`: the end of the party's load in effect `$3C` (Goku and the party list are in their records, the enemy is built).
pub(super) const ARRIVAL_KEY: u32 = 0x03_C845;

/// `05:8E3F`: the win of a gravity trial of the spaceship (a fight that ends with `$01D6` bit 7 set): the original sets the first
/// record's EXP to the start of the level two above and raises it (`05:C93A` twice, which reads the table row of each level
/// reached, then a level-up check after lowering the byte by two).
pub(super) const TRIAL_WIN_KEY: u32 = 0x05_8E3F;
/// `05:8E55`: where the trial's win goes on after the level-up check (`LDA $0B02`, the level shown in the message).
const TRIAL_WIN_DONE: u32 = 0x05_8E55;
/// `03:CD87`: the `LDY #0` after the 24-bit add of a trial's EXP (effect `$52`) to the first record, in front of its level-up check.
pub(super) const TRIAL_EXP_KEY: u32 = 0x03_CD87;
/// `05:C8D5`: the Porunga card, after the target is chosen and its cap is in `$00` (47, 48 for the category 9). The original refuses
/// a member whose level byte reaches `$00`, else raises it by `05:C93A` (the EXP of the next table row, then the byte plus one).
pub(super) const CARD_KEY: u32 = 0x05_C8D5;
/// `05:C8E9`: the card's `JSL $1C8044`, which turns the 24-bit number in `$00` (the new level) into the message's digits.
const CARD_MESSAGE: u32 = 0x05_C8E9;

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

/// The form's own level in the original: the form is built for a character of 47, with the BP of the row of 48.
const FORM_BASE_LEVEL: u16 = 47;
/// The level the builder gives the form (`03:8617`, `03:8620`).
const FORM_LEVEL: u16 = 48;
/// The characters the builder `03:85F0` makes forms of: Goku's Super Saiyan (unit `$3B`) and Piccolo's fusion with Dende (`$3E`).
const SUPER_SAIYAN_ID: u8 = 0;
const PICCOLO_ID: u8 = 2;

/// `$164C`, `$164D`: the character and the levels of the level routine `03:C64A`.
const GIFT_CHARACTER: u32 = 0x7E_164C;
const GIFT_LEVELS: u32 = 0x7E_164D;
/// `$7E:01D8`: the id of the scene that runs.
const SCENE_ID: u32 = 0x7E_01D8;
/// What `03:C64A` returns to when the scene engine runs it as the effect `$38` (the `JSL` at `03:B1C6` that dispatches an effect),
/// when the effect `$5B` runs it for each member (the `JSL` at `03:CFE1`), and when the scene `$3F` runs it for Goku (`03:FA82`):
/// the 24-bit address of the last byte of the `JSL`, which is what it pushes.
const EFFECT_RETURN: u32 = 0x03_B1C9;
const GURU_RETURN: u32 = 0x03_CFE4;
const POD_RETURN: u32 = 0x03_FA85;
/// The scenes whose effect `$38` is a gift of levels that the pack scales: Piccolo's fusion with Nail (`$37`, +5) and with Dende
/// (`$3C`, +4 or +2, then +1), and the level each is given at in the original, where the story expects the character to be (Piccolo
/// is at about 35 when he meets Nail, and about 41 after).
const NAIL_SCENE: u8 = 0x37;
const DENDE_SCENE: u8 = 0x3C;
const NAIL_LEVEL: u16 = 35;
const DENDE_LEVEL: u16 = 41;
/// Guru's unlocking of the party's potential (effect `$5B`, +2 for everyone) is given when the party is at about level 35.
const GURU_LEVEL: u16 = 35;
/// The roster record of Goku (the image of the save at `$7E:2800`), where the last fight's load takes him from.
const ROSTER_GOKU: u32 = 0x7E_2800;
/// `$7E:0EA3`: the encounter group; `$56` is the last fight, whose enemy is built before Goku joins.
const GROUP_INDEX: u32 = 0x7E_0EA3;
const LAST_FIGHT: u8 = 0x56;
/// Goku arrives at 11/10 of the mean of the party's three highest levels (owner's decision).
const ARRIVAL_NUM: u64 = 11;
const ARRIVAL_DEN: u64 = 10;

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

/// Makes the block live when it is not: the used bytes and the scratch zeroed, then the marker, as the enemy scaling does.
fn make_block_live(ctx: &mut Ctx<'_>) {
    if !block_live_ctx(ctx) {
        for offset in 0..SCRATCH_END as u32 {
            ctx.write8(BLOCK + offset, 0);
        }
        for (k, &byte) in MARKER.iter().enumerate() {
            ctx.write8(MARKER_AT + k as u32, byte);
        }
    }
}

/// Writes a 40-bit BP: the low 24 bits in the record and the high 16 in the party slot's part of the block, which is made live
/// when a part needs it.
fn set_bp(ctx: &mut Ctx<'_>, record: u32, bp: u64) {
    let (low, high) = super::state::split(bp);
    write24(ctx, record + BP, low);
    if high != 0 {
        make_block_live(ctx);
    }
    if block_live_ctx(ctx) {
        ctx.write16(BLOCK + (PARTY_HIGH as u32 + 2 * slot_of(record)), high);
    }
}

/// The 40-bit BP of the party record at the bus address `record`, and its writer (for the objects of the Item screen).
#[must_use]
pub(super) fn wide_bp(ctx: &Ctx<'_>, record: u32) -> u64 {
    bp_of(ctx, record)
}

pub(super) fn set_wide_bp(ctx: &mut Ctx<'_>, record: u32, bp: u64) {
    set_bp(ctx, record, bp);
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

/// The loop of the level-up check on `record` of character `id`: while the EXP reaches the next level's start and the level is
/// below `cap`, the level goes up with the row's BP, HP, KI and byte `$0E`, HP and KI refilled, bit 7 of `$1F` set.
fn level_up_record(ctx: &mut Ctx<'_>, record: u32, id: u8, cap: u16) {
    let mut level = effective_level(ctx, record);
    if level >= cap {
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
    level_up_record(ctx, record, id, cap);
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

/// Raises the party record of character `id` from level `before` to `after` (above `before`, at most the level cap): the rows of
/// every level reached add their BP and KI, the EXP is set to the start of the level reached, and the maximum HP and byte `$0E`
/// are the last row's. HP and KI are filled.
fn raise_levels(ctx: &mut Ctx<'_>, record: u32, id: u8, before: u16, after: u16) {
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

/// `03:851B`: the raise of a party member by `$10` levels (`X` its record's offset). Replaced with the cap, for a character that
/// has a table: see [`raise_levels`].
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
        raise_levels(ctx, record, id, before, after);
    }
    ctx.set_x(x);
    ctx.set_cost(0);
}

/// The carry of a 24-bit EXP add to the record at `offset` goes to the EXP's high byte (the carry flag is the add's).
fn carry_to_high_byte(ctx: &mut Ctx<'_>, offset: u16) {
    if level_cap(ctx).is_some()
        && ctx.c()
        && let Some((_, id)) = record_of(ctx, offset)
    {
        make_levels_live(ctx);
        let high = ctx.peek8(exp_high_at(id)).saturating_add(1);
        ctx.write8(exp_high_at(id), high);
    }
}

/// `05:8FDD`: a member's 24-bit EXP has just been added to; the carry of the add goes to the EXP's high byte. Y is the record's
/// offset.
fn exp_carry(ctx: &mut Ctx<'_>) {
    carry_to_high_byte(ctx, ctx.y());
    ctx.resume_at(EXP_CARRY_KEY);
}

/// `03:CD87`: a trial's EXP (effect `$52`) has been added to the first record's 24 bits; its carry goes to the high byte.
fn trial_exp_carry(ctx: &mut Ctx<'_>) {
    carry_to_high_byte(ctx, 0);
    ctx.resume_at(TRIAL_EXP_KEY);
}

/// `05:8E3F`: a gravity trial is won. The original gives the first record the EXP that begins the level two above (reading the
/// table row of each level, which has none past the 48th: the EXP became whatever bytes follow the table and the level never went
/// on) and raises it by the level-up check. With the cap: the EXP of the level two above on the extended rows (at most the cap),
/// then the check; the level byte is left alone (the original lowers it by two after raising it by two). A first record that is
/// not a character with a table, and no cap, are the original's.
fn trial_win(ctx: &mut Ctx<'_>) {
    if let Some(cap) = level_cap(ctx)
        && let Some((record, id)) = record_of(ctx, 0)
    {
        let level = effective_level(ctx, record);
        if level < cap {
            let target = (level + 2).min(cap);
            set_exp(ctx, record, id, row_at(ctx, id, target).exp_start);
        }
        level_up_record(ctx, record, id, cap);
        ctx.set_x(0);
        ctx.set_y(0);
        ctx.resume_at(TRIAL_WIN_DONE);
        return;
    }
    ctx.resume_at(TRIAL_WIN_KEY);
}

/// `05:C8D5`: the Porunga card is used on the party record at `Y`, with the card's own cap in `$00`. With the cap, for a character
/// that has a table, from the card's own cap up (the levels the original refuses): a member below the level cap gets one level, on
/// the extended rows: the EXP is the start of the next level, then the level-up check runs (BP, HP, KI and byte `$0E` follow), and
/// the message shows the real level (the record's byte stops at 99); a member at the level cap is refused as before. Below the
/// card's cap, and for any other record, the original runs (a level and its EXP, no stats).
fn porunga_card(ctx: &mut Ctx<'_>) {
    if let Some(cap) = level_cap(ctx)
        && let Some((record, id)) = record_of(ctx, ctx.y())
    {
        let level = effective_level(ctx, record);
        if level >= u16::from(ctx.peek_dp8(0)) && level < cap {
            set_exp(ctx, record, id, row_at(ctx, id, level + 1).exp_start);
            level_up_record(ctx, record, id, cap);
            let shown = effective_level(ctx, record);
            ctx.write_dp8(0, shown as u8);
            ctx.write_dp8(1, (shown >> 8) as u8);
            ctx.write_dp8(2, 0);
            ctx.resume_at(CARD_MESSAGE);
            return;
        }
    }
    ctx.resume_at(CARD_KEY);
}

/// The factor of a level over the form's own level, `((level + 5) / 52)^power`, applied to `value` (rounded, halves up).
#[must_use]
pub(super) fn form_scaled(value: u64, level: u16, power: u32) -> u64 {
    let u = u128::from(level.max(FORM_BASE_LEVEL)) + 5;
    let u0 = u128::from(FORM_BASE_LEVEL) + 5;
    let (num, den) = (u.pow(power), u0.pow(power));
    u64::try_from((u128::from(value) * num + den / 2) / den).unwrap_or(u64::MAX)
}

/// The party record (the bus address) of character `id` that `03:F021` finds: the first record with both flag bits `$C0` set whose
/// character byte is `id`.
fn party_member(ctx: &Ctx<'_>, id: u8) -> Option<u32> {
    (0..PARTY_SLOTS)
        .map(party_record)
        .find(|&record| ctx.peek8(record + FLAGS) >= 0xC0 && ctx.peek8(record + CHARACTER) == id)
}

/// What the record had before the builder ran: the level, the BP and the maxima of HP and KI.
struct Before {
    level: u16,
    bp: u64,
    max_hp: u64,
    max_ki: u64,
}

/// The stats of a form built for a character of `level`: the builder's row numbers (`row`: BP of 24 bits, maximum HP, maximum KI)
/// scaled as the Super Saiyan's are ([`form_scaled`]: BP by `((level + 5) / 52)^3`, HP and KI by `(level + 5) / 52`, at most
/// 9 999), and never below what the character had (`before`: BP, maximum HP, maximum KI).
#[must_use]
fn form_stats(row: (u64, u64, u64), level: u16, before: (u64, u64, u64)) -> (u64, u64, u64) {
    let bp = form_scaled(row.0, level, 3).max(before.0);
    let hp = form_scaled(row.1, level, 1).min(HP_KI_MAX).max(before.1.min(HP_KI_MAX));
    let ki = form_scaled(row.2, level, 1).min(HP_KI_MAX).max(before.2.min(HP_KI_MAX));
    (bp, hp, ki)
}

/// `03:85F0`: the builder of a form, for Goku's Super Saiyan (`$3B`, `03:8683`) and Piccolo's fusion with Dende (`$3E`, effect
/// `$66`). The original sets the level to 48 and assigns BP, HP and KI from the character's row 48 (absolute numbers), whatever
/// the character had: with the level cap a character above 47 fell back to 48 and to the form's numbers (4 999 999 BP for Goku,
/// 1 900 000 for Piccolo), and the wide level and the BP's high part kept their old values. With the cap the original builder
/// runs, and then the character keeps his level (at least 48) with the stats of the form on the same curve as his own: BP times
/// `((L + 5) / 52)^3`, HP and KI times `(L + 5) / 52` (at most four digits), none of them below what he had. A level of 47 or
/// less gets the row's own numbers, as before (the form is built for 47).
fn form_builder(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    let id = ctx.a() as u8;
    let Some(cap) = level_cap(ctx) else {
        ctx.resume_at(FORM_KEY);
        return;
    };
    let Some(record) =
        (id == SUPER_SAIYAN_ID || id == PICCOLO_ID).then(|| party_member(ctx, id)).flatten()
    else {
        ctx.resume_at(FORM_KEY);
        return;
    };
    let before = Before {
        level: effective_level(ctx, record),
        bp: bp_of(ctx, record),
        max_hp: u64::from(ctx.peek16(record + MAX_HP)),
        max_ki: u64::from(ctx.peek16(record + MAX_KI)),
    };
    if ctx.call_original(FORM_KEY, ReturnKind::Rtl).is_err() {
        return;
    }
    // The form is built for 47 in the original (the row of 48): a character of 47 or less gets the row's numbers, a higher one the
    // curve's. The level is 48 at least, as the builder sets it.
    let level = before.level.min(cap);
    let row = (
        u64::from(peek24(ctx, record + BP)),
        u64::from(ctx.peek16(record + MAX_HP)),
        u64::from(ctx.peek16(record + MAX_KI)),
    );
    let (bp, hp, ki) = form_stats(row, level, (before.bp, before.max_hp, before.max_ki));
    set_bp(ctx, record, bp);
    for at in [MAX_HP, HP] {
        ctx.write16(record + at, hp as u16);
    }
    for at in [MAX_KI, KI] {
        ctx.write16(record + at, ki as u16);
    }
    set_level(ctx, record, id, level.max(FORM_LEVEL));
    ctx.set_cost(0);
}

/// The levels a story gift of `gained` levels is worth to a character at `level` when the story gives it at `reference`: the same
/// relative rise of his `u = level + 5` (BP goes with `u^3`), so `gained * (level + 5) / (reference + 5)`, rounded, never less
/// than the gift itself.
#[must_use]
pub(super) fn scaled_gain(gained: u16, level: u16, reference: u16) -> u16 {
    let (u, u0) = (u32::from(level) + 5, u32::from(reference) + 5);
    let scaled = (u32::from(gained) * u + u0 / 2) / u0;
    u16::try_from(scaled).unwrap_or(u16::MAX).max(gained)
}

/// The level Goku has when he arrives (the medical pod and the last fight's load): 11/10 of the mean of the three highest levels of
/// the others (`sixths` is that mean in sixths of a level), rounded, at most the cap; never below the level he has.
#[must_use]
pub(super) fn arrival_level(sixths: u32, current: u16, cap: u16) -> u16 {
    let target = (u64::from(sixths) * ARRIVAL_NUM + 3 * ARRIVAL_DEN) / (6 * ARRIVAL_DEN);
    let target = u16::try_from(target).unwrap_or(u16::MAX).min(cap);
    current.max(target)
}

/// The party records other than Goku's, as the party's level is read from them.
fn others_than_goku(ctx: &Ctx<'_>) -> Vec<Fighter> {
    (0..PARTY_SLOTS)
        .map(party_record)
        .filter(|&record| ctx.peek8(record + CHARACTER) != SUPER_SAIYAN_ID)
        .map(|record| Fighter {
            flags: ctx.peek8(record + FLAGS),
            unit: ctx.peek8(record + 1),
            level: effective_level(ctx, record),
            character: ctx.peek8(record + CHARACTER),
        })
        .collect()
}

/// The level the party brings Goku to now: [`arrival_level`] of the party's other members and the level he has.
fn goku_arrival(ctx: &Ctx<'_>, current: u16, cap: u16) -> Option<u16> {
    party_sixths(&others_than_goku(ctx)).map(|sixths| arrival_level(sixths, current, cap))
}

/// The Goku of the last fight before he joins, for the party's level that the last fight's enemy is made for (`scaling`): the
/// enemy is built before the load that gives Goku his record, from the roster. `fighters` are the party's records; `None` unless
/// the pack has the level cap, the fight is the last one, Goku is not in the party yet and the roster has him.
#[must_use]
pub(super) fn arriving_goku(ctx: &Ctx<'_>, fighters: &[Fighter]) -> Option<Fighter> {
    let cap = level_cap(ctx)?;
    if ctx.peek8(GROUP_INDEX) != LAST_FIGHT
        || ctx.peek8(ROSTER_GOKU + FLAGS) & 0xC0 == 0
        || ctx.peek8(ROSTER_GOKU + CHARACTER) != SUPER_SAIYAN_ID
        || fighters.iter().any(|f| f.character == SUPER_SAIYAN_ID && f.flags >= 0xC0)
    {
        return None;
    }
    let others: Vec<Fighter> =
        fighters.iter().filter(|f| f.character != SUPER_SAIYAN_ID).copied().collect();
    let sixths = party_sixths(&others)?;
    let current = effective_level(ctx, ROSTER_GOKU);
    Some(Fighter {
        flags: 0xC0,
        unit: 0,
        level: arrival_level(sixths, current, cap),
        character: SUPER_SAIYAN_ID,
    })
}

/// Which call of `03:C64A` this is, from the address it returns to and the scene: a gift the story gives at a level (the reference
/// of [`scaled_gain`]), or Goku's raise after the medical pod.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gift {
    Scaled { reference: u16 },
    Pod,
}

/// The gift a call of `03:C64A` that returns to `ret` in the scene `scene` is, if it is one the pack changes.
#[must_use]
fn gift_of(ret: u32, scene: u8) -> Option<Gift> {
    match (ret, scene) {
        (EFFECT_RETURN, NAIL_SCENE) => Some(Gift::Scaled { reference: NAIL_LEVEL }),
        (EFFECT_RETURN, DENDE_SCENE) => Some(Gift::Scaled { reference: DENDE_LEVEL }),
        (GURU_RETURN, _) => Some(Gift::Scaled { reference: GURU_LEVEL }),
        (POD_RETURN, _) => Some(Gift::Pod),
        _ => None,
    }
}

/// The gift the call that has just entered `03:C64A` is: the return address of its `JSL` is on the stack.
fn gift_context(ctx: &Ctx<'_>) -> Option<Gift> {
    let s = u32::from(ctx.s());
    let ret = u32::from(ctx.peek8(s + 1))
        | u32::from(ctx.peek8(s + 2)) << 8
        | u32::from(ctx.peek8(s + 3)) << 16;
    gift_of(ret, ctx.peek8(SCENE_ID))
}

/// `03:C64A`: the level routine of the story (`$164C` the character, `$164D` the levels: `$164D + 1` are given). Its callers other
/// than the three below, the floors, are the original's (a floor's negative delta wraps in 8 bits, and a floor is a catch-up, not
/// a gift). With the cap, for a character that has a table:
/// - the gifts of Piccolo's fusions (the effect `$38` in the scenes `$37` and `$3C`) and of Guru's unlocking (effect `$5B`) give
///   the levels [`scaled_gain`] says: the BP they were worth at the level the story gives them, whatever the level of the
///   character now (a Piccolo of 100 gets about 2.3 times the levels a Piccolo of 35 gets, and BP rises with `u^3`);
/// - the raise of Goku after the medical pod (scene `$3F`, to 49 in the original, which is 47 at his cap) takes him to the
///   level of the party ([`arrival_level`]) and never lowers him.
fn gift(ctx: &mut Ctx<'_>) {
    let Some(cap) = level_cap(ctx) else {
        ctx.resume_at(GIFT_KEY);
        return;
    };
    let Some(context) = gift_context(ctx) else {
        ctx.resume_at(GIFT_KEY);
        return;
    };
    let id = ctx.peek8(GIFT_CHARACTER);
    let Some(record) = (0..PARTY_SLOTS).map(party_record).find(|&record| {
        ctx.peek8(record + FLAGS) & 0x80 != 0 && ctx.peek8(record + CHARACTER) == id
    }) else {
        // The original finds no record and does nothing.
        ctx.set_cost(0);
        return;
    };
    if id >= WITH_TABLE {
        ctx.resume_at(GIFT_KEY);
        return;
    }
    let before = effective_level(ctx, record);
    let after = match context {
        Gift::Scaled { reference } => {
            let gained = u16::from(ctx.peek8(GIFT_LEVELS)) + 1;
            (before + scaled_gain(gained, before, reference)).min(cap)
        }
        Gift::Pod if id == SUPER_SAIYAN_ID => goku_arrival(ctx, before, cap).unwrap_or(before),
        Gift::Pod => before,
    };
    if after > before {
        raise_levels(ctx, record, id, before, after);
    }
    ctx.set_cost(0);
}

/// `03:C845`: the party of the last fight is loaded (effect `$3C`): Goku, built from the roster, and the others of the list. If he
/// is below the level the party gives him ([`arrival_level`], 11/10 of the mean of the three highest of the others) he is raised to
/// it, with his rows, HP and KI full.
fn arrival(ctx: &mut Ctx<'_>) {
    if let Some(cap) = level_cap(ctx)
        && let Some(record) = party_member(ctx, SUPER_SAIYAN_ID)
    {
        let before = effective_level(ctx, record);
        if let Some(after) = goku_arrival(ctx, before, cap).filter(|&after| after > before) {
            raise_levels(ctx, record, SUPER_SAIYAN_ID, before, after);
        }
    }
    ctx.resume_at(ARRIVAL_KEY);
}

/// What Porunga's level wish past the game's cap (`wish_past_cap`) leaves for the animation of the next step: the targets the
/// record's maximum HP and KI grow to, and the BP (the record has it already).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Wish {
    pub(super) max_hp: u16,
    pub(super) max_ki: u16,
    pub(super) bp: u64,
}

/// Whether Porunga's level wish gives the party record `record` a level the game's own check refuses: a character with a table
/// whose level is at the game's cap (47, 48) or above it and below the pack's cap.
#[must_use]
pub(super) fn wish_past_cap_ok(ctx: &Ctx<'_>, record: u32) -> bool {
    let Some(cap) = level_cap(ctx) else { return false };
    let id = ctx.peek8(record + CHARACTER);
    id < WITH_TABLE
        && effective_level(ctx, record) >= u16::from(ctx.peek8(GAME_CAPS + u32::from(id)))
        && effective_level(ctx, record) < cap
}

/// Adds `gain` to the BP snapshot of character `id` (`$7E:4C00 + 3 * id`, 24 bits, and its high part in the block): the wish adds
/// the level's BP to the record and to the snapshot, which the game restores the record from after a battle.
fn add_to_snapshot(ctx: &mut Ctx<'_>, id: u8, gain: u64) {
    let Some(high_at) = snapshot_high(id) else { return };
    let at = snapshot_at(id);
    let high = if block_live_ctx(ctx) { u64::from(ctx.peek16(BLOCK + high_at as u32)) } else { 0 };
    let total = (u64::from(peek24(ctx, at)) | high << 24) + gain;
    let (low, high) = super::state::split(total);
    write24(ctx, at, low);
    if high != 0 || block_live_ctx(ctx) {
        make_block_live(ctx);
        ctx.write16(BLOCK + high_at as u32, high);
    }
}

/// Porunga's level wish on the party record `record`, past the game's cap ([`wish_past_cap_ok`]; `None` when it does not apply): one
/// level, as the original's commit gives it (`01:E41D`) on the extended rows: the level (the byte saturates at 99), the EXP the
/// level begins at, the level's BP added to the record and to the snapshot, and byte `$0E`. The maximum HP and KI are not changed:
/// the next step, an animation, grows them to the targets of the result.
pub(super) fn wish_past_cap(ctx: &mut Ctx<'_>, record: u32) -> Option<Wish> {
    if !wish_past_cap_ok(ctx, record) {
        return None;
    }
    let id = ctx.peek8(record + CHARACTER);
    let level = effective_level(ctx, record) + 1;
    let row = row_at(ctx, id, level);
    set_level(ctx, record, id, level);
    set_exp(ctx, record, id, row.exp_start);
    let bp = bp_of(ctx, record) + row.bp_gain;
    set_bp(ctx, record, bp);
    add_to_snapshot(ctx, id, row.bp_gain);
    ctx.write8(record + BYTE_0E, row.byte_0e);
    let max_ki = (u64::from(ctx.peek16(record + MAX_KI)) + u64::from(row.ki_gain)).min(HP_KI_MAX);
    Some(Wish { max_hp: row.max_hp, max_ki: max_ki as u16, bp })
}

/// The variables of the Item effect that Porunga's level wish uses: the chosen member's offset, the effect's step and pending bits,
/// the targets of the animation of the next step (maximum HP, maximum KI, BP: 24 bits) and the Kaioken's byte.
const EFFECT_TARGET: u32 = 0x7E_0748;
const EFFECT_STEP: u32 = 0x7E_073C;
const EFFECT_PENDING: u32 = 0x7E_073B;
const HP_TARGET: u32 = 0x7E_0740;
const KI_TARGET: u32 = 0x7E_0742;
const BP_TARGET: u32 = 0x7E_0744;
const KAIOKEN: u32 = 0x7E_0C36;

/// `01:E3FC`: the chosen member's wish is committed (step 7 of the effect of Porunga, `01:E2A5`, after the A button): the original
/// refuses a member at his game cap (`03:84EE`), which with the level cap would stop the wish at 47 and 48. For a member at or
/// above it and below the pack's cap, with the original's other tests (flags bits 1 and 5, Goku with the Kaioken on), the wish is
/// given on the extended rows ([`wish_past_cap`]), the step goes on to the animation as the original's commit makes it, and the
/// party-wide wish (`party_boosts`) gives the same to the others. Anything else is the original's.
fn wish_step(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    let offset = ctx.peek16(EFFECT_TARGET);
    let at_cap = level_cap(ctx).is_some()
        && u32::from(offset) < 0x20 * PARTY_SLOTS
        && offset.is_multiple_of(0x20);
    if at_cap {
        let record = party_record(0) + u32::from(offset);
        let refused = ctx.peek8(record + FLAGS) & 0x22 != 0
            || (ctx.peek8(record + CHARACTER) == 0 && ctx.peek8(KAIOKEN) & 0x80 != 0);
        if !refused && let Some(wish) = wish_past_cap(ctx, record) {
            ctx.write16(HP_TARGET, wish.max_hp);
            ctx.write16(KI_TARGET, wish.max_ki);
            ctx.write16(BP_TARGET, wish.bp as u16);
            ctx.write8(BP_TARGET + 2, (wish.bp >> 16) as u8);
            ctx.write8(EFFECT_STEP, ctx.peek8(EFFECT_STEP).wrapping_add(1));
            ctx.write8(EFFECT_PENDING, ctx.peek8(EFFECT_PENDING) & 0xBF);
            super::party_boosts::wish_others(ctx);
            ctx.set_cost(0);
            return;
        }
    }
    ctx.resume_at(WISH_KEY);
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
        patch("mod_fp_form_builder", FORM_KEY, form_builder),
        patch("mod_fp_story_gift", GIFT_KEY, gift),
        patch("mod_fp_last_fight_arrival", ARRIVAL_KEY, arrival),
        patch("mod_fp_trial_win", TRIAL_WIN_KEY, trial_win),
        patch("mod_fp_trial_exp_carry", TRIAL_EXP_KEY, trial_exp_carry),
        patch("mod_fp_porunga_card", CARD_KEY, porunga_card),
        patch("mod_fp_porunga_wish", WISH_KEY, wish_step),
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
    fn a_form_keeps_the_level_and_rises_with_it_and_never_lowers_the_character() {
        // Piccolo's row 48 (1 900 000 BP, 710 HP, KI 124): at 47 or less it is the row's own, above it the Super Saiyan's rule.
        let row = (1_900_000, 710, 124);
        let nothing = (0, 0, 0);
        assert_eq!(form_stats(row, 48, nothing), (form_scaled(1_900_000, 48, 3), 724, 126));
        assert_eq!(form_stats(row, 47, nothing), (1_900_000, 710, 124), "the form is built for 47");
        // At level 80: (85 / 52)^3 = 4.37 times the BP, 85 / 52 times HP and KI.
        let (bp, hp, ki) = form_stats(row, 80, nothing);
        assert_eq!((hp, ki), (1161, 203));
        assert!((8_290_000..8_310_000).contains(&bp), "{bp}");
        // A character above the form (Guru boosts, a KI the row does not have) is not lowered: Piccolo's own KI is 178 at 47.
        assert_eq!(form_stats(row, 47, (0, 0, 178)).2, 178);
        assert_eq!(form_stats(row, 80, (9_000_000, 2000, 300)), (9_000_000, 2000, 300));
        // The maxima stay at four digits.
        let (_, hp, ki) = form_stats((1, 800, 189), 999, nothing);
        assert_eq!((hp, ki), (9_999, 3_649));
        assert_eq!(form_stats(row, 999, (0, 20_000, 20_000)).1, 9_999);
    }

    #[test]
    fn a_story_gift_is_worth_the_same_relative_rise_at_any_level() {
        // At the level the story gives it, or below, a gift is the original's.
        assert_eq!(scaled_gain(6, 35, 35), 6);
        assert_eq!(scaled_gain(6, 20, 35), 6, "never less than the gift");
        assert_eq!(scaled_gain(3, 30, 35), 3, "Guru's +3 at 30");
        // Above it the levels grow with u = level + 5: 6 * 105 / 40 = 15.75 at level 100; 6 * 1004 / 40 = 150.6 at 999.
        assert_eq!(scaled_gain(6, 100, 35), 16);
        assert_eq!(scaled_gain(6, 999, 35), 151);
        assert_eq!(scaled_gain(3, 100, 35), 8, "Guru's +3 at 100");
        // The relative rise of u is the story's: (u + gain) / u is about the same.
        let story = f64::from(6 + 40) / 40.0;
        let here = f64::from(100 + 5 + scaled_gain(6, 100, 35)) / 105.0;
        assert!((story - here).abs() < 0.01, "{story} {here}");
    }

    #[test]
    fn goku_arrives_at_eleven_tenths_of_the_top_three_and_never_lower() {
        // The mean of 40, 40 and 40 is 40, in sixths 240; 1.1 * 40 = 44.
        assert_eq!(arrival_level(240, 30, 999), 44);
        assert_eq!(arrival_level(240, 47, 999), 47, "never lowered");
        // 45 is 49.5, rounded up to 50.
        assert_eq!(arrival_level(270, 10, 999), 50);
        // The mean of 100, 90 and 80 is 90 (540 sixths): 99.
        assert_eq!(arrival_level(540, 48, 999), 99);
        // 999 stays the cap, and a party of one (the mean of one level is level * 6).
        assert_eq!(arrival_level(6 * 999, 1, 999), 999);
        assert_eq!(arrival_level(6 * 900, 1, 500), 500, "the level cap of the pack");
    }

    #[test]
    fn only_the_story_gifts_the_pack_changes_are_told_from_the_floors() {
        // The scene engine's dispatch of an effect, in the scenes of Piccolo's fusions.
        assert_eq!(gift_of(0x03_B1C9, 0x37), Some(Gift::Scaled { reference: NAIL_LEVEL }));
        assert_eq!(gift_of(0x03_B1C9, 0x3C), Some(Gift::Scaled { reference: DENDE_LEVEL }));
        // The other scenes' effect `$38` (Gohan's +3, the Snake Way's +20) and the floors are the original's.
        for scene in [0x04, 0x4C, 0x00, 0x3F] {
            assert_eq!(gift_of(0x03_B1C9, scene), None, "scene {scene:02X}");
        }
        for floor in [0x03_EAFB, 0x03_EBBB, 0x03_EC61, 0x03_EE6B, 0x03_D231] {
            assert_eq!(gift_of(floor, 0x3C), None, "{floor:06X}");
        }
        // Guru's effect `$5B` and the pod's raise are told by their own call sites.
        assert_eq!(gift_of(0x03_CFE4, 0x2D), Some(Gift::Scaled { reference: GURU_LEVEL }));
        assert_eq!(gift_of(0x03_FA85, 0x3F), Some(Gift::Pod));
    }

    #[test]
    fn the_hooks_are_at_their_addresses_and_none_is_another_hooks_key() {
        let entries = hooks();
        let keys: Vec<u32> = entries.iter().map(|entry| entry.key).collect();
        assert_eq!(
            keys,
            [
                0x03_841E, 0x03_851B, 0x05_8FDD, 0x03_85F0, 0x03_C64A, 0x03_C845, 0x05_8E3F,
                0x03_CD87, 0x05_C8D5, 0x01_E3FC
            ]
        );
        let cheats = crate::cheats::entry_points();
        for entry in &entries {
            assert!(!cheats.contains(&entry.key), "{} is a cheat's key", entry.name);
            assert_eq!(entry.mode, HookMode::Patch);
            assert!(entry.name.starts_with("mod_fp_"));
        }
    }
}
