//! The named features of the packs (M10, Task 10): the behaviours the game has by name, which a pack selects in its manifest
//! (`ssd_mod::feature`). [`hooks_of`] is the registry: a match on the feature's id gives its hooks. Each hook reads the
//! feature's parameters from its slot of `Family::Feature` (`super::redirect::overlay`) and declines at once when the slot is
//! not in the image, so a run without the pack is the original's.
//!
//! **`ssj_kaioken`** (`docs/re/data/kaioken-bp.md`, sections 4 to 7; unit `$3B`, the Super Saiyan Goku, is unit 59):
//! - the boost's multiplier `m` is capped at `2 * (max_factor - 1)`, so the factor `1 + m / 2` is at most `max_factor` ([`SSJ_MULTIPLIER_KEY`]);
//! - while the Kaioken is on (`$0C36` bit 7) a move of the party costs `hp_cost_percent` of the fighter's maximum HP, and the
//!   fighter keeps at least 1 HP ([`SSJ_HP_COST_KEY`]);
//! - the eight stores of unit `$39` or `$3A` into a record (the Kaioken's stops, its prompt, the start of a fight, the Auto
//!   choice, a map's return, a story's event) are skipped while the record is unit `$3B` ([`UNIT_STORES`]).
//!
//! The command list of unit `$3B` with the Kaioken is the pack's `[[unit]]` (`tables.toml`), read by the tables' redirector. The
//! 40-bit boost and its restores are the wide BP's hooks ([`super::bp_kaioken`]).
//!
//! **`enemy_scaling`** (`docs/mods/free-progression.md`): the enemies of a battle follow the party's level; its hooks are
//! [`super::scaling`]'s.
//!
//! **`party_boosts`**: Guru, Porunga's level wish and Launch act on every member of the party; its hooks are
//! [`super::party_boosts`]'s.

use super::redirect::overlay;
use super::state::{PARTY_SLOTS, party_record, side_record};
use ssd_mod::Family;
use ssd_mod::feature::{ENEMY_SCALING, FREE_PROGRESSION, PARTY_BOOSTS, SSJ_KAIOKEN};
use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, ReturnKind, RoutineFn};

/// The unit id of the Super Saiyan Goku (`$3B`).
const SUPER_SAIYAN: u8 = 0x3B;
/// `05:AB98`: the `STA $01EF` that stores the boost's multiplier `m` in A (8 bits, loaded from `1D:8E7B` at `05:AB94`).
pub(super) const SSJ_MULTIPLIER_KEY: u32 = 0x05_AB98;
/// `05:9D4A`: the `LDA $1401`, after the move's KI is paid or refused (`05:9C57`'s tail): the side record is copied back after it,
/// so a change to side record A made here reaches the party record. Every path to it ends the move of the acting fighter.
pub(super) const SSJ_HP_COST_KEY: u32 = 0x05_9D4A;

/// The keys of the eight unit stores of [`UNIT_STORES`], in its order.
pub(super) const SSJ_UNIT_STORE_KEYS: [u32; 8] = [
    UNIT_STORES[0].key,
    UNIT_STORES[1].key,
    UNIT_STORES[2].key,
    UNIT_STORES[3].key,
    UNIT_STORES[4].key,
    UNIT_STORES[5].key,
    UNIT_STORES[6].key,
    UNIT_STORES[7].key,
];

/// Where a unit-id store writes: the party record of the acting character (`$0B01,Y`, Y the party record's offset) or the side
/// record Y names (`$0001,Y`, Y `$1400` or `$1440`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Record {
    Party,
    Side,
}

/// One of the stores of kaioken-bp.md section 6: the address of the `STA` that writes the unit, the address after it (the
/// store is three bytes, `STA abs,Y`), and the record it writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UnitStore {
    key: u32,
    after: u32,
    record: Record,
}

/// The eight stores that put unit `$39` or `$3A` into record `+01` (kaioken-bp.md section 6, "Where a mod must look"). Each is
/// a `STA` of the accumulator; the hook skips it when the record's unit is `$3B`. `03:F089` is reached only for a `$3A` record
/// (the `CMP #$3A` before it), so it never skips for `$3B`; it is listed so that the sites are the documented ones.
const UNIT_STORES: [UnitStore; 8] = [
    // `01:9FE8`: the prompt's "no": the pre-boost BP goes back and the Kaioken stops, unit `$39`.
    UnitStore { key: 0x01_9FE8, after: 0x01_9FEB, record: Record::Party },
    // `01:9FF7`: the prompt's "yes" with KI of 10 or more, unit `$3A`.
    UnitStore { key: 0x01_9FF7, after: 0x01_9FFA, record: Record::Party },
    // `01:A004`: the other Kaioken stop of the prompt, unit `$39`.
    UnitStore { key: 0x01_A004, after: 0x01_A007, record: Record::Party },
    // `05:83F7`: the start of a fight with a stale Kaioken command, unit `$39`.
    UnitStore { key: 0x05_83F7, after: 0x05_83FA, record: Record::Party },
    // `05:9D2F`: the Kaioken stops by KI, unit `$39` into the acting side record.
    UnitStore { key: 0x05_9D2F, after: 0x05_9D32, record: Record::Side },
    // `03:8D19`: the Auto choice's Kaioken, unit `$3A`.
    UnitStore { key: 0x03_8D19, after: 0x03_8D1C, record: Record::Party },
    // `03:F089`: a map's return turns a `$3A` record back into `$39` (for `$3B` the compare before it fails).
    UnitStore { key: 0x03_F089, after: 0x03_F08C, record: Record::Party },
    // `05:A885`: the story's animation event `$52`, unit `$3A` into the side record Y names.
    UnitStore { key: 0x05_A885, after: 0x05_A888, record: Record::Side },
];

/// The parameters of `ssj_kaioken` in the image, `(max_factor, hp_cost_percent)`, or `None` when the pack does not enable it.
fn ssj_kaioken(ctx: &Ctx<'_>) -> Option<(u8, u8)> {
    let at = overlay(ctx, Family::Feature, SSJ_KAIOKEN)?;
    Some((ctx.peek8(at), ctx.peek8(at + 1)))
}

/// The largest `m` of the Kaioken for a `max_factor`: the factor `1 + m / 2` is then at most `max_factor`.
#[must_use]
fn multiplier_cap(max_factor: u8) -> u8 {
    2 * max_factor.saturating_sub(1)
}

/// The HP a fighter keeps after a move's cost of `percent` of its maximum: `hp - maximum * percent / 100`, never below 1, and
/// a fighter at 0 HP is left at 0 (the cost does not revive).
#[must_use]
fn hp_after_cost(hp: u16, maximum: u16, percent: u8) -> u16 {
    if hp == 0 {
        return 0;
    }
    let cost = u32::from(maximum) * u32::from(percent) / 100;
    u32::from(hp).saturating_sub(cost).max(1) as u16
}

/// The bus address of the side record `Y` names (`$1400` for side A, `$1440` for side B).
fn side_record_at(y: u16) -> Option<u32> {
    match y {
        0x1400 => Some(side_record(false)),
        0x1440 => Some(side_record(true)),
        _ => None,
    }
}

/// The bus address of the unit byte the store writes into, for the store's record and `Y`: `$7E:0B01 + Y` for a party record,
/// `$7E:(Y + 1)` for a side record. `None` when `Y` names no record of that kind.
fn unit_byte(record: Record, y: u16) -> Option<u32> {
    match record {
        Record::Party => {
            (u32::from(y) < 0x20 * PARTY_SLOTS).then_some(party_record(0) + u32::from(y) + 1)
        }
        Record::Side => side_record_at(y).map(|at| at + 1),
    }
}

/// `05:AB98`: the boost's multiplier. For the Super Saiyan's Kaioken (the side record of `Y` is unit `$3B`) it is capped, so the
/// factor is at most `max_factor`; the boost's 40-bit add and its restores are [`super::bp_kaioken`]'s.
fn multiplier_wrapper(ctx: &mut Ctx<'_>) {
    if let Some((max_factor, _)) = ssj_kaioken(ctx)
        && side_record_at(ctx.y()).is_some_and(|record| ctx.peek8(record + 1) == SUPER_SAIYAN)
    {
        let cap = multiplier_cap(max_factor);
        if ctx.al() > cap {
            ctx.set_al(cap);
        }
    }
    ctx.resume_at(SSJ_MULTIPLIER_KEY);
}

/// `05:9D4A`: the end of a move of the party. The Kaioken on (`$0C36` bit 7), the party acting (`$14C2` bit 0 clear) and side
/// record A unit `$3B`: its HP falls by the cost of `hp_cost_percent` of its maximum, at least 1 HP is kept.
fn hp_cost_wrapper(ctx: &mut Ctx<'_>) {
    if let Some((_, percent)) = ssj_kaioken(ctx) {
        let record = side_record(false);
        let acting = ctx.peek8(0x7E_14C2) & 1 == 0;
        let kaioken_on = ctx.peek8(0x7E_0C36) & 0x80 != 0;
        if acting && kaioken_on && ctx.peek8(record + 1) == SUPER_SAIYAN {
            let hp = ctx.peek16(record + 3);
            let maximum = ctx.peek16(record + 0x0A);
            ctx.write16(record + 3, hp_after_cost(hp, maximum, percent));
        }
    }
    ctx.resume_at(SSJ_HP_COST_KEY);
}

/// One of the [`UNIT_STORES`]: its `STA` is skipped (the routine resumes after it) when the feature is on and the record's unit is
/// `$3B`; otherwise it runs as the original's.
fn unit_store<const N: usize>(ctx: &mut Ctx<'_>) {
    let store = UNIT_STORES[N];
    let keep = ssj_kaioken(ctx).is_some()
        && unit_byte(store.record, ctx.y()).is_some_and(|at| ctx.peek8(at) == SUPER_SAIYAN);
    ctx.resume_at(if keep { store.after } else { store.key });
}

/// The hook entries of `ssj_kaioken`: the multiplier, the HP cost and the eight unit stores, all `Patch`.
fn ssj_kaioken_hooks() -> Vec<HookEntry> {
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
        patch("mod_ssj_kaioken_multiplier", SSJ_MULTIPLIER_KEY, multiplier_wrapper),
        patch("mod_ssj_kaioken_hp_cost", SSJ_HP_COST_KEY, hp_cost_wrapper),
        patch("mod_ssj_unit_store_1", SSJ_UNIT_STORE_KEYS[0], unit_store::<0>),
        patch("mod_ssj_unit_store_2", SSJ_UNIT_STORE_KEYS[1], unit_store::<1>),
        patch("mod_ssj_unit_store_3", SSJ_UNIT_STORE_KEYS[2], unit_store::<2>),
        patch("mod_ssj_unit_store_4", SSJ_UNIT_STORE_KEYS[3], unit_store::<3>),
        patch("mod_ssj_unit_store_5", SSJ_UNIT_STORE_KEYS[4], unit_store::<4>),
        patch("mod_ssj_unit_store_6", SSJ_UNIT_STORE_KEYS[5], unit_store::<5>),
        patch("mod_ssj_unit_store_7", SSJ_UNIT_STORE_KEYS[6], unit_store::<6>),
        patch("mod_ssj_unit_store_8", SSJ_UNIT_STORE_KEYS[7], unit_store::<7>),
    ]
}

/// The features the game has, by id (the ids of `ssd_mod::feature`).
const REGISTERED: &[u16] = &[SSJ_KAIOKEN, FREE_PROGRESSION, ENEMY_SCALING, PARTY_BOOSTS];

/// The hooks of the feature `id`: a match on the id, the registry of the named features.
fn hooks_of(id: u16) -> Vec<HookEntry> {
    match id {
        SSJ_KAIOKEN => ssj_kaioken_hooks(),
        FREE_PROGRESSION => super::progression::hook_entries(),
        ENEMY_SCALING => super::scaling::hooks(),
        PARTY_BOOSTS => super::party_boosts::hooks(),
        _ => Vec::new(),
    }
}

/// The hook entries of every named feature, all `Patch`: they cost nothing and never call the original.
pub(super) fn hook_entries() -> Vec<HookEntry> {
    REGISTERED.iter().flat_map(|&id| hooks_of(id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_has_the_hooks_of_each_feature_and_an_unknown_id_has_none() {
        assert!(hooks_of(0).is_empty(), "no feature has id 0");
        assert!(hooks_of(SSJ_KAIOKEN + 100).is_empty());
        let names: Vec<String> =
            hooks_of(SSJ_KAIOKEN).into_iter().map(|entry| entry.name).collect();
        assert_eq!(names.len(), 10, "the multiplier, the HP cost and the eight stores");
        assert_eq!(names[0], "mod_ssj_kaioken_multiplier");
        assert_eq!(names[1], "mod_ssj_kaioken_hp_cost");
        assert_eq!(names[9], "mod_ssj_unit_store_8");
        let progression: Vec<String> =
            hooks_of(FREE_PROGRESSION).into_iter().map(|entry| entry.name).collect();
        assert_eq!(progression.len(), 10, "the ten hooks of free_progression");
        assert_eq!(progression[0], "mod_fp_raditz_goku");
        assert_eq!(progression[9], "mod_fp_piccolo_bp_high");
        let scaling = hooks_of(ENEMY_SCALING).len();
        assert_eq!(scaling, 3, "the record builder's exit, the EXP award and the random pick");
        let boosts: Vec<String> =
            hooks_of(PARTY_BOOSTS).into_iter().map(|entry| entry.name).collect();
        assert_eq!(boosts, ["mod_party_guru", "mod_party_porunga_level", "mod_party_lunch"]);
        assert_eq!(
            hook_entries().len(),
            names.len() + progression.len() + scaling + boosts.len(),
            "the registry is the four"
        );
    }

    #[test]
    fn the_hooks_are_at_the_documented_addresses_and_none_is_a_cheat_key() {
        assert_eq!(SSJ_MULTIPLIER_KEY, 0x05_AB98);
        assert_eq!(SSJ_HP_COST_KEY, 0x05_9D4A);
        let keys: Vec<u32> = UNIT_STORES.iter().map(|store| store.key).collect();
        assert_eq!(
            keys,
            [
                0x01_9FE8, 0x01_9FF7, 0x01_A004, 0x05_83F7, 0x05_9D2F, 0x03_8D19, 0x03_F089,
                0x05_A885
            ]
        );
        let cheats = crate::cheats::entry_points();
        for entry in hook_entries() {
            assert!(!cheats.contains(&entry.key), "{} is a cheat's key", entry.name);
            assert_eq!(entry.mode, HookMode::Patch, "{}", entry.name);
            assert_eq!(entry.cycles, Some(0), "{}", entry.name);
        }
        let mut keys: Vec<u32> = hook_entries().iter().map(|entry| entry.key).collect();
        keys.sort_unstable();
        let all = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), all, "no two features hook one instruction");
    }

    #[test]
    fn each_store_resumes_after_its_three_byte_store_and_each_key_is_its_own_store() {
        for store in UNIT_STORES {
            assert_eq!(store.after, store.key + 3, "{store:?}: STA abs,Y is three bytes");
        }
        assert!(
            UNIT_STORES.iter().filter(|store| store.record == Record::Side).count() == 2,
            "the two stores into a side record"
        );
    }

    #[test]
    fn a_party_offset_and_a_side_base_name_their_unit_byte_and_nothing_else_does() {
        assert_eq!(unit_byte(Record::Party, 0x00), Some(0x7E_0B01));
        assert_eq!(unit_byte(Record::Party, 0x100), Some(0x7E_0C01), "the ninth party record");
        assert_eq!(unit_byte(Record::Party, 0x120), None, "there is no tenth party record");
        assert_eq!(unit_byte(Record::Side, 0x1400), Some(0x7E_1401));
        assert_eq!(unit_byte(Record::Side, 0x1440), Some(0x7E_1441));
        assert_eq!(unit_byte(Record::Side, 0x1420), None);
    }

    #[test]
    fn the_multiplier_is_capped_so_the_factor_is_at_most_max_factor() {
        assert_eq!(multiplier_cap(10), 18, "1 + 18 / 2 is the factor 10");
        assert_eq!(multiplier_cap(11), 20, "the table's top, a factor of 11, is not capped");
        assert_eq!(multiplier_cap(1), 0, "a factor of 1 is no boost");
        // The factor is `1 + m / 2`, that is `(2 + m) / 2` in halves: the cap keeps `2 + m` within `2 * max_factor`, and one
        // more would pass it (unless the table's top, 20, is reached first).
        for max_factor in 1..=11_u32 {
            let cap = u32::from(multiplier_cap(max_factor as u8));
            assert!(2 + cap <= 2 * max_factor, "max_factor {max_factor}");
            if cap < 20 {
                assert!(2 + cap + 1 > 2 * max_factor, "max_factor {max_factor}: the cap is tight");
            }
        }
    }

    #[test]
    fn the_hp_cost_is_a_percent_of_the_maximum_and_never_takes_the_fighter_below_one() {
        assert_eq!(hp_after_cost(800, 800, 10), 720, "a tenth of 800 is 80");
        assert_eq!(hp_after_cost(100, 800, 10), 20);
        assert_eq!(hp_after_cost(50, 800, 10), 1, "the cost would be 80: one HP is kept");
        assert_eq!(hp_after_cost(1, 800, 10), 1);
        assert_eq!(hp_after_cost(0, 800, 10), 0, "a fighter at 0 is not revived");
        assert_eq!(hp_after_cost(800, 800, 0), 800, "a cost of 0 percent is none");
    }

    #[test]
    fn the_side_bases_name_their_records_and_nothing_else_does() {
        assert_eq!(side_record_at(0x1400), Some(side_record(false)));
        assert_eq!(side_record_at(0x1440), Some(side_record(true)));
        assert_eq!(side_record_at(0x0B00), None, "a party record is not a side record");
    }
}
