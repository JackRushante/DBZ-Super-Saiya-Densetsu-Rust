//! The enemies that follow the party's level (M10, Task 19): the named feature `enemy_scaling`. The design and its numbers are
//! in `docs/mods/free-progression.md`; the parameter is in `docs/mods/features.md`. In short, every enemy of a battle is set to
//! the strength it would have if it had been made for the party's level, on the curve the original gives its own enemies and
//! the party's levels, and the difficulty moves that level up or down.
//!
//! **The model** (all integer arithmetic, so that a replay plays the same on every machine). With `u = level + 5`:
//! - the party's level `P` is the mean of the levels of its three highest fighters ([`party_sixths`]), kept in sixths;
//! - the difficulty is a factor on the party's `u`: 7/8, 1 or 9/8;
//! - the enemy's own level `L` is the level of its record (the unit row's, or the pack row's), the level the enemy was made for;
//! - `q = difficulty * (P + 5) / (L + 5)`; then HP and KI are scaled by `q`, BP by `q^3`, the EXP it pays by `q^2`, and the level
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
//! A hook declines at once (it only resumes at its own key) without the feature in the image, in a fight of the scripted-loss
//! list ([`crate::cheats::SCRIPTED_LOSS`]), and without a fighter in the party. A BP above 24 bits is kept as the wide BP of
//! `super::state` keeps it: the low 24 bits in the record and the high 16 in the enemy's part of the mod state block, which the
//! hook makes live when it must (the block is left alone while every BP fits 24 bits).

use super::redirect::overlay;
use super::state::{
    BLOCK, ENEMY_HIGH, ENEMY_SLOTS, MARKER, MARKER_AT, PARTY_SLOTS, SCRATCH_END, enemy_record,
    party_record, split,
};
use ssd_mod::Family;
use ssd_mod::feature::ENEMY_SCALING;
use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, ReturnKind, RoutineFn};

/// `01:F7F1`: the `RTL` of `01:F719`, which the loop over the five enemy slots reaches when it is done.
pub(super) const STATS_KEY: u32 = 0x01_F7F1;
/// `1D:8C66`: the `RTL` of `1D:8C33`, after the reward of one defeated enemy is added to the battle's EXP.
pub(super) const EXP_KEY: u32 = 0x1D_8C66;

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

/// The shift of the levels in the model: `u = level + 5` (see the module documentation).
const SHIFT: u128 = 5;
/// The highest level an enemy's record is given. The special attack's roll adds `10 + level + a byte` in 8 bits, so a level
/// above this could wrap it.
const MAX_LEVEL: u8 = 200;
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

/// One party record, as the party's level is read from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Fighter {
    pub(super) flags: u8,
    pub(super) unit: u8,
    pub(super) level: u8,
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
    let mut levels: Vec<u8> = fighters
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

/// The party's `u` with the difficulty, in 48ths: `(P + 5) * difficulty` with `P` in sixths (so `(sixths + 30) * eighths`).
fn effective(party_sixths: u32, difficulty: Difficulty) -> u128 {
    (u128::from(party_sixths) + 6 * SHIFT) * difficulty.eighths()
}

/// The record level an enemy gets: the party's level with the difficulty, `difficulty * (P + 5) - 5` rounded to a whole
/// level, at least 1 and at most [`MAX_LEVEL`]. At the normal difficulty it is the party's mean, rounded.
#[must_use]
pub(super) fn scaled_level(party_sixths: u32, difficulty: Difficulty) -> u8 {
    let rounded = (effective(party_sixths, difficulty) + 24) / 48;
    rounded.saturating_sub(SHIFT).clamp(1, u128::from(MAX_LEVEL)) as u8
}

/// The ratio `q` of one enemy: the party's `u` with the difficulty over the enemy's own `u`, as a fraction of 48ths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ratio {
    num: u128,
    den: u128,
}

/// The ratio of an enemy made for level `enemy_level` against the party (see the module documentation).
#[must_use]
pub(super) fn ratio(party_sixths: u32, difficulty: Difficulty, enemy_level: u8) -> Ratio {
    Ratio { num: effective(party_sixths, difficulty), den: 48 * (u128::from(enemy_level) + SHIFT) }
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
                level: ctx.peek8(base + LEVEL),
                character: ctx.peek8(base + CHARACTER),
            }
        })
        .collect()
}

/// The difficulty and the party's level (in sixths) of this fight, or `None` when the scaling does not act: the feature is not in
/// the image, the fight is a scripted loss, or no fighter is in the party.
fn active(ctx: &Ctx<'_>) -> Option<(Difficulty, u32)> {
    let slot = overlay(ctx, Family::Feature, ENEMY_SCALING)?;
    let difficulty = Difficulty::from_byte(ctx.peek8(slot))?;
    if crate::cheats::SCRIPTED_LOSS.contains(&ctx.peek8(GROUP_INDEX)) {
        return None;
    }
    Some((difficulty, party_sixths(&fighters(ctx))?))
}

/// `01:F7F1`: the enemy records are complete, and each one `01:F719` filled is scaled: its level, HP and maximum HP, KI and
/// maximum KI, and BP. The loop's own start is mirrored (slot 0 is not visited for an even count), and a slot whose flags are
/// zero, or whose level is zero (a unit row of a party member), is left as it is.
fn stats_wrapper(ctx: &mut Ctx<'_>) {
    if let Some((difficulty, party)) = active(ctx) {
        let first = u32::from(ctx.peek8(ENEMY_COUNT) & 1 == 0);
        let mut highs = [0_u16; ENEMY_SLOTS as usize];
        for slot in first..ENEMY_SLOTS {
            let base = enemy_record(slot);
            let level = ctx.peek8(base + LEVEL);
            if ctx.peek8(base + FLAGS) == 0 || level == 0 {
                continue;
            }
            let ratio = ratio(party, difficulty, level);
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
            ctx.write8(base + LEVEL, scaled_level(party, difficulty));
        }
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
    if let Some((difficulty, party)) = active(ctx) {
        let row = u32::from(ctx.x());
        if row < UNIT_ROWS * UNIT_ROW_LEN && row % UNIT_ROW_LEN == 0 {
            let at = UNIT_STATS + row;
            let level = ctx.peek8(at + ROW_LEVEL);
            if level != 0 {
                let reward = peek24(ctx, at + ROW_EXP);
                let scaled = scale_exp(reward, ratio(party, difficulty, level));
                let total = peek24(ctx, BATTLE_EXP).wrapping_sub(reward).wrapping_add(scaled);
                write24(ctx, BATTLE_EXP, total & 0xFF_FFFF);
            }
        }
    }
    ctx.resume_at(EXP_KEY);
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

/// The hooks of `enemy_scaling`: the records of a battle and the EXP of a defeated enemy.
pub(super) fn hooks() -> Vec<HookEntry> {
    vec![
        patch("mod_enemy_scaling_stats", STATS_KEY, stats_wrapper),
        patch("mod_enemy_scaling_exp", EXP_KEY, exp_wrapper),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

    /// A fighter in the party at `level`, character `character`.
    fn fighter(character: u8, level: u8) -> Fighter {
        Fighter { flags: 0xC0, unit: character, level, character }
    }

    /// An enemy of the kind the original has: level 20, HP 300, KI 100, BP 20 000, EXP 2 000 (the worked example of the design).
    fn example(party: u32, difficulty: Difficulty) -> (u8, u16, u16, u64, u32) {
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
    fn the_keys_are_the_returns_of_the_two_routines_named_in_the_documentation() {
        assert_eq!(STATS_KEY, 0x01_F7F1);
        assert_eq!(EXP_KEY, 0x1D_8C66);
        let names: Vec<String> = hooks().into_iter().map(|entry| entry.name).collect();
        assert_eq!(names, ["mod_enemy_scaling_stats", "mod_enemy_scaling_exp"]);
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
            assert_eq!(scaled_level(party, Difficulty::Normal), level, "the record's level");
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
                let mut last = (0_u16, 0_u16, 0_u64, 0_u32, 0_u8);
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
        assert_eq!(scaled_level(6 * 255, Difficulty::Hard), MAX_LEVEL, "the level is capped");
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
}

#[cfg(test)]
mod rom;
