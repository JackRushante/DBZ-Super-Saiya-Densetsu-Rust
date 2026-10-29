//! The enemies that follow the party's level (M10, Task 19): the named feature `enemy_scaling`. The design and its numbers are
//! in `docs/mods/free-progression.md`; the parameter is in `docs/mods/features.md`. In short, every enemy of a battle is set to
//! the strength it would have if it had been made for the party's level, on the curve the original gives its own enemies and
//! the party's levels, and the difficulty moves that level up or down. A pack may instead give two offsets in levels from the
//! party's, one for the random battles and one for every other battle ([`Scaling::Offsets`]).
//!
//! **The model** (all integer arithmetic, so that a replay plays the same on every machine). With `u = level + 5`:
//! - the party's level `P` is the mean of the levels of its three highest fighters ([`party_sixths`]), kept in sixths;
//! - the difficulty is a factor on the party's `u`: 7/8, 1 or 9/8;
//! - the enemy's own level `L` is the level of its record (the unit row's, or the pack row's), the level the enemy was made for;
//! - `q = difficulty * (P + 5) / (L + 5)` (with offsets, `q = (P + offset + 5) / (L + 5)`, the offset of the kind of battle); then HP and KI are scaled by `q`, BP by `q^3`, the EXP it pays by `q^2`, and the level
//!   of its record becomes the party's level with the difficulty (`difficulty * (P + 5) - 5`). At `P = L` and the normal
//!   difficulty `q` is exactly 1 and every number is the original's.
//!
//! **Where it acts.** Both keys are `Patch`es at the `RTL` of a routine of the original, which they let run:
//! - `01:F7F1` ([`STATS_KEY`]), the `RTL` of the enemy record builder `01:F719`: the five records are complete (the
//!   data tables' redirector of `01:F7DF` has written a pack unit's row, and the original has drawn its BP's small correction),
//!   and the pack's scaling is applied once to every record the builder filled. The enemy power cheat acts after this, on the
//!   scaled numbers, because its wrapper (`cheats`) calls the original and multiplies when it returns.
//! - `1D:8C66` ([`EXP_KEY`]), the `RTL` of the EXP award `1D:8C33` (one defeated enemy): the reward the original has added to
//!   the battle's EXP is replaced by the scaled one. The EXP cheat multiplies the whole award, scaled, when it returns.
//!
//! Four more hooks keep the fights that wait for an HP floor ([`FLOORS`]) on the story's path (see "The HP-threshold fights" below).
//!
//! A hook declines at once (it only resumes at its own key) without the feature in the image, in a fight of the scripted-loss
//! list ([`crate::cheats::SCRIPTED_LOSS`]), and without a fighter in the party. A BP above 24 bits is kept as the wide BP of
//! `super::state` keeps it: the low 24 bits in the record and the high 16 in the enemy's part of the mod state block, which the
//! hook makes live when it must (the block is left alone while every BP fits 24 bits).

use super::redirect::overlay;
use super::state::{
    BATTLE_KIND, BLOCK, ENEMY_HIGH, ENEMY_SLOTS, KIND_MARKER, MARKER, MARKER_AT, PARTY_SLOTS,
    SCRATCH_END, enemy_record, party_record, split,
};
use ssd_menus::levels::RECORD_LEVEL_MAX;
use ssd_mod::Family;
use ssd_mod::feature::{ENEMY_SCALING, SCALING_OFFSETS};
use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, ReturnKind, RoutineFn};

/// `01:F7F1`: the `RTL` of `01:F719`, which the loop over the five enemy slots reaches when it is done.
pub(super) const STATS_KEY: u32 = 0x01_F7F1;
/// `1D:8C66`: the `RTL` of `1D:8C33`, after the reward of one defeated enemy is added to the battle's EXP.
pub(super) const EXP_KEY: u32 = 0x1D_8C66;

/// `02:D37C`: the `RTL` of `02:D34A`, the pick of a random encounter's group. `02:D34A` has one caller, `02:D250`, in the
/// normal-battle branch of the roll that starts a battle (`02:D241`); a story battle (a script's battle, the roll's own story
/// branch, a form change) never runs it. It is the game's own line between the two kinds of battle.
pub(super) const PICK_KEY: u32 = 0x02_D37C;

/// `05:A51D`: the `CPY $1443` of `05:A4F5`, the step of the hit that lets four enemies withdraw at a floor of HP; `Y` holds the floor.
pub(super) const FLOOR_KEY: u32 = 0x05_A51D;
/// `05:A46E`: the `BCC` after `05:BDDC` (the hit's HP subtraction) in the hit step `05:A435`; the carry is clear when the HP went below zero.
pub(super) const LETHAL_KEY: u32 = 0x05_A46E;
/// `05:A49F`: the store that keeps the Ginyu duel's enemy at 1 HP when a hit would kill him (`A` holds 1).
pub(super) const GINYU_KEY: u32 = 0x05_A49F;
/// `05:9808`: the `BCC` after the `CPY #$15` with which the end of the second form's fight (group `$54`) judges the HP (`05:9802`).
pub(super) const JUDGE_KEY: u32 = 0x05_9808;

/// `$7E:0EA3`: the encounter group of the battle (`01:F692`).
const GROUP_INDEX: u32 = 0x7E_0EA3;
/// `$7E:0EA0`: the number of enemies in the battle (`01:F692`); `01:F719` starts its loop at slot 0 for an odd count and at
/// slot 1 for an even one.
const ENEMY_COUNT: u32 = 0x7E_0EA0;
/// `$7E:14F3`: the EXP of the battle so far, 24 bits (`1D:8C33` adds each defeated enemy's reward).
const BATTLE_EXP: u32 = 0x7E_14F3;
/// `02:B214`: the unit rows, 14 bytes each, 105 of them; `1D:8C33` reads the reward of a unit at `+11` (24 bits).
const UNIT_STATS: u32 = 0x02_B214;
const UNIT_ROW_LEN: u32 = 14;
const UNIT_ROWS: u32 = 105;
/// The offsets in a unit row of the level and the EXP reward.
const ROW_LEVEL: u32 = 1;
const ROW_EXP: u32 = 11;

/// The offsets in a party or enemy record (`characters.md`): flags, unit, level, HP, KI, BP, maximum HP, maximum KI, character.
const FLAGS: u32 = 0x00;
const UNIT: u32 = 0x01;
const LEVEL: u32 = 0x02;
const HP: u32 = 0x03;
const KI: u32 = 0x05;
const BP: u32 = 0x07;
const MAX_HP: u32 = 0x0A;
const MAX_KI: u32 = 0x0C;
const CHARACTER: u32 = 0x1C;

/// `$7E:1440`: the record of the enemy the hit step works on (side B), laid out as an enemy record; `$7E:1443` is its HP.
const SIDE_B: u32 = 0x7E_1440;
/// `$7E:15F1`: the Ginyu duel's body swap state; 2 once the swap is done (`05:D28F`), and the 1-HP guard of the duel ends with it.
const SWAP_STATE: u32 = 0x7E_15F1;
/// The group of the Ginyu duel, the second form's fight, and the units whose fights wait for a floor of HP (`05:A4F5`): Zarbon's
/// first form and Frieza's first three forms, and the floor (in HP) at which the story takes the enemy away.
const GINYU_GROUP: u8 = 0x51;
const SECOND_FORM_GROUP: u8 = 0x54;
const SECOND_FORM_UNIT: u8 = 0x36;
const FLOORS: [(u8, u16); 4] = [(0x1E, 30), (0x35, 300), (0x36, 20), (0x37, 50)];

/// The shift of the levels in the model: `u = level + 5` (see the module documentation).
const SHIFT: u128 = 5;
/// The highest level an enemy is made for: the highest level of the party (`level_cap`, at most 999). The record's own byte
/// saturates at 99 (`ssd_menus::levels::RECORD_LEVEL_MAX`: the special attack's roll adds `10 + level + a byte` in 8 bits).
const MAX_LEVEL: u16 = 999;
/// The largest BP the wide record holds (40 bits), the HP and KI of a record (16 bits) and the EXP award (24 bits).
const MAX_BP: u64 = (1 << 40) - 1;
const MAX_WORD: u64 = 0xFFFF;
const MAX_EXP: u64 = 0xFF_FFFF;

/// The characters whose level is not the party's progress: 8 to 10 (Nail, Tsumuri and Maima, whose stats are their unit's),
/// 11 (Dende, who stops at level 3) and 12 (which the level-up check leaves out as well, `03:8429`).
const WITHOUT_LEVELS: [u8; 5] = [8, 9, 10, 11, 12];
/// The unit of the temporary party record the story gives the borrowed Vegeta (`01:EAD5`), which earns no EXP either.
const TEMPORARY_UNIT: u8 = 0x4A;

/// How far the enemies are set above or below the party's level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Difficulty {
    Easy,
    Normal,
    Hard,
}

impl Difficulty {
    /// The difficulty a feature byte stands for (`ssd_mod::feature`: `easy` 0, `normal` 1, `hard` 2).
    pub(super) fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Easy),
            1 => Some(Self::Normal),
            2 => Some(Self::Hard),
            _ => None,
        }
    }

    /// The factor on the party's `u`, in eighths: 7/8, 1 and 9/8.
    const fn eighths(self) -> u128 {
        match self {
            Self::Easy => 7,
            Self::Normal => 8,
            Self::Hard => 9,
        }
    }
}

/// How the enemies' target level is set from the party's: by a factor (the pack's `difficulty`), or by a number of levels for
/// each kind of battle (the pack's `random_offset` and `boss_offset`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scaling {
    /// The target is `difficulty * (P + 5) - 5` for every battle.
    Difficulty(Difficulty),
    /// The target is `P + random` for a random battle and `P + story` for any other, in levels (at least 1, at most
    /// [`MAX_LEVEL`]).
    Offsets { random: i8, story: i8 },
}

impl Scaling {
    /// The scaling a feature slot's bytes stand for: the first byte is a difficulty (0 to 2, the slot of a pack written with
    /// `difficulty` alone has no other byte), or [`SCALING_OFFSETS`] followed by the two offsets as signed bytes.
    pub(super) fn from_bytes(mode: u8, random: u8, story: u8) -> Option<Self> {
        if mode == SCALING_OFFSETS {
            return Some(Self::Offsets { random: random as i8, story: story as i8 });
        }
        Difficulty::from_byte(mode).map(Self::Difficulty)
    }
}

/// The two kinds of battle: a random encounter (the pick of `02:D34A` ran for it) or any other, which is every story battle,
/// boss and scripted fight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Random,
    Story,
}

/// One party record, as the party's level is read from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Fighter {
    pub(super) flags: u8,
    pub(super) unit: u8,
    pub(super) level: u16,
    pub(super) character: u8,
}

/// The party's level in sixths of a level, from the party records: the mean of the three highest levels among the fighters
/// that grow (a record in the party, flags `$80` or `$40`, so a fighter who is down still counts; not one of
/// [`WITHOUT_LEVELS`] or the [`TEMPORARY_UNIT`]; with a level). A party with fewer than three of them is the mean of those it has,
/// and one with none is the mean of every record in the party. `None` when no record is in the party.
///
/// Sixths hold the mean of one, two or three levels exactly (the sum times 6, 3 or 2), so that the identity of the scaling is
/// exact: a party whose mean is the enemy's level gives a ratio of exactly 1.
#[must_use]
pub(super) fn party_sixths(fighters: &[Fighter]) -> Option<u32> {
    let present = |f: &&Fighter| f.flags & 0xC0 != 0 && f.level != 0;
    let mut levels: Vec<u16> = fighters
        .iter()
        .filter(present)
        .filter(|f| !WITHOUT_LEVELS.contains(&f.character) && f.unit != TEMPORARY_UNIT)
        .map(|f| f.level)
        .collect();
    if levels.is_empty() {
        levels = fighters.iter().filter(present).map(|f| f.level).collect();
    }
    levels.sort_unstable_by(|a, b| b.cmp(a));
    levels.truncate(3);
    let count = u32::try_from(levels.len()).ok().filter(|&count| count != 0)?;
    Some(levels.iter().map(|&level| u32::from(level)).sum::<u32>() * (6 / count))
}

/// The level the enemies of one kind of battle are made for, resolved once from the party and the scaling: the party's `u`
/// with the difficulty or the offset, in 48ths (`(P + 5) * factor`, or `(P + offset + 5)` times 8), and the level of the record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Target {
    effective: u128,
}

/// The target of a battle of `kind`: `party_sixths` is the party's level in sixths.
///
/// With a difficulty it is the party's `u` times the factor. With offsets it is the party's level plus the kind's offset, kept
/// between level 1 and [`MAX_LEVEL`] (a level below 1 does not exist, and the BP of a level 0 or less is no number), in sixths so
/// that the identity holds: a party whose mean is `L` and an offset of 0 give `q = 1` exactly.
#[must_use]
pub(super) fn target(party_sixths: u32, scaling: Scaling, kind: Kind) -> Target {
    let effective = match scaling {
        Scaling::Difficulty(difficulty) => {
            (u128::from(party_sixths) + 6 * SHIFT) * difficulty.eighths()
        }
        Scaling::Offsets { random, story } => {
            let offset = i64::from(if kind == Kind::Random { random } else { story });
            let sixths = (i64::from(party_sixths) + 6 * offset).clamp(6, 6 * i64::from(MAX_LEVEL));
            (sixths as u128 + 6 * SHIFT) * 8
        }
    };
    Target { effective }
}

impl Target {
    /// The record level an enemy gets: the target level rounded to a whole level, at least 1 and at most [`MAX_LEVEL`].
    /// With the normal difficulty it is the party's mean, rounded.
    #[must_use]
    pub(super) fn level(self) -> u16 {
        let rounded = (self.effective + 24) / 48;
        rounded.saturating_sub(SHIFT).clamp(1, u128::from(MAX_LEVEL)) as u16
    }

    /// The ratio `q` of an enemy made for level `enemy_level` against this target (see the module documentation).
    #[must_use]
    pub(super) fn ratio(self, enemy_level: u8) -> Ratio {
        Ratio { num: self.effective, den: 48 * (u128::from(enemy_level) + SHIFT) }
    }
}

/// The record level of a battle at a difficulty (the tests' shorthand for [`target`]; the kind does not matter to a difficulty).
#[cfg(test)]
pub(super) fn scaled_level(party_sixths: u32, difficulty: Difficulty) -> u16 {
    target(party_sixths, Scaling::Difficulty(difficulty), Kind::Story).level()
}

/// The ratio `q` of one enemy at a difficulty (the tests' shorthand for [`target`]).
#[cfg(test)]
#[must_use]
pub(super) fn ratio(party_sixths: u32, difficulty: Difficulty, enemy_level: u8) -> Ratio {
    target(party_sixths, Scaling::Difficulty(difficulty), Kind::Story).ratio(enemy_level)
}

/// The ratio `q` of one enemy: the target's `u` over the enemy's own `u`, as a fraction of 48ths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ratio {
    num: u128,
    den: u128,
}

impl Ratio {
    /// `value * q^power`, rounded to the nearest whole number (halves up), at least 1 for a value that is not 0, and at
    /// most `max`. At `q = 1` it is `value` itself.
    fn apply(self, value: u64, power: u32, max: u64) -> u64 {
        if value == 0 {
            return 0;
        }
        let (num, den) = (self.num.pow(power), self.den.pow(power));
        let scaled = (u128::from(value) * num + den / 2) / den;
        u64::try_from(scaled).map_or(max, |scaled| scaled.clamp(1, max))
    }
}

/// The HP of an enemy at `q`: `hp * q`, at most 65535.
#[must_use]
pub(super) fn scale_hp(hp: u16, ratio: Ratio) -> u16 {
    ratio.apply(u64::from(hp), 1, MAX_WORD) as u16
}

/// The KI of an enemy at `q`: `ki * q`, at most 65535.
#[must_use]
pub(super) fn scale_ki(ki: u16, ratio: Ratio) -> u16 {
    ratio.apply(u64::from(ki), 1, MAX_WORD) as u16
}

/// The BP of an enemy at `q`: `bp * q^3`, at most 40 bits.
#[must_use]
pub(super) fn scale_bp(bp: u64, ratio: Ratio) -> u64 {
    ratio.apply(bp, 3, MAX_BP)
}

/// The EXP an enemy pays at `q`: `exp * q^2`, at most 24 bits.
#[must_use]
pub(super) fn scale_exp(exp: u32, ratio: Ratio) -> u32 {
    ratio.apply(u64::from(exp), 2, MAX_EXP) as u32
}

/// Whether the block's marker is in place: a neutral block reads as zero.
fn live(ctx: &Ctx<'_>) -> bool {
    MARKER.iter().enumerate().all(|(k, &byte)| ctx.peek8(MARKER_AT + k as u32) == byte)
}

/// Makes the mod state block live as a state without parts is applied (`state::apply_events`): the used bytes and the scratch of
/// the battle hooks are zero, and then the marker is written. The power-on fill of work RAM would be read as high parts
/// otherwise.
fn make_live(ctx: &mut Ctx<'_>) {
    for offset in 0..SCRATCH_END as u32 {
        ctx.write8(BLOCK + offset, 0);
    }
    for (k, &byte) in MARKER.iter().enumerate() {
        ctx.write8(MARKER_AT + k as u32, byte);
    }
}

/// The 24 bits at the bus address `at`, low byte first.
fn peek24(ctx: &Ctx<'_>, at: u32) -> u32 {
    u32::from(ctx.peek16(at)) | u32::from(ctx.peek8(at + 2)) << 16
}

/// Writes the low 24 bits of `value` at the bus address `at`, low byte first.
fn write24(ctx: &mut Ctx<'_>, at: u32, value: u32) {
    ctx.write16(at, value as u16);
    ctx.write8(at + 2, (value >> 16) as u8);
}

/// The party records, as the fighters the party's level is read from.
fn fighters(ctx: &Ctx<'_>) -> Vec<Fighter> {
    (0..PARTY_SLOTS)
        .map(|slot| {
            let base = party_record(slot);
            Fighter {
                flags: ctx.peek8(base + FLAGS),
                unit: ctx.peek8(base + UNIT),
                level: super::levels::effective_level(ctx, base),
                character: ctx.peek8(base + CHARACTER),
            }
        })
        .collect()
}

/// The scaling and the party's level (in sixths) of this fight, or `None` when the scaling does not act: the feature is not in
/// the image, the fight is a scripted loss, or no fighter is in the party.
fn active(ctx: &Ctx<'_>) -> Option<(Scaling, u32)> {
    let scaling = scaling_of(ctx)?;
    if crate::cheats::SCRIPTED_LOSS.contains(&ctx.peek8(GROUP_INDEX)) {
        return None;
    }
    Some((scaling, party_sixths(&fighters(ctx))?))
}

/// The scaling the pack's slot gives, or `None` without the feature in the image. A slot of the difficulty is one byte (the
/// bytes after it belong to the next slot and are not read); a slot of offsets is three.
fn scaling_of(ctx: &Ctx<'_>) -> Option<Scaling> {
    let slot = overlay(ctx, Family::Feature, ENEMY_SCALING)?;
    let mode = ctx.peek8(slot);
    if mode == SCALING_OFFSETS {
        Scaling::from_bytes(mode, ctx.peek8(slot + 1), ctx.peek8(slot + 2))
    } else {
        Scaling::from_bytes(mode, 0, 0)
    }
}

/// `$7F:FB50`: the marker of the kind-of-battle bytes, four bytes.
const KIND_MARKER_AT: u32 = BLOCK + BATTLE_KIND as u32;
/// `$7F:FB54`: 1 between the pick of a random encounter's group and the build of its records, else 0.
const PENDING_AT: u32 = KIND_MARKER_AT + 4;
/// `$7F:FB55`: 1 while the battle whose records were last built is a random encounter, else 0.
const KIND_AT: u32 = KIND_MARKER_AT + 5;

/// Whether the kind-of-battle bytes are in place: without their marker the battle is a story battle.
fn kind_live(ctx: &Ctx<'_>) -> bool {
    KIND_MARKER.iter().enumerate().all(|(k, &byte)| ctx.peek8(KIND_MARKER_AT + k as u32) == byte)
}

/// Puts the kind-of-battle bytes in place (the marker, and both bytes zero) when they are not, because the power-on fill of
/// work RAM is random.
fn make_kind_live(ctx: &mut Ctx<'_>) {
    if !kind_live(ctx) {
        ctx.write8(PENDING_AT, 0);
        ctx.write8(KIND_AT, 0);
        for (k, &byte) in KIND_MARKER.iter().enumerate() {
            ctx.write8(KIND_MARKER_AT + k as u32, byte);
        }
    }
}

/// Whether the pack gives offsets: only then are the two kinds of battle told apart (the difficulty treats them alike, and
/// leaves work RAM alone).
fn with_offsets(ctx: &Ctx<'_>) -> bool {
    matches!(scaling_of(ctx), Some(Scaling::Offsets { .. }))
}

/// `02:D37C`: a random encounter's group has been picked, and the battle it starts has not built its records yet.
fn pick_wrapper(ctx: &mut Ctx<'_>) {
    if with_offsets(ctx) {
        make_kind_live(ctx);
        ctx.write8(PENDING_AT, 1);
    }
    ctx.resume_at(PICK_KEY);
}

/// The kind of the battle whose records are being built: random if the pick left its mark, else a story battle. The mark is
/// taken (a builder run for a form change, or for the next story battle, finds none), and the kind is kept for the EXP award
/// of the same battle ([`stored_kind`]).
fn take_kind(ctx: &mut Ctx<'_>) -> Kind {
    if !with_offsets(ctx) {
        return Kind::Story;
    }
    make_kind_live(ctx);
    let random = ctx.peek8(PENDING_AT) == 1;
    ctx.write8(PENDING_AT, 0);
    ctx.write8(KIND_AT, u8::from(random));
    if random { Kind::Random } else { Kind::Story }
}

/// The kind of the battle whose records were built last.
fn stored_kind(ctx: &Ctx<'_>) -> Kind {
    if kind_live(ctx) && ctx.peek8(KIND_AT) == 1 { Kind::Random } else { Kind::Story }
}

/// `01:F7F1`: the enemy records are complete, and each one `01:F719` filled is scaled: its level, HP and maximum HP, KI and
/// maximum KI, and BP. The loop's own start is mirrored (slot 0 is not visited for an even count), and a slot whose flags are
/// zero, or whose level is zero (a unit row of a party member), is left as it is.
fn stats_wrapper(ctx: &mut Ctx<'_>) {
    let kind = take_kind(ctx);
    if let Some((scaling, party)) = active(ctx) {
        let aim = target(party, scaling, kind);
        let first = u32::from(ctx.peek8(ENEMY_COUNT) & 1 == 0);
        let mut highs = [0_u16; ENEMY_SLOTS as usize];
        let mut shown = Vec::new();
        for slot in first..ENEMY_SLOTS {
            let base = enemy_record(slot);
            let level = ctx.peek8(base + LEVEL);
            if ctx.peek8(base + FLAGS) == 0 || level == 0 {
                continue;
            }
            let ratio = aim.ratio(level);
            let before = (ctx.peek16(base + MAX_HP), peek24(ctx, base + BP));
            for at in [HP, MAX_HP] {
                let scaled = scale_hp(ctx.peek16(base + at), ratio);
                ctx.write16(base + at, scaled);
            }
            for at in [KI, MAX_KI] {
                let scaled = scale_ki(ctx.peek16(base + at), ratio);
                ctx.write16(base + at, scaled);
            }
            let (low, high) = split(scale_bp(u64::from(peek24(ctx, base + BP)), ratio));
            write24(ctx, base + BP, low);
            highs[slot as usize] = high;
            ctx.write8(base + LEVEL, aim.level().min(u16::from(RECORD_LEVEL_MAX)) as u8);
            shown.push(format!(
                "unit {:#04x} L{level}->{} HP {}->{} BP {}->{}",
                ctx.peek8(base + UNIT),
                aim.level(),
                before.0,
                ctx.peek16(base + MAX_HP),
                before.1,
                u64::from(peek24(ctx, base + BP)) | u64::from(high) << 24,
            ));
        }
        log::info!(
            "enemy scaling ({scaling:?}, {kind:?} battle, party level {:.1}): {}",
            f64::from(party) / 6.0,
            shown.join("; ")
        );
        // The high parts: written when the block is live (zero for a BP that fits), and the block made live for the first
        // part that does not. A block that nothing needs is left alone.
        if highs.iter().any(|&high| high != 0) && !live(ctx) {
            make_live(ctx);
        }
        if live(ctx) {
            for (slot, high) in highs.into_iter().enumerate() {
                ctx.write16(BLOCK + (ENEMY_HIGH + 2 * slot) as u32, high);
            }
        }
    }
    ctx.resume_at(STATS_KEY);
}

/// `1D:8C66`: one defeated enemy's reward has been added to the battle's EXP. The original added the unit row's EXP (`X` still
/// holds 14 times the unit); the scaled reward replaces it. Only a unit of the original table is scaled (a pack's unit has no
/// row to read), and a unit whose row has no level is left as it is.
fn exp_wrapper(ctx: &mut Ctx<'_>) {
    if let Some((scaling, party)) = active(ctx) {
        let row = u32::from(ctx.x());
        if row < UNIT_ROWS * UNIT_ROW_LEN && row % UNIT_ROW_LEN == 0 {
            let at = UNIT_STATS + row;
            let level = ctx.peek8(at + ROW_LEVEL);
            if level != 0 {
                let reward = peek24(ctx, at + ROW_EXP);
                let ratio = target(party, scaling, stored_kind(ctx)).ratio(level);
                let total = peek24(ctx, BATTLE_EXP)
                    .wrapping_sub(reward)
                    .wrapping_add(scale_exp(reward, ratio));
                write24(ctx, BATTLE_EXP, total & 0xFF_FFFF);
            }
        }
    }
    ctx.resume_at(EXP_KEY);
}

/// The floor of HP of the unit `unit` as the enemy scaling has scaled its HP: the original's `floor` times the same `q` (at least 1),
/// so that the floor keeps its place in the enemy's HP. `None` where the scaling does not act (the feature is not in the image, the
/// fight is a scripted loss, no fighter is in the party) or the unit has no row of the original table to take the level from.
fn scaled_floor(ctx: &Ctx<'_>, unit: u8, floor: u16) -> Option<u16> {
    let (scaling, party) = active(ctx)?;
    let row = u32::from(unit) * UNIT_ROW_LEN;
    if u32::from(unit) >= UNIT_ROWS {
        return None;
    }
    let level = ctx.peek8(UNIT_STATS + row + ROW_LEVEL);
    if level == 0 {
        return None;
    }
    let ratio = target(party, scaling, stored_kind(ctx)).ratio(level);
    Some(scale_hp(floor, ratio))
}

/// `05:A51D`: the floor at which Zarbon's first form and Frieza's first three forms withdraw (`05:A4F5`: when the HP after the
/// hit is at or below it, the HP becomes the floor and the enemy is gone). `Y` holds the original's floor; it is replaced by the
/// scaled one, which the compare and the store that follow use.
fn floor_wrapper(ctx: &mut Ctx<'_>) {
    let unit = ctx.peek8(SIDE_B + UNIT);
    if let Some(scaled) = scaled_floor(ctx, unit, ctx.y()) {
        ctx.set_y(scaled);
    }
    ctx.resume_at(FLOOR_KEY);
}

/// `05:A46E`: the hit has been taken off the HP of the defender (`Y` is its record). The original lets an enemy of [`FLOORS`] die
/// from a hit that takes it from above its floor to zero or below (the withdrawal is judged only for an enemy that is still
/// alive). With the scaling an enemy has thousands of HP, and one hit may do that: the HP is then set to the scaled floor and the
/// carry set, so that the original goes on as for a hit that left the enemy alive, and `05:A4F5` makes it withdraw.
fn lethal_wrapper(ctx: &mut Ctx<'_>) {
    if ctx.y() == SIDE_B as u16 {
        let unit = ctx.peek8(SIDE_B + UNIT);
        let hp = ctx.peek16(SIDE_B + HP);
        if (!ctx.c() || hp == 0)
            && let Some(&(_, floor)) = FLOORS.iter().find(|&&(floored, _)| floored == unit)
            && let Some(scaled) = scaled_floor(ctx, unit, floor)
        {
            ctx.write16(SIDE_B + HP, scaled);
            ctx.set_c(true);
        }
    }
    ctx.resume_at(LETHAL_KEY);
}

/// `05:A49F`: the Ginyu duel keeps its enemy at 1 HP when a hit would kill him, until the swap. `A` holds that 1: it becomes the
/// scaled 1 (the HP the original leaves him at, times `q`).
fn ginyu_wrapper(ctx: &mut Ctx<'_>) {
    let unit = ctx.peek8(SIDE_B + UNIT);
    if ctx.peek8(GROUP_INDEX) == GINYU_GROUP
        && ctx.peek8(SWAP_STATE) != 2
        && let Some(scaled) = scaled_floor(ctx, unit, ctx.a())
    {
        ctx.set_a(scaled);
    }
    ctx.resume_at(GINYU_KEY);
}

/// `05:9808`: the end of the second form's fight judges the HP of the first enemy: below 21 (the floor of 20 and one) the story
/// goes on, else the generic checks follow. The carry says "21 or more"; it is set from the scaled floor.
fn judge_wrapper(ctx: &mut Ctx<'_>) {
    let slot = enemy_record(0);
    if ctx.peek8(GROUP_INDEX) == SECOND_FORM_GROUP && ctx.peek8(slot + UNIT) == SECOND_FORM_UNIT {
        let floor = FLOORS
            .iter()
            .find(|&&(unit, _)| unit == SECOND_FORM_UNIT)
            .map_or(20, |&(_, floor)| floor);
        if let Some(scaled) = scaled_floor(ctx, SECOND_FORM_UNIT, floor) {
            ctx.set_c(ctx.y() > scaled);
        }
    }
    ctx.resume_at(JUDGE_KEY);
}

/// A `Patch` hook that costs nothing, as every hook of the packs.
fn patch(name: &str, key: u32, func: RoutineFn) -> HookEntry {
    HookEntry {
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
    }
}

/// A `Patch` in the middle of the battle's code, which is run only with the widths of the accumulator and index registers the
/// instruction at `key` is read with (`m8`, and the index always 16 bits there).
fn patch_at(name: &str, key: u32, m8: bool, func: RoutineFn) -> HookEntry {
    HookEntry { entry_m8: Some(m8), entry_x8: Some(false), ..patch(name, key, func) }
}

/// The hooks of `enemy_scaling`: the records of a battle, the EXP of a defeated enemy, the mark of a random encounter, and the four
/// that keep the fights with a floor of HP on the story's path.
pub(super) fn hooks() -> Vec<HookEntry> {
    vec![
        patch("mod_enemy_scaling_stats", STATS_KEY, stats_wrapper),
        patch("mod_enemy_scaling_exp", EXP_KEY, exp_wrapper),
        patch("mod_enemy_scaling_pick", PICK_KEY, pick_wrapper),
        patch_at("mod_enemy_scaling_floor", FLOOR_KEY, true, floor_wrapper),
        patch_at("mod_enemy_scaling_lethal", LETHAL_KEY, true, lethal_wrapper),
        patch_at("mod_enemy_scaling_ginyu", GINYU_KEY, false, ginyu_wrapper),
        patch_at("mod_enemy_scaling_judge", JUDGE_KEY, true, judge_wrapper),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

    /// A fighter in the party at `level`, character `character`.
    fn fighter(character: u8, level: u8) -> Fighter {
        Fighter { flags: 0xC0, unit: character, level: level.into(), character }
    }

    /// An enemy of the kind the original has: level 20, HP 300, KI 100, BP 20 000, EXP 2 000 (the worked example of the design).
    fn example(party: u32, difficulty: Difficulty) -> (u16, u16, u16, u64, u32) {
        let ratio = ratio(party, difficulty, 20);
        (
            scaled_level(party, difficulty),
            scale_hp(300, ratio),
            scale_ki(100, ratio),
            scale_bp(20_000, ratio),
            scale_exp(2_000, ratio),
        )
    }

    #[test]
    fn the_keys_are_the_returns_of_the_routines_named_in_the_documentation() {
        assert_eq!(STATS_KEY, 0x01_F7F1);
        assert_eq!(EXP_KEY, 0x1D_8C66);
        assert_eq!(PICK_KEY, 0x02_D37C);
        assert_eq!(FLOOR_KEY, 0x05_A51D);
        assert_eq!(LETHAL_KEY, 0x05_A46E);
        assert_eq!(GINYU_KEY, 0x05_A49F);
        assert_eq!(JUDGE_KEY, 0x05_9808);
        let names: Vec<String> = hooks().into_iter().map(|entry| entry.name).collect();
        assert_eq!(
            names,
            [
                "mod_enemy_scaling_stats",
                "mod_enemy_scaling_exp",
                "mod_enemy_scaling_pick",
                "mod_enemy_scaling_floor",
                "mod_enemy_scaling_lethal",
                "mod_enemy_scaling_ginyu",
                "mod_enemy_scaling_judge"
            ]
        );
        let cheats = crate::cheats::entry_points();
        for entry in hooks() {
            assert!(!cheats.contains(&entry.key), "{} is a cheat's key", entry.name);
            assert_eq!(entry.mode, HookMode::Patch, "{}", entry.name);
            assert_eq!(entry.cycles, Some(0), "{}", entry.name);
        }
    }

    #[test]
    fn the_difficulty_bytes_are_the_values_of_the_names_in_the_feature() {
        assert_eq!(Difficulty::from_byte(0), Some(Difficulty::Easy));
        assert_eq!(Difficulty::from_byte(1), Some(Difficulty::Normal));
        assert_eq!(Difficulty::from_byte(2), Some(Difficulty::Hard));
        assert_eq!(Difficulty::from_byte(3), None);
    }

    #[test]
    fn the_party_level_is_the_mean_of_the_three_highest_fighters_in_sixths() {
        // Four fighters: the three highest (30, 28, 12) are 70, and the mean 23.33 is 140 sixths.
        let party = [fighter(0, 30), fighter(1, 28), fighter(3, 12), fighter(4, 3)];
        assert_eq!(party_sixths(&party), Some(140));
        assert_eq!(
            party_sixths(&[fighter(0, 10), fighter(1, 10)]),
            Some(60),
            "two fighters: their mean"
        );
        assert_eq!(party_sixths(&[fighter(0, 47)]), Some(282), "one fighter: his level");
        assert_eq!(
            party_sixths(&[fighter(0, 5), fighter(1, 5), fighter(2, 6)]),
            Some(32),
            "5.33 levels"
        );
        assert_eq!(party_sixths(&[]), None, "no party, no level");
    }

    #[test]
    fn a_fighter_who_is_down_counts_and_a_record_outside_the_party_does_not() {
        let mut down = fighter(0, 30);
        down.flags = 0x80;
        let mut absent = fighter(1, 40);
        absent.flags = 0;
        let mut nobody = fighter(2, 0);
        nobody.flags = 0xC0;
        assert_eq!(party_sixths(&[down, absent, nobody, fighter(3, 20)]), Some(150), "30 and 20");
    }

    #[test]
    fn the_support_characters_and_the_temporary_unit_leave_the_level_alone_unless_they_are_all_there_is()
     {
        let party = [fighter(0, 40), fighter(11, 3), fighter(8, 25)];
        assert_eq!(party_sixths(&party), Some(240), "Dende and Nail do not pull the mean down");
        let mut borrowed = fighter(7, 17);
        borrowed.unit = TEMPORARY_UNIT;
        assert_eq!(
            party_sixths(&[fighter(0, 40), borrowed]),
            Some(240),
            "the borrowed record is left out"
        );
        assert_eq!(
            party_sixths(&[fighter(8, 25), fighter(9, 15), fighter(10, 15)]),
            Some(110),
            "a party of the characters without levels: the mean of what they are"
        );
        assert_eq!(party_sixths(&[fighter(12, 9)]), Some(54), "character 12 alone");
        assert_eq!(
            party_sixths(&[fighter(13, 9), fighter(11, 3)]),
            Some(54),
            "a pack's character counts"
        );
    }

    #[test]
    fn at_the_enemys_own_level_and_the_normal_difficulty_every_number_is_the_originals() {
        for level in 1..=60_u8 {
            let party = 6 * u32::from(level);
            let ratio = ratio(party, Difficulty::Normal, level);
            for hp in [0_u16, 1, 7, 55, 300, 2_000, 65_535] {
                assert_eq!(scale_hp(hp, ratio), hp, "HP {hp} at level {level}");
                assert_eq!(scale_ki(hp, ratio), hp, "KI {hp} at level {level}");
            }
            for bp in [0_u64, 1, 3, 441, 20_000, 1_550_000, 10_000_000, 16_777_215] {
                assert_eq!(scale_bp(bp, ratio), bp, "BP {bp} at level {level}");
            }
            for exp in [0_u32, 1, 9, 2_000, 50_000, 16_777_215] {
                assert_eq!(scale_exp(exp, ratio), exp, "EXP {exp} at level {level}");
            }
            assert_eq!(
                scaled_level(party, Difficulty::Normal),
                u16::from(level),
                "the record's level"
            );
        }
    }

    #[test]
    fn the_identity_holds_for_a_party_whose_mean_is_the_enemys_level_whatever_its_levels() {
        // (4, 5, 6) and (3, 5, 7) and (5, 5, 5) all have the mean 5.
        for levels in [[4_u8, 5, 6], [3, 5, 7], [5, 5, 5]] {
            let party: Vec<Fighter> =
                levels.iter().enumerate().map(|(k, &level)| fighter(k as u8, level)).collect();
            let sixths = party_sixths(&party).unwrap();
            assert_eq!(sixths, 30);
            let ratio = ratio(sixths, Difficulty::Normal, 5);
            assert_eq!(
                (scale_hp(200, ratio), scale_bp(1_500, ratio), scale_exp(250, ratio)),
                (200, 1_500, 250)
            );
        }
    }

    #[test]
    fn the_worked_example_of_the_design_gives_its_numbers() {
        // The table of docs/mods/free-progression.md: level 20, HP 300, KI 100, BP 20 000, EXP 2 000 at the normal difficulty.
        let expected = [
            (1, (1, 72, 24, 276, 115)),
            (5, (5, 120, 40, 1_280, 320)),
            (10, (10, 180, 60, 4_320, 720)),
            (20, (20, 300, 100, 20_000, 2_000)),
            (30, (30, 420, 140, 54_880, 3_920)),
            (40, (40, 540, 180, 116_640, 6_480)),
            (47, (47, 624, 208, 179_978, 8_653)),
        ];
        for (party, numbers) in expected {
            assert_eq!(example(6 * party, Difficulty::Normal), numbers, "party level {party}");
        }
    }

    #[test]
    fn each_difficulty_is_its_factor_on_the_partys_level_and_the_numbers_are_ordered() {
        // At the enemy's own level: BP times (7/8)^3 and (9/8)^3, HP and KI times 7/8 and 9/8, EXP times the squares.
        assert_eq!(example(120, Difficulty::Easy), (17, 263, 88, 13_398, 1_531));
        assert_eq!(example(120, Difficulty::Normal), (20, 300, 100, 20_000, 2_000));
        assert_eq!(example(120, Difficulty::Hard), (23, 338, 113, 28_477, 2_531));
        for party in [6, 30, 120, 282] {
            let easy = example(party, Difficulty::Easy);
            let normal = example(party, Difficulty::Normal);
            let hard = example(party, Difficulty::Hard);
            assert!(easy.1 <= normal.1 && normal.1 <= hard.1, "HP at {party}");
            assert!(easy.3 <= normal.3 && normal.3 <= hard.3, "BP at {party}");
            assert!(easy.4 <= normal.4 && normal.4 <= hard.4, "EXP at {party}");
            assert!(easy.0 <= normal.0 && normal.0 <= hard.0, "level at {party}");
        }
    }

    #[test]
    fn the_bp_factors_of_the_designs_second_table_are_the_ones_the_law_gives() {
        // docs/mods/free-progression.md: the BP factor `q^3` of an enemy made for level L against a party of level P, normal.
        let parties = [1_u32, 5, 10, 20, 30, 40, 47];
        let table: [(u8, [f64; 7]); 3] = [
            (5, [0.22, 1.0, 3.4, 16.0, 43.0, 91.0, 140.0]),
            (20, [0.014, 0.064, 0.22, 1.0, 2.7, 5.8, 9.0]),
            (40, [0.0024, 0.011, 0.037, 0.17, 0.47, 1.0, 1.5]),
        ];
        for (enemy_level, factors) in table {
            for (&party, &factor) in parties.iter().zip(&factors) {
                let ratio = ratio(6 * party, Difficulty::Normal, enemy_level);
                let actual = scale_bp(1_000_000_000, ratio) as f64 / 1e9;
                assert!(
                    (actual - factor).abs() <= factor * 0.05,
                    "enemy level {enemy_level}, party {party}: the law gives {actual}, the table {factor}"
                );
            }
        }
    }

    #[test]
    fn every_number_grows_with_the_partys_level_and_never_falls_for_any_difficulty() {
        for difficulty in ALL {
            for enemy_level in [1_u8, 2, 5, 20, 48, 100] {
                let mut last = (0_u16, 0_u16, 0_u64, 0_u32, 0_u16);
                for party in 6..=600_u32 {
                    let ratio = ratio(party, difficulty, enemy_level);
                    let now = (
                        scale_hp(300, ratio),
                        scale_ki(100, ratio),
                        scale_bp(20_000, ratio),
                        scale_exp(2_000, ratio),
                        scaled_level(party, difficulty),
                    );
                    assert!(
                        now.0 >= last.0
                            && now.1 >= last.1
                            && now.2 >= last.2
                            && now.3 >= last.3
                            && now.4 >= last.4,
                        "{difficulty:?}, enemy level {enemy_level}, party {party}: {now:?} after {last:?}"
                    );
                    last = now;
                }
            }
        }
    }

    #[test]
    fn a_weaker_party_weakens_the_enemy_with_no_floor_and_a_stronger_one_strengthens_it() {
        let low = example(6, Difficulty::Normal);
        let original = example(120, Difficulty::Normal);
        let high = example(282, Difficulty::Normal);
        assert!(
            low.1 < original.1 && low.3 < original.3 && low.4 < original.4,
            "weaker than the original"
        );
        assert!(
            high.1 > original.1 && high.3 > original.3 && high.4 > original.4,
            "stronger than the original"
        );
        // No floor at the original's numbers, but a floor at one: a nonzero number never reaches zero.
        let tiny = ratio(6, Difficulty::Easy, 100);
        assert_eq!(
            (scale_hp(1, tiny), scale_ki(1, tiny), scale_bp(1, tiny), scale_exp(1, tiny)),
            (1, 1, 1, 1)
        );
        assert_eq!(
            (scale_hp(0, tiny), scale_bp(0, tiny), scale_exp(0, tiny)),
            (0, 0, 0),
            "zero stays zero"
        );
    }

    #[test]
    fn the_numbers_saturate_at_the_widths_they_are_kept_in() {
        let huge = ratio(600, Difficulty::Hard, 1);
        assert_eq!(scale_hp(65_535, huge), 65_535, "HP and KI are 16 bits");
        assert_eq!(scale_ki(65_535, huge), 65_535);
        assert_eq!(scale_exp(16_777_215, huge), 16_777_215, "EXP is 24 bits");
        let bp = scale_bp(16_777_215, huge);
        assert!(bp > 0xFF_FFFF && bp < MAX_BP, "a full 24-bit BP grows into the wide part: {bp}");
        assert_eq!(scale_bp(MAX_BP, huge), MAX_BP, "BP is 40 bits");
        assert_eq!(
            scaled_level(600, Difficulty::Hard),
            113,
            "the record's level at party level 100, hard"
        );
        assert_eq!(scaled_level(6 * 1000, Difficulty::Hard), MAX_LEVEL, "the level is capped");
        assert_eq!(scaled_level(6, Difficulty::Easy), 1, "and never below 1");
    }

    #[test]
    fn a_bp_past_24_bits_splits_into_the_record_and_the_blocks_high_part_and_back() {
        let ratio = ratio(600, Difficulty::Normal, 48);
        let bp = scale_bp(3_000_000, ratio);
        assert!(
            bp > 0xFF_FFFF,
            "Frieza's last form against a level 100 party needs the wide part: {bp}"
        );
        let (low, high) = split(bp);
        assert_eq!(super::super::state::wide(low, high), bp);
        assert_ne!(high, 0);
    }

    #[test]
    fn the_scaled_party_to_enemy_ratio_stays_close_to_the_original_while_the_unscaled_one_does_not()
    {
        // A BP curve for the party that grows as (level + 5)^3 (the curve the model is fitted to): the ratio of party to scaled
        // enemy is then the same at every level, while the same enemy unscaled falls behind by the cube of the levels.
        let party_bp = |level: u32| u64::from((level + 5).pow(3)) * 4;
        let enemy_level = 20_u8;
        let enemy_bp = party_bp(20) * 2;
        let at_own_level = party_bp(20) as f64 / enemy_bp as f64;
        for level in [5_u32, 10, 20, 30, 47] {
            let scaled = scale_bp(enemy_bp, ratio(6 * level, Difficulty::Normal, enemy_level));
            let ratio_now = party_bp(level) as f64 / scaled as f64;
            assert!((ratio_now / at_own_level - 1.0).abs() < 0.01, "level {level}: {ratio_now}");
        }
        let unscaled = party_bp(47) as f64 / enemy_bp as f64;
        assert!(
            unscaled / at_own_level > 8.0,
            "unscaled the same party is eight times stronger relative to the enemy"
        );
    }

    /// The offsets `(random, story)` as a scaling.
    fn offsets(random: i8, story: i8) -> Scaling {
        Scaling::Offsets { random, story }
    }

    /// The levels and BP of the level-20 example enemy at a target.
    fn example_at(party: u32, scaling: Scaling, kind: Kind) -> (u16, u16, u64) {
        let aim = target(party, scaling, kind);
        let ratio = aim.ratio(20);
        (aim.level(), scale_hp(300, ratio), scale_bp(20_000, ratio))
    }

    #[test]
    fn the_offsets_put_a_random_battle_and_a_story_battle_at_their_own_levels_from_the_partys() {
        // Random at -3 and story at +3: the levels the owner asked for, at a party of level 46.
        let scaling = offsets(-3, 3);
        let party = 6 * 46;
        assert_eq!(target(party, scaling, Kind::Random).level(), 43);
        assert_eq!(target(party, scaling, Kind::Story).level(), 49);
        for level in [1_u32, 5, 12, 20, 46, 100] {
            let party = 6 * level;
            let low = target(party, scaling, Kind::Random).level();
            let high = target(party, scaling, Kind::Story).level();
            let want = |offset: i64| (i64::from(level) + offset).clamp(1, 999) as u16;
            assert_eq!((low, high), (want(-3), want(3)), "party level {level}");
        }
        // The offset of the other kind does not leak in.
        assert_eq!(target(party, offsets(-3, 9), Kind::Random).level(), 43);
        assert_eq!(target(party, offsets(9, 3), Kind::Story).level(), 49);
        // A fractional party level (the mean of three) is rounded after the offset: 5.33 and +2 is 7.33, level 7.
        assert_eq!(target(32, offsets(2, 2), Kind::Random).level(), 7);
        assert_eq!(target(34, offsets(2, 2), Kind::Random).level(), 8, "5.67 + 2 rounds up");
    }

    #[test]
    fn with_offsets_the_numbers_are_the_enemys_made_for_the_target_level() {
        // An enemy of level 20 against a party of level 20: offset 0 is the original, +3 is the numbers of a party of 23, -3 of 17.
        let party = 6 * 20;
        let original = example_at(party, Scaling::Difficulty(Difficulty::Normal), Kind::Story);
        assert_eq!(original, (20, 300, 20_000));
        assert_eq!(example_at(party, offsets(0, 0), Kind::Random), original);
        assert_eq!(example_at(party, offsets(0, 0), Kind::Story), original);
        for (offset, kind) in [(3_i8, Kind::Story), (-3, Kind::Random)] {
            let level = (20 + i32::from(offset)) as u32;
            let by_party = example_at(6 * level, Scaling::Difficulty(Difficulty::Normal), kind);
            assert_eq!(
                example_at(party, offsets(offset, offset), kind),
                by_party,
                "offset {offset} is the numbers of a party of level {level}"
            );
        }
        // Monotonic in the offset.
        let mut last = (0_u16, 0_u16, 0_u64);
        for offset in -10..=10_i8 {
            let now = example_at(party, offsets(offset, offset), Kind::Story);
            assert!(now.0 > last.0 && now.1 >= last.1 && now.2 >= last.2, "offset {offset}");
            last = now;
        }
    }

    #[test]
    fn the_offsets_keep_the_clamps_of_one_and_the_level_cap() {
        // A party of level 1 and -10: the target is level 1, not a level at or below 0, and q is the one of level 1.
        let aim = target(6, offsets(-10, -10), Kind::Random);
        assert_eq!(aim.level(), 1);
        assert_eq!(
            aim.ratio(10),
            target(6, Scaling::Difficulty(Difficulty::Normal), Kind::Story).ratio(10)
        );
        assert_eq!(target(6 * 7, offsets(-10, -10), Kind::Story).level(), 1, "7 - 10 is below 1");
        assert_eq!(target(6 * 11, offsets(-10, -10), Kind::Story).level(), 1);
        assert_eq!(target(6 * 12, offsets(-10, -10), Kind::Story).level(), 2);
        // The top: a party of level 995 and +10 would be 1005.
        assert_eq!(target(6 * 995, offsets(10, 10), Kind::Story).level(), MAX_LEVEL);
        assert_eq!(target(6 * 1200, offsets(10, 10), Kind::Story).level(), MAX_LEVEL);
        assert_eq!(
            target(6 * 100, offsets(10, 10), Kind::Story).level(),
            110,
            "no clamp at 200 any more"
        );
        let top = target(6 * 1200, offsets(10, 10), Kind::Story).ratio(20);
        let at_cap = target(6 * 999, offsets(0, 0), Kind::Story).ratio(20);
        assert_eq!(top, at_cap, "q follows the clamped level too");
        // Never a number of 0 for a nonzero value, whatever the offset.
        let tiny = target(6, offsets(-10, -10), Kind::Random).ratio(100);
        assert_eq!((scale_hp(1, tiny), scale_bp(1, tiny), scale_exp(1, tiny)), (1, 1, 1));
    }

    #[test]
    fn a_slot_of_the_difficulty_alone_is_the_old_scaling_and_a_slot_of_offsets_the_new_one() {
        // The slot of the released pack 0.2.0: one byte, the difficulty. Whatever follows it is not read.
        for (byte, difficulty) in
            [(0, Difficulty::Easy), (1, Difficulty::Normal), (2, Difficulty::Hard)]
        {
            for next in [0_u8, 0xFD, 3, 0xFF] {
                assert_eq!(
                    Scaling::from_bytes(byte, next, next),
                    Some(Scaling::Difficulty(difficulty))
                );
            }
        }
        assert_eq!(Scaling::from_bytes(SCALING_OFFSETS, 0xFD, 3), Some(offsets(-3, 3)));
        assert_eq!(Scaling::from_bytes(SCALING_OFFSETS, 10, 0xF6), Some(offsets(10, -10)));
        assert_eq!(
            Scaling::from_bytes(4, 0, 0),
            None,
            "a byte that is no mode: the feature is off"
        );
        // For the old form the kind of battle changes nothing, and it is the difficulty's own law.
        for party in [6_u32, 100, 282] {
            for difficulty in ALL {
                let story = target(party, Scaling::Difficulty(difficulty), Kind::Story);
                assert_eq!(story, target(party, Scaling::Difficulty(difficulty), Kind::Random));
                assert_eq!(story.level(), scaled_level(party, difficulty));
                assert_eq!(story.ratio(20), ratio(party, difficulty, 20));
            }
        }
    }
}

#[cfg(test)]
mod rom;
