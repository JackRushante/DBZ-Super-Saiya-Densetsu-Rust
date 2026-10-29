//! The feature `free_progression` (`ssd_mod::feature::FREE_PROGRESSION`): the story moves on events, not on the level of the party.
//! The gates are listed, with what each one gates and why it is replaced or kept, in `docs/re/data/free-progression.md`; the
//! hooks here are the replacements. Every hook is a `Patch` that reads the feature's slot of `Family::Feature` and declines at
//! once (`resume_at` its own key) when the slot is not in the image, so a run without the pack is the original's.
//!
//! How a level compare is replaced: the story loads a party member's level into A and compares it with a constant. A hook on the
//! load (or on the compare that follows it) puts into A the number that makes the compare come out as the story's event says,
//! and the original goes on from there, so every branch after the compare is the original's own code.
//!
//! - **Raditz** (`03:F22C`, `03:F239`, in the map 0 routine `03:F212`): the original starts the forced fight when Goku is at
//!   level 5 (or Piccolo at 4) with flag 3 set (the Gohan card) and flag `0x0B` clear. The feature starts it when flag `0x0A` is
//!   set instead: the imposter in the cave is beaten, which is the last thing the opening leads to.
//! - **Random battles** (`02:D477`, maps 0 and 2): the original stops them at the same levels. The feature never stops them.
//! - **The recruits' deadline** (`03:F30B`, `03:F31B`, in `03:F300`): the original kills the recruits not yet found when Piccolo
//!   reaches level 9 (Gohan 10 without him). The feature makes both compares fail. The Remastered story fix does the same at
//!   `03:F320`; these two keys make the feature complete without it.
//! - **Super Saiyan** (`05:8794` in the battle state that starts the transformation, `03:868E` in the builder's guard `03:8683`):
//!   the original asks for Goku at level 47. The feature asks for the rest only (the group of the last fight, a friend who has
//!   fallen, the party): the level it compares is raised to 47 if it is lower.
//! - **Goku's arrival in the Ginyu squad's chapter** (`03:EDF6`, in the map 16 set-up): the original reads Goku's level to set one
//!   of three flags (`0x10` to `0x12`), his arrival during the first fight, the second or the last. The feature gives the arrival
//!   its parameter says (`ginyu_arrival`: 1 to 3; 0 is the original's rule).
//! - **Piccolo's return** (`03:C59C`, `03:C5A7`, in effect `$35`): the original gives Piccolo the BP of a table indexed by his level
//!   minus 33, which holds the levels 33 to 40 only. At another level the feature leaves his BP as it is.
//!
//! What the feature leaves as the original has it, and why, is in the same note: the floors and gifts only raise a level, Dende's
//! healing at level 3 and Gohan's rage at level 11 are abilities of a fighter and the Auto choice and the counter-attack compare the
//! BP of both sides in the battle, so none of them moves the story.

use super::redirect::overlay;
use ssd_mod::Family;
use ssd_mod::feature::{FREE_PROGRESSION, LEVEL_CAP_SET};
use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, ReturnKind, RoutineFn};

/// `03:F22C`: the `LDA $0B02,Y` of Goku's level in the map 0 routine (`03:F212`); the `CMP #$05` follows it.
pub(super) const RADITZ_GOKU_KEY: u32 = 0x03_F22C;
/// `03:F239`: the `LDA $0B02,Y` of Piccolo's level in the same routine; the `CMP #$04` follows it.
pub(super) const RADITZ_PICCOLO_KEY: u32 = 0x03_F239;
/// The length of those loads (`LDA abs,Y`).
const LOAD_ABS_LEN: u32 = 3;
/// The levels the original's compares pass at: Goku 5 (`BCS`), Piccolo 4 (`BCC` to leave).
const RADITZ_GOKU_LEVEL: u8 = 5;
const RADITZ_PICCOLO_LEVEL: u8 = 4;
/// Story flag `0x0A` (`$0171` bit 2): the imposter Raditz is beaten (scene `$03` sets it). Flag `0x0B` (bit 3) is the forced
/// fight's own and flag 3 (`$0170` bit 3) the Gohan card; the original's routine tests both before it reads a level.
const IMPOSTER_BEATEN: (u32, u8) = (0x7E_0171, 0x04);

/// `02:D477`: the entry of the stop of the random battles of maps 0 and 2, reached by `jmp` from `02:D428`; it ends with `rtl`.
pub(super) const ENCOUNTER_KEY: u32 = 0x02_D477;

/// `03:F30B` and `03:F31B`: the `CMP #$09` of Piccolo's level and the `CMP #$0A` of Gohan's, in the level check of map 5.
pub(super) const DEADLINE_PICCOLO_KEY: u32 = 0x03_F30B;
pub(super) const DEADLINE_GOHAN_KEY: u32 = 0x03_F31B;

/// `05:8794`: the `CMP #$2F` after the `LDA $0002,Y` of the level of the acting record, in the battle state that starts Goku's
/// transformation (group `$56` or `$6E`, a friend fallen: `$14D2`). `03:868E`: the `CMP #$2F` of `03:8683`, which builds the form.
pub(super) const SSJ_TRIGGER_KEY: u32 = 0x05_8794;
pub(super) const SSJ_BUILD_KEY: u32 = 0x03_868E;
/// The level both compare with (47, Goku's cap).
const SSJ_LEVEL: u8 = 0x2F;

/// `03:EDF6`: the `CMP #$22` after the `LDA $7E2802` of Goku's level in the roster image, in the map 16 set-up (`03:ED9C`).
pub(super) const GINYU_KEY: u32 = 0x03_EDF6;
/// The levels that give the three arrivals in the original's rule: below 34, 34 and 35, 36 and more.
const GINYU_LEVELS: [u8; 3] = [0x20, 0x22, 0x24];

/// `03:C59C`: the `LDA $03C5BA,X` of the low word of the BP in effect `$35`; `03:C5A7`: the `LDA $03C5BC,X` of its high byte.
/// Both are 4 bytes (`LDA long,X`) and read the table at the index the level makes.
pub(super) const PICCOLO_BP_LOW_KEY: u32 = 0x03_C59C;
pub(super) const PICCOLO_BP_HIGH_KEY: u32 = 0x03_C5A7;
const LOAD_LONG_LEN: u32 = 4;
/// The first party record (`$0B00`): Piccolo, as effect `$35` has just made the party. His level is at `+02`, his BP at `+07`.
const FIRST_RECORD: u32 = 0x7E_0B00;
const RECORD_LEVEL: u32 = 2;
const RECORD_BP: u32 = 7;
/// The levels the BP table holds: from 33, eight entries.
const PICCOLO_TABLE_LEVELS: std::ops::RangeInclusive<u8> = 33..=40;

/// The parameter of the feature in the image (`ginyu_arrival`), or `None` when the pack does not enable it. A slot with a
/// `level_cap` begins with [`LEVEL_CAP_SET`], then the arrival, then the cap as two bytes.
fn feature(ctx: &Ctx<'_>) -> Option<u8> {
    let at = overlay(ctx, Family::Feature, FREE_PROGRESSION)?;
    let first = ctx.peek8(at);
    Some(if first == LEVEL_CAP_SET { ctx.peek8(at + 1) } else { first })
}

/// The level cap of the pack (`level_cap`, above the game's 48), or `None` for a pack without it or without the feature.
pub(super) fn level_cap(ctx: &Ctx<'_>) -> Option<u16> {
    level_cap_in(|address| ctx.peek8(address))
}

/// The level cap of the pack in the image `read` reads (a bus reader, as [`ssd_mod::find`] takes it), or `None` for an image
/// without a pack that sets it. The game's hooks read it from the machine's memory ([`level_cap`]); the application reads the
/// same bytes for the save editor and the HD Status screen.
#[must_use]
pub fn level_cap_in(read: impl Fn(u32) -> u8) -> Option<u16> {
    let at = ssd_mod::find(&read, Family::Feature, FREE_PROGRESSION)?;
    (read(at) == LEVEL_CAP_SET)
        .then(|| u16::from(read(at + 2)) | u16::from(read(at + 3)) << 8)
        .filter(|&cap| cap > 48)
        .map(|cap| cap.min(ssd_menus::levels::LEVEL_MAX))
}

/// Whether the imposter Raditz is beaten (story flag `0x0A`).
fn imposter_beaten(ctx: &Ctx<'_>) -> bool {
    ctx.peek8(IMPOSTER_BEATEN.0) & IMPOSTER_BEATEN.1 != 0
}

/// The level a compare of the Raditz routine is given: the level it passes at when the story's event holds, 0 when it does not.
#[must_use]
fn raditz_level(event: bool, passes_at: u8) -> u8 {
    if event { passes_at } else { 0 }
}

/// The level a compare of the Super Saiyan's guards sees: the party's level, or 47 if it is lower.
#[must_use]
fn super_saiyan_level(level: u8) -> u8 {
    level.max(SSJ_LEVEL)
}

/// The level of Goku the map 16 set-up is given for the arrival `arrival` (1 to 3), or `None` for 0, the original's rule.
#[must_use]
fn ginyu_level(arrival: u8) -> Option<u8> {
    GINYU_LEVELS.get(usize::from(arrival).checked_sub(1)?).copied()
}

/// Whether the BP table of effect `$35` has an entry for `level` (33 to 40).
#[must_use]
fn piccolo_table_has(level: u8) -> bool {
    PICCOLO_TABLE_LEVELS.contains(&level)
}

/// The load of a level in the Raditz routine: with Goku (or Piccolo) found in the party (carry clear after `03:F021`), A gets the
/// level that makes the compare pass when the imposter is beaten and fail when it is not, and the routine goes on after the load.
fn raditz(ctx: &mut Ctx<'_>, key: u32, passes_at: u8) {
    if feature(ctx).is_some() && !ctx.c() {
        let level = raditz_level(imposter_beaten(ctx), passes_at);
        ctx.set_al(level);
        ctx.set_nz8(level);
        ctx.resume_at(key + LOAD_ABS_LEN);
    } else {
        ctx.resume_at(key);
    }
}

/// `03:F22C`, `LDA $0B02,Y` (Goku's level): replaced, resumes after it. A is 8-bit.
fn raditz_goku(ctx: &mut Ctx<'_>) {
    raditz(ctx, RADITZ_GOKU_KEY, RADITZ_GOKU_LEVEL);
}

/// `03:F239`, `LDA $0B02,Y` (Piccolo's level): replaced, resumes after it. A is 8-bit.
fn raditz_piccolo(ctx: &mut Ctx<'_>) {
    raditz(ctx, RADITZ_PICCOLO_KEY, RADITZ_PICCOLO_LEVEL);
}

/// `02:D477`: with the feature, the random battles of maps 0 and 2 are never stopped: the routine returns (`rtl`) at its entry, as
/// it does when the levels are below its compares, and the encounter level stays what the map set. Without it, the original runs.
fn encounters(ctx: &mut Ctx<'_>) {
    if feature(ctx).is_some() {
        ctx.set_cost(0);
    } else {
        ctx.resume_at(ENCOUNTER_KEY);
    }
}

/// A compare of the recruits' deadline (`CMP #$09`, `CMP #$0A`): with the feature A is 0, so the compare fails and the routine
/// returns without the deadline. The compare itself is left to the original.
fn deadline(ctx: &mut Ctx<'_>, key: u32) {
    if feature(ctx).is_some() {
        ctx.set_al(0);
    }
    ctx.resume_at(key);
}

fn deadline_piccolo(ctx: &mut Ctx<'_>) {
    deadline(ctx, DEADLINE_PICCOLO_KEY);
}

fn deadline_gohan(ctx: &mut Ctx<'_>) {
    deadline(ctx, DEADLINE_GOHAN_KEY);
}

/// The `CMP #$2F` of a Super Saiyan guard: with the feature a level below 47 is read as 47. The compare is left to the original.
fn super_saiyan(ctx: &mut Ctx<'_>, key: u32) {
    if feature(ctx).is_some() {
        ctx.set_al(super_saiyan_level(ctx.al()));
    }
    ctx.resume_at(key);
}

fn super_saiyan_trigger(ctx: &mut Ctx<'_>) {
    super_saiyan(ctx, SSJ_TRIGGER_KEY);
}

fn super_saiyan_build(ctx: &mut Ctx<'_>) {
    super_saiyan(ctx, SSJ_BUILD_KEY);
}

/// `03:EDF6`: the compare of Goku's level in the map 16 set-up. With the feature's `ginyu_arrival` from 1 to 3, A is the level of that
/// arrival; with 0 the original's rule stays. The compare is left to the original.
fn ginyu_arrival(ctx: &mut Ctx<'_>) {
    if let Some(level) = feature(ctx).and_then(ginyu_level) {
        ctx.set_al(level);
    }
    ctx.resume_at(GINYU_KEY);
}

/// Whether effect `$35` is given a level its BP table does not hold (the first record's level, Piccolo's).
fn piccolo_out_of_table(ctx: &Ctx<'_>) -> bool {
    !piccolo_table_has(ctx.peek8(FIRST_RECORD + RECORD_LEVEL))
}

/// `03:C59C`, `LDA $03C5BA,X` (the BP's low word, A 16-bit): for a level outside the table A is the record's own BP word, which
/// the store after it writes back, so the BP stays. The original loads it for a level in the table. Resumes after the load.
fn piccolo_bp_low(ctx: &mut Ctx<'_>) {
    if feature(ctx).is_some() && piccolo_out_of_table(ctx) {
        let word = ctx.read16(FIRST_RECORD + RECORD_BP + u32::from(ctx.y()));
        ctx.set_a(word);
        ctx.set_nz16(word);
        ctx.resume_at(PICCOLO_BP_LOW_KEY + LOAD_LONG_LEN);
    } else {
        ctx.resume_at(PICCOLO_BP_LOW_KEY);
    }
}

/// `03:C5A7`, `LDA $03C5BC,X` (the BP's high byte, A 8-bit, Y two more than at the first load): as [`piccolo_bp_low`].
fn piccolo_bp_high(ctx: &mut Ctx<'_>) {
    if feature(ctx).is_some() && piccolo_out_of_table(ctx) {
        let byte = ctx.read8(FIRST_RECORD + RECORD_BP + u32::from(ctx.y()));
        ctx.set_al(byte);
        ctx.set_nz8(byte);
        ctx.resume_at(PICCOLO_BP_HIGH_KEY + LOAD_LONG_LEN);
    } else {
        ctx.resume_at(PICCOLO_BP_HIGH_KEY);
    }
}

/// One hook of the feature: a `Patch` at `key`, costing nothing. `m8` is the width of the accumulator the instruction at `key` is
/// read with (the hook is run only then).
fn patch(name: &str, key: u32, m8: bool, func: RoutineFn, ret: ReturnKind) -> HookEntry {
    HookEntry {
        name: name.to_owned(),
        key,
        mode: HookMode::Patch,
        action: HookAction::Routine { func, ret },
        entry_m8: Some(m8),
        entry_x8: None,
        flags_mask: 0,
        verify_sample: 1,
        cycles: Some(0),
        bound: None,
        budget_frames: None,
    }
}

/// The hook entries of `free_progression`, all `Patch`. The Raditz, deadline and Super Saiyan hooks and the Ginyu one resume at
/// their own key or after the load they replace; the encounter hook returns (`rtl`) as the routine it replaces ends.
pub(super) fn hook_entries() -> Vec<HookEntry> {
    let rtl = ReturnKind::Rtl;
    vec![
        patch("mod_fp_raditz_goku", RADITZ_GOKU_KEY, true, raditz_goku, rtl),
        patch("mod_fp_raditz_piccolo", RADITZ_PICCOLO_KEY, true, raditz_piccolo, rtl),
        patch("mod_fp_encounters", ENCOUNTER_KEY, true, encounters, rtl),
        patch("mod_fp_deadline_piccolo", DEADLINE_PICCOLO_KEY, true, deadline_piccolo, rtl),
        patch("mod_fp_deadline_gohan", DEADLINE_GOHAN_KEY, true, deadline_gohan, rtl),
        patch("mod_fp_ssj_trigger", SSJ_TRIGGER_KEY, true, super_saiyan_trigger, rtl),
        patch("mod_fp_ssj_build", SSJ_BUILD_KEY, true, super_saiyan_build, rtl),
        patch("mod_fp_ginyu_arrival", GINYU_KEY, true, ginyu_arrival, rtl),
        patch("mod_fp_piccolo_bp_low", PICCOLO_BP_LOW_KEY, false, piccolo_bp_low, rtl),
        patch("mod_fp_piccolo_bp_high", PICCOLO_BP_HIGH_KEY, true, piccolo_bp_high, rtl),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_raditz_compares_pass_only_when_the_event_holds() {
        // The original's compares: `CMP #$05` then `BCS` (A at least 5 starts the fight) and `CMP #$04` then `BCC` leaves (A at
        // least 4 starts it). The level the hook gives passes them with the event and fails them without.
        assert!(raditz_level(true, RADITZ_GOKU_LEVEL) >= 5);
        assert!(raditz_level(false, RADITZ_GOKU_LEVEL) < 5);
        assert!(raditz_level(true, RADITZ_PICCOLO_LEVEL) >= 4);
        assert!(raditz_level(false, RADITZ_PICCOLO_LEVEL) < 4);
        assert_eq!(IMPOSTER_BEATEN, (0x7E_0171, 0x04), "flag 0x0A is $0171 bit 2");
    }

    #[test]
    fn the_super_saiyan_level_is_raised_to_47_and_never_lowered() {
        assert_eq!(super_saiyan_level(1), 47);
        assert_eq!(super_saiyan_level(46), 47);
        assert_eq!(super_saiyan_level(47), 47);
        assert_eq!(super_saiyan_level(48), 48, "a level above is left as it is");
        assert_eq!(super_saiyan_level(99), 99);
    }

    #[test]
    fn the_ginyu_arrivals_are_the_three_levels_of_the_original_rule_and_zero_is_the_rule_itself() {
        assert_eq!(ginyu_level(0), None, "0: the original's rule stays");
        // The map 16 set-up: below 34 is the first arrival (flag 0x10), 34 and 35 the second (0x11), 36 and more the last (0x12).
        let arrival = |level: u8| {
            if level < 34 {
                1
            } else if level < 36 {
                2
            } else {
                3
            }
        };
        for wanted in 1..=3_u8 {
            let level = ginyu_level(wanted).expect("an arrival from 1 to 3 has a level");
            assert_eq!(arrival(level), wanted, "arrival {wanted} is read from level {level}");
        }
        assert_eq!(ginyu_level(4), None, "the feature refuses it, the hook leaves it");
    }

    #[test]
    fn the_bp_table_of_effect_35_holds_the_levels_33_to_40() {
        for level in 0..=255_u8 {
            assert_eq!(piccolo_table_has(level), (33..=40).contains(&level), "level {level}");
        }
    }

    #[test]
    fn the_hooks_are_at_their_addresses_and_none_is_a_cheat_or_another_feature_key() {
        let entries = hook_entries();
        let keys: Vec<u32> = entries.iter().map(|entry| entry.key).collect();
        assert_eq!(
            keys,
            [
                0x03_F22C, 0x03_F239, 0x02_D477, 0x03_F30B, 0x03_F31B, 0x05_8794, 0x03_868E,
                0x03_EDF6, 0x03_C59C, 0x03_C5A7
            ]
        );
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "one hook per key");
        let cheats = crate::cheats::entry_points();
        for entry in &entries {
            assert!(!cheats.contains(&entry.key), "{} is a cheat's key", entry.name);
            assert!(
                !super::super::features::SSJ_UNIT_STORE_KEYS.contains(&entry.key),
                "{} is a Kaioken key",
                entry.name
            );
            assert_eq!(entry.mode, HookMode::Patch, "{}", entry.name);
            assert_eq!(entry.cycles, Some(0), "{}", entry.name);
            assert!(entry.name.starts_with("mod_fp_"), "{}", entry.name);
        }
    }

    #[test]
    fn each_hook_names_the_width_of_the_accumulator_its_instruction_is_read_with() {
        for entry in hook_entries() {
            // Only the load of the BP's low word is read with a 16-bit accumulator (`REP #$20` before it).
            let wide = entry.key == PICCOLO_BP_LOW_KEY;
            assert_eq!(entry.entry_m8, Some(!wide), "{}", entry.name);
            assert_eq!(entry.entry_x8, None, "{}", entry.name);
        }
    }

    #[test]
    fn the_replaced_loads_are_resumed_after_their_length() {
        assert_eq!(RADITZ_GOKU_KEY + LOAD_ABS_LEN, 0x03_F22F, "the CMP #$05");
        assert_eq!(RADITZ_PICCOLO_KEY + LOAD_ABS_LEN, 0x03_F23C, "the CMP #$04");
        assert_eq!(PICCOLO_BP_LOW_KEY + LOAD_LONG_LEN, 0x03_C5A0, "the STA $0B07,Y");
        assert_eq!(PICCOLO_BP_HIGH_KEY + LOAD_LONG_LEN, 0x03_C5AB, "the STA $0B07,Y");
    }
}
