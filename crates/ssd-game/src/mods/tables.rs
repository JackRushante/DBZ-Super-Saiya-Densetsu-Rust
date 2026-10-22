//! The data tables of the packs (M10, Task 7; `docs/re/data/tables.md`, rows 6, 7, 9 to 17): the redirectors that give the
//! game a pack's unit rows, names and command lists, attacks, encounter groups and character levels, at the addresses the
//! game reads them from. Each declines for an id without an overlay (`ssd_mod::find`), so a run without the tables is the
//! original's. Every key is a `Patch`: it never calls the original and costs nothing of its own (see [`super`]).
//!
//! The layouts are `ssd_mod::tables`': a unit is its 14-byte row and its sprite byte, an attack its KI cost, its animation's
//! original attack and its name, a level row 11 bytes, a level cap one byte.
//!
//! **Keys and the state each reads.** M and X are from the traces of `.work/re` where a key ran in them (`03:8769`, `01:BC5E`,
//! `03:870E`, `03:87D0`, `05:AB14`, `01:F630`, `01:F7DF`, `01:F6B5`, `00:8A3F`, `03:84FB`), else from the listing (`00:8AD5`,
//! `03:BD53`, `03:847A`). D is `0000` and DBR `00` at every key the traces ran:
//! - `03:8769` (M16 X16): the command list. `03:8752` wrote the unit's list pointer to `$1A..$1C`; the wrapper writes the
//!   pack's list there and in A.
//! - `01:BC5E` (M16 X16): the unit's name. `$00..$02` hold the name pointer and `Y` is 2 x the unit id.
//! - `03:870E` (M16 X16): the attack's name. `Y` is 2 x the attack id (`0` is looked up as `$44`, before the key).
//! - `03:87D0` (M8 X16): an attack's KI cost; `X` is the command id.
//! - `05:AB14` (M8 X16): an attack's animation (`1D:8E3A`); `X` is the command id.
//! - `01:F630` (M8 X16): the battle sprite of an enemy: the pose set of `01:FCA7[unit]`, the unit's `sprite_like` for a unit that
//!   borrows one.
//! - `01:FA32` (M16 X16): the anchor word of an enemy's picture in the position setup of the enemy slots below `$60`
//!   (`01:FA1A`): the word of `01:FD79[unit]` that `01:FA2E` loads. A unit that borrows a sprite takes its `sprite_like`'s
//!   word, as it takes the pose set. `X` is 2 x the unit id.
//! - `01:FBD7` (M8 X16): the picture source of an enemy in the draw of the enemy slots below `$60` (`01:FBB0`): the byte of
//!   `01:FD10[unit]` that `01:FBD3` loads, before the `PHA` that keeps it. A unit that borrows a sprite takes its
//!   `sprite_like`'s source, as it takes the pose set. `X` is the unit id.
//! - `01:F99B` (M16 X16): the piece list of an enemy's picture, in the sprite staging of the enemy slots below `$60` (`01:F4B6`,
//!   which calls `01:F983` with `Y` the slot's offset): the pointer of `02:A597[unit]` that `01:F999` loads. A unit that borrows
//!   a sprite takes its `sprite_like`'s pointer, as it takes the pose set.
//! - `01:F596` (M8 X16): the unit id of a live enemy slot in the list the battle's tile staging reads (`01:F586`, then `05:BE54`):
//!   the `STA $7F:0000,X` that stores it. A unit that borrows a sprite is listed as its `sprite_like`. `Y` is the slot's offset.
//! - `01:F7DF` (M8 X16): the end of each enemy slot of `01:F719` (the record is complete, and the slot's BP sign is drawn):
//!   the pack unit's row is copied into the record the way the original copies it. The exit `01:F7F1` cannot do it: the sign
//!   is drawn per slot, and only this tail sees each slot's seed.
//! - `00:8AD5` (M8 X16): the RTL after the override of a character 8 to 10's stats from unit row `id + 26` (`00:8A69`).
//! - `03:BD53` (M8 X16): the RTL after `03:BCF2` built a party record from the unit its `+01` names.
//! - `01:F6B5` (M8 X16): a group's slot byte in `01:F692`'s loop; `X` is 5 x the group plus the slot.
//! - `00:8A3F` (M16 X16): the level row's pointer in `00:8A28`; `Y` is the record's offset.
//! - `03:84EB` (M16 X16): the level row's pointer in the level-up check's own copy of that load (`03:84D4`, called by
//!   `03:84A5`); `Y` is the record's offset, as at `00:8A3F`.
//! - `03:84FB` (M8 X16): a character's level cap in `03:84EE`; `X` is the character id.
//! - `03:847A` (M8 X16): the level cap at the `PLX` of `03:841E`'s level-up path; `X` is the character id.
//!
//! The level cap of the promotion (`03:8600`, which reads `03:850E` too) is not redirected: it is outside the level-100 path.

use super::redirect::overlay;
use ssd_mod::Family;
use ssd_mod::tables::{ATTACK_NAME_AT, NO_SPRITE, UNIT_ITEM_LEN, UNIT_SPRITE_AT};
use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, ReturnKind};

/// `03:8769`: the `SEP #$20` after `03:8752` stored the command list's pointer (`$1A..$1B`, bank `$1C`).
pub(super) const COMMAND_LIST_KEY: u32 = 0x03_8769;
/// `01:BC5E`: the `STA $00` after the unit name's pointer is loaded (`LDA [$00],Y` at `01:BC5C`).
pub(super) const UNIT_NAME_KEY: u32 = 0x01_BC5E;
/// `01:F7DF`: the `REP #$20` after the enemy slot's record is written (the slot's offset is in `Y`).
pub(super) const ENEMY_STATS_KEY: u32 = 0x01_F7DF;
/// `00:8AD5`: the `RTL` of `00:8A69`, after the override writes the character's stats.
pub(super) const CHARACTER_STATS_KEY: u32 = 0x00_8AD5;
/// `03:BD53`: the `RTL` of `03:BCF2`, after the party record is built.
pub(super) const PARTY_STATS_KEY: u32 = 0x03_BD53;
/// `01:F630`: the `ASL A` after the sprite table's byte of the enemy's unit is loaded (`01:F62C`).
pub(super) const SPRITE_KEY: u32 = 0x01_F630;
/// `01:FA32`: the `SEC` after the anchor word of an enemy's picture is loaded (`LDA $01FD79,X` at `01:FA2E`, in the setup of the
/// enemy slots below `$60`, `01:FA1A`).
pub(super) const UNIT_ANCHOR_KEY: u32 = 0x01_FA32;
/// `01:FBD7`: the `PHA` after the picture source of an enemy's unit is loaded (`LDA $01FD10,X` at `01:FBD3`, in the draw of the
/// enemy slots below `$60`, `01:FBB0`).
pub(super) const UNIT_SOURCE_KEY: u32 = 0x01_FBD7;
/// `01:F99B`: the `STA $00` after the piece list's pointer of an enemy's unit is loaded (`LDA [$00]` at `01:F999`, in `01:F983`,
/// which the battle's sprite staging calls with `Y` the enemy slot's offset).
pub(super) const UNIT_PIECES_KEY: u32 = 0x01_F99B;
/// `01:F596`: the `STA $7F:0000,X` of an enemy slot's unit id, in the list the battle's tile staging reads (`01:F586`, `05:BE54`).
pub(super) const UNIT_STAGING_KEY: u32 = 0x01_F596;
/// `03:870E`: the `STA $00` after the attack name's pointer is loaded (`LDA [$00],Y` at `03:870C`).
pub(super) const ATTACK_NAME_KEY: u32 = 0x03_870E;
/// `03:87D0`: the `STA $00` after the KI cost is loaded (`LDA $03884B,X` at `03:87CC`).
pub(super) const ATTACK_KI_KEY: u32 = 0x03_87D0;
/// `05:AB14`: the `STA $14E0` after the animation is loaded (`LDA $1D8E3A,X` at `05:AB10`).
pub(super) const ATTACK_ANIMATION_KEY: u32 = 0x05_AB14;
/// `01:F6B5`: the `CMP #$FF` after a group's slot byte is loaded (`LDA $02BF0B,X` at `01:F6B1`).
pub(super) const GROUP_KEY: u32 = 0x01_F6B5;
/// `00:8A3F`: the `TYA` after `00:8A28` stored the level row's pointer in `$00..$02`.
pub(super) const LEVEL_ROW_KEY: u32 = 0x00_8A3F;
/// `03:84EB`: the `SEP #$20` after `03:84D4` stored the level row's pointer in `$00..$02` for the level-up check (`03:84A5`,
/// which reads the EXP threshold of the next level from the row at `Y`). The same pointer as [`LEVEL_ROW_KEY`], read by another
/// routine: without this key the check reads the original's rows, so a character can never level past 48.
pub(super) const LEVEL_THRESHOLD_KEY: u32 = 0x03_84EB;
/// `03:84FB`: the `BEQ` after the cap is loaded (`LDA $03850E,X` at `03:84F7`).
pub(super) const CAP_KEY: u32 = 0x03_84FB;
/// `03:847A`: the `PLX` after the cap is loaded (`LDA $03850E,X` at `03:8476`).
pub(super) const CAP_CHECK_KEY: u32 = 0x03_847A;

/// `01:FCA7`: a byte per unit id, which `01:F62C` reads for the battle sprite of the enemy.
const SPRITE_TABLE: u32 = 0x01_FCA7;
/// `01:FD79`: a word per unit id, the anchor of its battle picture (`01:FA2E` loads it for the position setup of the enemy slots
/// below `$60`; the draw of the slots above takes it too).
const ANCHOR_TABLE: u32 = 0x01_FD79;
/// `01:FD10`: a byte per unit id, the picture source of an enemy's draw (`01:FBD3`, and `01:F9C8` for the slots above `$60`).
const SOURCE_TABLE: u32 = 0x01_FD10;
/// `02:A597`: a word per unit id, the pointer of the unit's piece list in bank `$02` (`01:F983`).
const PIECES_TABLE: u32 = 0x02_A597;
/// `01:F7F2`: the eight words the BP of an enemy is corrected by (`01:F7F2 + 2 x (divisor & 7)`, `01:F79E`).
const BP_CORRECTION: u32 = 0x01_F7F2;
/// `1D:8E3A`: the effect and animation byte of each attack (`05:AB10`).
const ANIMATION_TABLE: u32 = 0x1D_8E3A;
/// `$7E:0E00`: the enemy records, five of 32 bytes (`01:F719`); `$7E:0E01 + Y` is the unit id of a slot.
const ENEMY_RECORDS: u16 = 0x0E00;
/// `$7E:0B00`: the party records, nine of 32 bytes; `+01` is the unit id, `+1C` the character id (`00:8A69`, `03:BCF2`).
const PARTY_RECORDS: u16 = 0x0B00;
/// `$7E:0EA3`: the encounter group index of the battle (`01:F692`).
const GROUP_INDEX: u16 = 0x0EA3;
/// `$7E:002F`: the seed of the battle's random draws (`rng_seed_a`), read after `01:F719`'s draw of a slot.
const RNG_SEED: u16 = 0x2F;
/// The offset of the BP in a unit row (bytes 4 to 6, 24 bits).
const ROW_BP: usize = 4;

/// The hook entries of the data tables, in Rust mode's place among the redirectors: all `Patch`.
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
        patch("mod_command_list", COMMAND_LIST_KEY, command_list_wrapper),
        patch("mod_unit_name", UNIT_NAME_KEY, unit_name_wrapper),
        patch("mod_enemy_stats", ENEMY_STATS_KEY, enemy_stats_wrapper),
        patch("mod_character_stats", CHARACTER_STATS_KEY, character_stats_wrapper),
        patch("mod_party_stats", PARTY_STATS_KEY, party_stats_wrapper),
        patch("mod_unit_sprite", SPRITE_KEY, sprite_wrapper),
        patch("mod_unit_anchor", UNIT_ANCHOR_KEY, unit_anchor_wrapper),
        patch("mod_unit_source", UNIT_SOURCE_KEY, unit_source_wrapper),
        patch("mod_unit_pieces", UNIT_PIECES_KEY, unit_pieces_wrapper),
        patch("mod_unit_staging", UNIT_STAGING_KEY, unit_staging_wrapper),
        patch("mod_attack_name", ATTACK_NAME_KEY, attack_name_wrapper),
        patch("mod_attack_ki", ATTACK_KI_KEY, attack_ki_wrapper),
        patch("mod_attack_animation", ATTACK_ANIMATION_KEY, attack_animation_wrapper),
        patch("mod_group", GROUP_KEY, group_wrapper),
        patch("mod_level_row", LEVEL_ROW_KEY, level_row_wrapper),
        patch("mod_level_threshold", LEVEL_THRESHOLD_KEY, level_threshold_wrapper),
        patch("mod_cap", CAP_KEY, cap_wrapper),
        patch("mod_cap_check", CAP_CHECK_KEY, cap_check_wrapper),
    ]
}

/// Stores the 24-bit pointer `address` at direct page `offset`: the low word at `offset`, the bank at `offset + 2`.
fn store_pointer(ctx: &mut Ctx<'_>, offset: u16, address: u32) {
    ctx.write_dp16(offset, address as u16);
    ctx.write_dp8(offset + 2, (address >> 16) as u8);
}

/// `03:8769`: the unit's command list. The original's list pointer (`$1A..$1C`, A) is replaced by the pack's list when the
/// unit has one. `$18` holds `2 x unit id` (`03:8752`'s `ASL`).
fn command_list_wrapper(ctx: &mut Ctx<'_>) {
    let unit = ctx.peek_dp16(0x18) / 2;
    if let Some(address) = overlay(ctx, Family::CommandList, unit) {
        store_pointer(ctx, 0x1A, address);
        ctx.set_a(address as u16);
    }
    ctx.resume_at(COMMAND_LIST_KEY);
}

/// `01:BC5E`: the unit's name, from the pack when the unit has one. `Y` is `2 x unit id`.
fn unit_name_wrapper(ctx: &mut Ctx<'_>) {
    let unit = ctx.y() / 2;
    if let Some(address) = overlay(ctx, Family::UnitName, unit) {
        store_pointer(ctx, 0x00, address);
        ctx.set_a(address as u16);
    }
    ctx.resume_at(UNIT_NAME_KEY);
}

/// `03:870E`: the attack's name, from the pack when the attack has one. `Y` is `2 x attack id`; the name follows the
/// attack's KI cost and animation in its item.
fn attack_name_wrapper(ctx: &mut Ctx<'_>) {
    let attack = ctx.y() / 2;
    if let Some(item) = overlay(ctx, Family::Attack, attack) {
        let name = item + ATTACK_NAME_AT;
        store_pointer(ctx, 0x00, name);
        ctx.set_a(name as u16);
    }
    ctx.resume_at(ATTACK_NAME_KEY);
}

/// `03:87D0`: an attack's KI cost, from the pack when the attack has one. `X` is the command id.
fn attack_ki_wrapper(ctx: &mut Ctx<'_>) {
    let attack = ctx.x();
    if let Some(item) = overlay(ctx, Family::Attack, attack) {
        let ki = ctx.peek8(item);
        ctx.set_al(ki);
    }
    ctx.resume_at(ATTACK_KI_KEY);
}

/// `05:AB14`: an attack's animation, from the pack when the attack has one: the byte `1D:8E3A` holds for the original attack
/// the pack names (`animation_like`, the second byte of the item). `X` is the command id.
fn attack_animation_wrapper(ctx: &mut Ctx<'_>) {
    let attack = ctx.x();
    if let Some(item) = overlay(ctx, Family::Attack, attack) {
        let like = u32::from(ctx.peek8(item + 1));
        let animation = ctx.peek8(ANIMATION_TABLE + like);
        ctx.set_al(animation);
    }
    ctx.resume_at(ATTACK_ANIMATION_KEY);
}

/// `01:F630`: the battle sprite of an enemy. The original takes the pose set of `01:FCA7[unit]`; a unit that borrows a sprite
/// takes the set of its `sprite_like` instead. `X` is the unit id.
fn sprite_wrapper(ctx: &mut Ctx<'_>) {
    let unit = ctx.x();
    if let Some(item) = overlay(ctx, Family::Unit, unit) {
        let sprite = ctx.peek8(item + UNIT_SPRITE_AT);
        if sprite != NO_SPRITE {
            let pose = ctx.peek8(SPRITE_TABLE + u32::from(sprite));
            ctx.set_al(pose);
        }
    }
    ctx.resume_at(SPRITE_KEY);
}

/// `01:FA32`: the anchor of an enemy's picture, for the position setup of the slots below `$60` (`01:FA1A`). A unit that
/// borrows a sprite takes its `sprite_like`'s anchor, as it takes the pose set in [`sprite_wrapper`]. `X` is 2 x the unit id.
fn unit_anchor_wrapper(ctx: &mut Ctx<'_>) {
    let unit = ctx.x() / 2;
    if let Some(item) = overlay(ctx, Family::Unit, unit) {
        let sprite = ctx.peek8(item + UNIT_SPRITE_AT);
        if sprite != NO_SPRITE {
            let anchor = ctx.peek16(ANCHOR_TABLE + 2 * u32::from(sprite));
            ctx.set_a(anchor);
        }
    }
    ctx.resume_at(UNIT_ANCHOR_KEY);
}

/// `01:FBD7`: the picture source of an enemy, for the draw of the slots below `$60` (`01:FBB0`). A unit that borrows a sprite
/// takes its `sprite_like`'s source, as it takes the pose set in [`sprite_wrapper`]. `X` is the unit id.
fn unit_source_wrapper(ctx: &mut Ctx<'_>) {
    let unit = ctx.x();
    if let Some(item) = overlay(ctx, Family::Unit, unit) {
        let sprite = ctx.peek8(item + UNIT_SPRITE_AT);
        if sprite != NO_SPRITE {
            let source = ctx.peek8(SOURCE_TABLE + u32::from(sprite));
            ctx.set_al(source);
        }
    }
    ctx.resume_at(UNIT_SOURCE_KEY);
}

/// `01:F99B`: the piece list of an enemy, for the sprite staging of the slots below `$60` (`01:F983`, with `Y` the slot's
/// offset). A unit that borrows a sprite takes its `sprite_like`'s pointer, as it takes the pose set in [`sprite_wrapper`].
fn unit_pieces_wrapper(ctx: &mut Ctx<'_>) {
    let unit = u16::from(ctx.peek_db8(ENEMY_RECORDS.wrapping_add(ctx.y()).wrapping_add(1)));
    if let Some(item) = overlay(ctx, Family::Unit, unit) {
        let sprite = ctx.peek8(item + UNIT_SPRITE_AT);
        if sprite != NO_SPRITE {
            let pieces = ctx.peek16(PIECES_TABLE + 2 * u32::from(sprite));
            ctx.set_a(pieces);
        }
    }
    ctx.resume_at(UNIT_PIECES_KEY);
}

/// `01:F596`: the unit id an enemy slot is listed by in the battle's tile staging (`01:F586`, read by `05:BE54`), for a live
/// slot (`Y` its offset). A unit that borrows a sprite is listed as its `sprite_like`, so its tiles are staged from that unit.
fn unit_staging_wrapper(ctx: &mut Ctx<'_>) {
    let base = ENEMY_RECORDS.wrapping_add(ctx.y());
    if ctx.peek_db8(base) & 0x40 != 0 {
        let unit = u16::from(ctx.peek_db8(base.wrapping_add(1)));
        if let Some(item) = overlay(ctx, Family::Unit, unit) {
            let sprite = ctx.peek8(item + UNIT_SPRITE_AT);
            if sprite != NO_SPRITE {
                ctx.set_al(sprite);
            }
        }
    }
    ctx.resume_at(UNIT_STAGING_KEY);
}

/// `01:F6B5`: a slot of an encounter group, from the pack when the group has one. `X` is `5 x group + slot`.
fn group_wrapper(ctx: &mut Ctx<'_>) {
    let group = ctx.peek_db8(GROUP_INDEX);
    let slot = ctx.x().wrapping_sub(5 * u16::from(group));
    if slot < 5
        && let Some(item) = overlay(ctx, Family::Group, u16::from(group))
    {
        let unit = ctx.peek8(item + u32::from(slot));
        ctx.set_al(unit);
    }
    ctx.resume_at(GROUP_KEY);
}

/// `00:8A3F`: the pointer of a character's level rows, from the pack when the character has rows. `Y` is the record's offset
/// (the character id is at `+1C`); the caller then reads the row of the level at `Y`, 11 bytes each.
fn level_row_wrapper(ctx: &mut Ctx<'_>) {
    level_rows(ctx);
    ctx.resume_at(LEVEL_ROW_KEY);
}

/// [`LEVEL_THRESHOLD_KEY`]: the same pointer for the level-up check's own load; it goes on at its own key.
fn level_threshold_wrapper(ctx: &mut Ctx<'_>) {
    level_rows(ctx);
    ctx.resume_at(LEVEL_THRESHOLD_KEY);
}

/// The pack's level rows of the character whose record is at `Y`, stored as the pointer at `$00..$02`.
fn level_rows(ctx: &mut Ctx<'_>) {
    let record = ctx.y();
    let character = ctx.peek_db8(PARTY_RECORDS.wrapping_add(record) + 0x1C);
    if let Some(address) = overlay(ctx, Family::LevelRows, u16::from(character)) {
        store_pointer(ctx, 0x00, address);
    }
}

/// `03:84FB`: a character's level cap, from the pack when the character has one. `X` is the character id. The flags are set
/// as the original's load sets them (the `BEQ` that follows tests them).
fn cap_wrapper(ctx: &mut Ctx<'_>) {
    let character = ctx.x();
    if let Some(item) = overlay(ctx, Family::LevelCap, character) {
        let cap = ctx.peek8(item);
        ctx.set_al(cap);
        ctx.set_nz8(cap);
    }
    ctx.resume_at(CAP_KEY);
}

/// `03:847A`: a character's level cap at the level-up path, from the pack when the character has one. `X` is the character id.
fn cap_check_wrapper(ctx: &mut Ctx<'_>) {
    let character = ctx.x();
    if let Some(item) = overlay(ctx, Family::LevelCap, character) {
        let cap = ctx.peek8(item);
        ctx.set_al(cap);
    }
    ctx.resume_at(CAP_CHECK_KEY);
}

/// The 15 bytes of the unit item at `item`: its row and its sprite byte.
fn row_at(ctx: &Ctx<'_>, item: u32) -> [u8; UNIT_ITEM_LEN] {
    let mut row = [0; UNIT_ITEM_LEN];
    for (offset, byte) in row.iter_mut().enumerate() {
        *byte = ctx.peek8(item + offset as u32);
    }
    row
}

/// The 24-bit BP of a unit row (bytes 4 to 6).
fn row_bp(row: &[u8; UNIT_ITEM_LEN]) -> u32 {
    u32::from_le_bytes([row[ROW_BP], row[ROW_BP + 1], row[ROW_BP + 2], 0])
}

/// Writes the fields of a record that `01:F719`, `00:8A69` and `03:BCF2` take from a unit row, at the record `base`: the level
/// (`+02`), HP (`+03`, maximum `+0A`), KI (`+05`, maximum `+0C`), BP (`+07..+09`), tie (`+0E`), the defence factor (`+17`) and
/// the class (`+1B`). The EXP (`+0F..+11`) and the flags (`+00`) are not written: the original leaves them to its own code.
fn write_record(ctx: &mut Ctx<'_>, base: u16, row: &[u8; UNIT_ITEM_LEN], bp: u32) {
    let hp = u16::from_le_bytes([row[2], row[3]]);
    let ki = u16::from_le_bytes([row[7], row[8]]);
    ctx.write_db8(base + 0x02, row[1]);
    ctx.write_db16(base + 0x03, hp);
    ctx.write_db16(base + 0x0A, hp);
    ctx.write_db16(base + 0x05, ki);
    ctx.write_db16(base + 0x0C, ki);
    let [bp0, bp1, bp2, _] = bp.to_le_bytes();
    ctx.write_db8(base + 0x07, bp0);
    ctx.write_db8(base + 0x08, bp1);
    ctx.write_db8(base + 0x09, bp2);
    ctx.write_db8(base + 0x0E, row[10]);
    ctx.write_db8(base + 0x17, row[9]);
    ctx.write_db8(base + 0x1B, row[0]);
}

/// The packed BCD of the two low decimal digits of `bp`, as `06:E969` leaves them in `$03` (the digits' value as a byte).
fn bcd_low_digits(bp: u32) -> u8 {
    let low = (bp % 100) as u8;
    ((low / 10) << 4) | (low % 10)
}

/// The BP `01:F719` gives an enemy from its unit row: the row's BP less the packed digits `divisor`, plus or minus the
/// correction word `correction`. The seed's bit 0 chooses the sign (clear: plus). The plus path does what the original does
/// with its borrow: the subtraction's borrow stays out of the high byte (the `CLC` before its add), so the result can be
/// 65536 more than exact arithmetic gives; the minus path borrows exactly.
fn enemy_bp(row_bp: u32, divisor: u8, correction: u16, seed: u8) -> u32 {
    let low = row_bp as u16;
    let high = (row_bp >> 16) as u8;
    if seed & 1 == 0 {
        let (sum, carry) = low.wrapping_sub(u16::from(divisor)).overflowing_add(correction);
        (u32::from(high.wrapping_add(u8::from(carry))) << 16) | u32::from(sum)
    } else {
        let value = i64::from(low) - i64::from(divisor) - i64::from(correction);
        let borrow = u8::from(value < 0);
        (u32::from(high.wrapping_sub(borrow)) << 16) | (value & 0xFFFF) as u32
    }
}

/// `01:F7DF`: the end of an enemy slot's record in `01:F719`. A live slot whose unit has a pack row takes the row's fields, and
/// its BP the way `01:F719` computes one. `Y` is the slot's offset in the enemy records.
fn enemy_stats_wrapper(ctx: &mut Ctx<'_>) {
    let base = ENEMY_RECORDS.wrapping_add(ctx.y());
    if ctx.peek_db8(base) == 0 {
        ctx.resume_at(ENEMY_STATS_KEY);
        return;
    }
    let unit = u16::from(ctx.peek_db8(base.wrapping_add(1)));
    if let Some(item) = overlay(ctx, Family::Unit, unit) {
        let row = row_at(ctx, item);
        let row_bp = row_bp(&row);
        let divisor = bcd_low_digits(row_bp);
        let correction = ctx.peek16(BP_CORRECTION + 2 * u32::from(divisor & 7));
        let seed = ctx.peek_dp8(RNG_SEED);
        let bp = enemy_bp(row_bp, divisor, correction, seed);
        write_record(ctx, base, &row, bp);
    }
    ctx.resume_at(ENEMY_STATS_KEY);
}

/// `00:8AD5`: the stats of a character 8 to 10 (not 11), from unit row `id + 26` when the unit has a pack row (`00:8A69`
/// copies the ROM row here). `Y` is the record's offset.
fn character_stats_wrapper(ctx: &mut Ctx<'_>) {
    let record = ctx.y();
    let character = ctx.peek_db8(PARTY_RECORDS.wrapping_add(record) + 0x1C);
    if (8..=255).contains(&character) && character != 11 {
        // The original adds 26 in 8 bits (`CLC; ADC #$1A`), so the unit id wraps.
        let unit = u16::from(character.wrapping_add(26));
        if let Some(item) = overlay(ctx, Family::Unit, unit) {
            let row = row_at(ctx, item);
            let bp = row_bp(&row);
            write_record(ctx, PARTY_RECORDS.wrapping_add(record), &row, bp);
        }
    }
    ctx.resume_at(CHARACTER_STATS_KEY);
}

/// `03:BD53`: the party record `03:BCF2` has built from the unit its `+01` names, from the pack's row when the unit has one.
/// `Y` is the record's offset. The BP is copied exactly, as `03:BCF2` copies it.
fn party_stats_wrapper(ctx: &mut Ctx<'_>) {
    let record = ctx.y();
    let unit = u16::from(ctx.peek_db8(PARTY_RECORDS.wrapping_add(record) + 0x01));
    if let Some(item) = overlay(ctx, Family::Unit, unit) {
        let row = row_at(ctx, item);
        let bp = row_bp(&row);
        write_record(ctx, PARTY_RECORDS.wrapping_add(record), &row, bp);
    }
    ctx.resume_at(PARTY_STATS_KEY);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_are_the_instructions_named_in_the_documentation() {
        assert_eq!(COMMAND_LIST_KEY, 0x03_8769);
        assert_eq!(UNIT_NAME_KEY, 0x01_BC5E);
        assert_eq!(ENEMY_STATS_KEY, 0x01_F7DF);
        assert_eq!(CHARACTER_STATS_KEY, 0x00_8AD5);
        assert_eq!(PARTY_STATS_KEY, 0x03_BD53);
        assert_eq!(SPRITE_KEY, 0x01_F630);
        assert_eq!(UNIT_ANCHOR_KEY, 0x01_FA32);
        assert_eq!(UNIT_SOURCE_KEY, 0x01_FBD7);
        assert_eq!(UNIT_PIECES_KEY, 0x01_F99B);
        assert_eq!(UNIT_STAGING_KEY, 0x01_F596);
        assert_eq!(GROUP_KEY, 0x01_F6B5);
        assert_eq!(LEVEL_ROW_KEY, 0x00_8A3F);
        assert_eq!(LEVEL_THRESHOLD_KEY, 0x03_84EB);
        assert_eq!(CAP_KEY, 0x03_84FB);
        assert_eq!(CAP_CHECK_KEY, 0x03_847A);
    }

    #[test]
    fn no_key_is_a_cheat_key_and_all_are_patches_that_cost_nothing() {
        let cheats = crate::cheats::entry_points();
        for entry in hook_entries() {
            assert!(!cheats.contains(&entry.key), "{} is a cheat's key", entry.name);
            assert_eq!(entry.mode, HookMode::Patch, "{}", entry.name);
            assert_eq!(entry.cycles, Some(0), "{}", entry.name);
        }
    }

    #[test]
    fn the_digits_are_the_packed_bcd_of_the_bp_mod_100() {
        assert_eq!(bcd_low_digits(123_456), 0x56);
        assert_eq!(bcd_low_digits(65_536), 0x36);
        assert_eq!(bcd_low_digits(0), 0x00);
        assert_eq!(bcd_low_digits(99), 0x99);
        assert_eq!(bcd_low_digits(16_777_215), 0x15);
    }

    #[test]
    fn the_minus_path_borrows_exactly_and_the_plus_path_keeps_its_borrow_out_of_the_high_byte() {
        // Minus: 65536 - 54 - 100 = 65382, exact.
        assert_eq!(enemy_bp(65_536, 54, 100, 0x01), 65_382);
        // Plus: low 0 - 54 wraps to 0xFFCA, plus 100 is 0x002E with a carry, so the high byte goes to 2: 131118, where the exact sum is 65582.
        assert_eq!(enemy_bp(65_536, 54, 100, 0x00), 131_118);
        // Without a borrow the two agree: 300000 - 6 + 50 (the divisor 0x06, the correction 50).
        assert_eq!(enemy_bp(300_000, 0x06, 50, 0x00), 300_044);
        assert_eq!(enemy_bp(300_000, 0x06, 50, 0x01), 299_944);
    }

    #[test]
    fn the_bp_of_a_row_is_its_three_bytes_and_the_layouts_are_the_ssd_mod_ones() {
        // The fields a record takes from a unit row (`units.md`): class 0, level 1, HP 2..3, BP 4..6, KI 7..8, defence 9, tie 10.
        let mut row = [0_u8; UNIT_ITEM_LEN];
        row[0] = 3;
        row[1] = 9;
        row[2..4].copy_from_slice(&300_u16.to_le_bytes());
        row[4..7].copy_from_slice(&[0x40, 0x42, 0x0F]);
        row[7..9].copy_from_slice(&50_u16.to_le_bytes());
        row[9] = 30;
        row[10] = 7;
        assert_eq!(row_bp(&row), 0x0F_4240);
        assert_eq!(ssd_mod::tables::LEVEL_ROW_LEN, 11, "the level row is 11 bytes");
        assert_eq!(ATTACK_NAME_AT, 2, "an attack's name follows its KI and animation");
    }
}

/// The level-up check (`03:841E`) run headless on the batch 4a test pack, which gives character 0 a cap of 100 and a curve
/// past the original's rows (`.work/m10/tables-pack`, copied from the batch's scratch; the pack is ours, not the ROM's).
#[cfg(test)]
mod level_check_rom {
    use ssd_oracle::{Joypad, Machine, MemorySaveStore, NullAudio, StatusRegister, Stops};

    /// The bus address of the routine's call stub: `jsl $03:841E` is written here and run from `$7F:FE00`.
    const STUB: u32 = 0x7F_FE00;
    /// The address the stub returns to (after its four bytes).
    const BACK: u32 = STUB + 4;
    /// `$7E:0B00`: the first party record; `+02` its level, `+0F..+11` its EXP (24 bits).
    const RECORD: u32 = 0x7E_0B00;

    #[test]
    #[ignore = "needs SSD_ROM_PATH and the test pack in .work/m10/tables-pack"]
    fn one_call_of_the_check_takes_goku_from_47_to_100_with_the_pack_curve() {
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
        // Boot: the game runs its title with no input, so the work RAM is the game's own before the record is staged.
        let frame = Stops { frame_end: true, ..Stops::default() };
        for _ in 0..300 {
            machine.run(&frame, &mut |_| Joypad::default(), &mut NullAudio, 1).unwrap();
        }
        // Goku (character 0, unit 0x39 is not needed by the check), present (`$C0`), level 47, EXP 100 000 (`$0186A0`).
        machine.write_wram(RECORD, &[0xC0]).unwrap();
        machine.write_wram(RECORD + 0x02, &[47]).unwrap();
        machine.write_wram(RECORD + 0x1C, &[0]).unwrap();
        machine.write_wram(RECORD + 0x0F, &[0xA0, 0x86, 0x01]).unwrap();
        machine.write_wram(STUB, &[0x22, 0x1E, 0x84, 0x03]).unwrap();
        let mut registers = machine.cpu_registers();
        registers.pbr = (STUB >> 16) as u8;
        registers.pc = (STUB & 0xFFFF) as u16;
        registers.dbr = 0;
        registers.d = 0;
        registers.y = 0;
        registers.p = StatusRegister::from(0x24);
        machine.set_cpu_registers(registers);
        machine.set_breakpoint(BACK);
        let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
        let stopped = machine.run(&stops, &mut |_| Joypad::default(), &mut NullAudio, 2).unwrap();
        assert!(
            matches!(stopped, ssd_oracle::Stopped::Breakpoint { key } if key == BACK),
            "the check did not return: {stopped:?}"
        );
        let wram = machine.wram();
        let level = wram[(RECORD - 0x7E_0000) as usize + 0x02];
        let exp = &wram[(RECORD - 0x7E_0000) as usize + 0x0F..(RECORD - 0x7E_0000) as usize + 0x12];
        assert_eq!(level, 100, "one call of the check reaches the cap of the pack");
        assert_eq!(exp, [0xA0, 0x86, 0x01], "the EXP is not spent by the check");
    }
}
