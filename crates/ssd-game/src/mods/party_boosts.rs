//! The feature `party_boosts` (`ssd_mod::feature::PARTY_BOOSTS`): three objects of the Item screen that act on one chosen member
//! act on every member of the party. Each is a switch of the pack (`guru`, `porunga_level`, `lunch`).
//!
//! **How.** An object's effect is a state machine run once a frame by `01:DBDC` (`menus.md`, "The item effects"); its first step
//! applies the effect to the record at `$0748` (the chosen member's offset in the party records, `$20` a member), and the next
//! steps are the animation and the closing of the use. Each hook here is a `Patch` at the `RTL` that ends the successful path of
//! that first step: the original has applied the effect to the chosen member, and the hook then runs the *original's own code* of
//! that step once for every other member, with `$0748` pointing at him (`Ctx::call_original`). The caps and the rules of the game
//! (the level cap table, the Kaioken guard, the marks that stop a second boost) are therefore the game's. The hook puts back
//! what the extra runs touched in the chosen member's state (`$0748`, the step `$073C`, the pending bits `$073B` and the targets
//! of the animation `$0740..$0746`), so the use goes on for him as it does without the pack, and the object is used up once.
//!
//! The party is the first `$01BB` party records that are in use (flags bit 7 or bit 6, as the member list of the Item screen,
//! `01:F168`, takes them). A member the original refuses the effect for is skipped, by the same test as the original's; the
//! original has no test for a knocked-out member in either (his flags keep bit 7), so he is included.
//!
//! - **Guru** (object 6; `01:DF13`, step 0 of the effect `01:DF03`, which King Kai's object 5 shares and keeps alone). The original
//!   refuses a member whose flags have bit 5 (he is boosted already), marks him, and computes the boosted BP (+50 percent, at most
//!   9 999 999) as the target `$0744/$0746` that the next step, a count-up animation (`01:DF9F` calling `01:E547`), moves his BP to.
//!   The hook is at `01:DF8A`, the `RTL` after the success. The count-up is a picture of the chosen member, with a sound for
//!   each call, so for the others the hook does not run it: it puts the BP where the count-up ends, the target the original
//!   computed for that member (the count-up clamps to it).
//! - **Porunga's level wish** (object 15, its second wish; `01:E41D`, the commit of step 7 of the effect `01:E2A5`). The original
//!   refuses a member who is at his level cap (`03:84EE`, the cap table), who has flags bit 1 or bit 5, or who is Goku (character 0)
//!   with the Kaioken on (`$0C36` bit 7); else it raises the level by one (at most 99), adds the level row's BP to his record and
//!   to the BP snapshot, and sets the targets for the maximum HP and KI that the next step, an animation, grows to. The hook is at
//!   `01:E4C1`, the `RTL` after the success. The extra members get the same test, then the same code. The animation of the maximum
//!   HP and KI runs per frame on the chosen member's picture (it draws and takes the wish's points), so for the others the hook
//!   puts the two maxima where the animation ends, the targets the original computed for that member. The wish's points and its
//!   mark are paid once, when the chosen member's animation ends, as in the original.
//! - **Launch** (object 9; `01:E066`, the effect that marks a member's attacks as the favourite ones, and its step 0 at `01:E074`).
//!   In a battle the Item screen (`01:8C63`) lets Launch take this normal effect three times in four; the other one in four
//!   (`rng_seed_a` & 3 is 0) it becomes object 27, an effect of its own (`01:ECAA`, "attack everyone"), which never reaches the
//!   code here and is left as the original has it. The normal effect refuses a member whose flags have bit 4 (he is marked
//!   already); else it sets bit 4 of his flags and bit 3 of his byte `+18`, and shows the mark. The hook is at `01:E0A0`, the
//!   `RTL` after the success. The extra members get the same test, then the same step. There is nothing to animate.
//!
//! Every hook declines at once (`resume_at` its own key) without the feature in the image, with its switch at 0, and for the
//! wrong object.

use super::redirect::overlay;
use super::state::{PARTY_SLOTS, party_record};
use ssd_mod::Family;
use ssd_mod::feature::PARTY_BOOSTS;
use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, ReturnKind, RoutineFn};

/// `01:DF8A`: the `RTL` after the success path of the BP objects' first step.
pub(super) const GURU_KEY: u32 = 0x01_DF8A;
/// `01:E4C1`: the `RTL` after the commit of Porunga's level wish.
pub(super) const PORUNGA_KEY: u32 = 0x01_E4C1;

/// The original code the hooks run for every other member: the first step of the BP objects (from its load of `$0748`), and
/// the level wish's commit (after its test), each ending with an `RTL`.
const GURU_STEP: u32 = 0x01_DF13;
const PORUNGA_COMMIT: u32 = 0x01_E41D;
/// `01:E0A0`: the `RTL` after the success path of Launch's step 0, which starts at `01:E074`.
pub(super) const LUNCH_KEY: u32 = 0x01_E0A0;
const LUNCH_STEP: u32 = 0x01_E074;
/// `03:84EE`: the level cap check (`Y` the record offset; carry set below the cap).
const CAP_CHECK: u32 = 0x03_84EE;

/// The object ids of Guru and of Launch.
const GURU: u8 = 6;
const LUNCH: u8 = 9;

/// The effect's variables: the chosen object, the step, the pending bits, the animation's targets and the chosen member's offset.
const ITEM_ID: u32 = 0x7E_0D7C;
const STEP: u32 = 0x7E_073C;
const PENDING: u32 = 0x7E_073B;
const HP_TARGET: u32 = 0x7E_0740;
const KI_TARGET: u32 = 0x7E_0742;
const BP_TARGET: u32 = 0x7E_0744;
const BP_TARGET_HIGH: u32 = 0x7E_0746;
const TARGET: u32 = 0x7E_0748;
/// The party's size (`$01BB`) and the Kaioken's byte (`$0C36`, bit 7 while it is on).
const PARTY_SIZE: u32 = 0x7E_01BB;
const KAIOKEN: u32 = 0x7E_0C36;

/// The bytes of a party record (`characters.md`).
const FLAGS: u32 = 0x00;
const MAX_HP: u32 = 0x0A;
const MAX_KI: u32 = 0x0C;
const BP: u32 = 0x07;
const CHARACTER: u32 = 0x1C;
/// A member is in the party when bit 7 or bit 6 of his flags is set.
const IN_PARTY: u8 = 0xC0;
/// The BP objects: the flag of a boost under way.
const GURU_DONE: u8 = 0x20;
/// The wish's refusal bits.
const WISH_REFUSED: u8 = 0x22;
/// Launch: the flag of a member whose attacks are marked already (the original refuses him).
const LUNCH_DONE: u8 = 0x10;

/// Which of the effects a hook serves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Boost {
    Guru,
    Porunga,
    Lunch,
}

impl Boost {
    /// The index of the effect's byte in the feature's parameters (`guru`, `porunga_level`, `lunch`).
    fn parameter(self) -> u32 {
        match self {
            Self::Guru => 0,
            Self::Porunga => 1,
            Self::Lunch => 2,
        }
    }
}

/// Whether the feature is in the image with the switch of `boost` on.
fn on(ctx: &Ctx<'_>, boost: Boost) -> bool {
    overlay(ctx, Family::Feature, PARTY_BOOSTS)
        .is_some_and(|at| ctx.peek8(at + boost.parameter()) != 0)
}

/// The offsets (in the party records, `$20` a member) of the members other than the one at `chosen`, among the first `size`
/// records, for whom `fits` says the effect applies. `flags(offset)` gives a record's flags.
fn others(
    size: u8,
    chosen: u16,
    flags: impl Fn(u16) -> u8,
    fits: impl Fn(u16, u8) -> bool,
) -> Vec<u16> {
    (0..u16::from(size.min(PARTY_SLOTS as u8)))
        .map(|slot| slot * 0x20)
        .filter(|&offset| offset != chosen)
        .filter(|&offset| flags(offset) & IN_PARTY != 0)
        .filter(|&offset| fits(offset, flags(offset)))
        .collect()
}

/// The BP objects are refused for a member who is boosted already (flags bit 5).
#[must_use]
fn guru_fits(flags: u8) -> bool {
    flags & GURU_DONE == 0
}

/// Launch is refused for a member whose attacks are marked already (flags bit 4).
#[must_use]
fn lunch_fits(flags: u8) -> bool {
    flags & LUNCH_DONE == 0
}

/// The level wish's tests besides the cap: refused for flags bit 1 or bit 5, and for Goku (character 0) with the Kaioken on.
#[must_use]
fn wish_fits(flags: u8, character: u8, kaioken_on: bool) -> bool {
    flags & WISH_REFUSED == 0 && (character != 0 || !kaioken_on)
}

/// The registers and the effect's variables that an extra run of the original's step may change and the chosen member's use
/// still needs.
struct Saved {
    registers: ssd_oracle::Registers,
    target: u16,
    step: u8,
    pending: u8,
    targets: [u16; 4],
}

impl Saved {
    fn take(ctx: &Ctx<'_>) -> Self {
        Self {
            registers: ctx.registers(),
            target: ctx.peek16(TARGET),
            step: ctx.peek8(STEP),
            pending: ctx.peek8(PENDING),
            targets: [
                ctx.peek16(HP_TARGET),
                ctx.peek16(KI_TARGET),
                ctx.peek16(BP_TARGET),
                ctx.peek16(BP_TARGET_HIGH),
            ],
        }
    }

    fn put_back(self, ctx: &mut Ctx<'_>) {
        ctx.write16(TARGET, self.target);
        ctx.write8(STEP, self.step);
        ctx.write8(PENDING, self.pending);
        ctx.write16(HP_TARGET, self.targets[0]);
        ctx.write16(KI_TARGET, self.targets[1]);
        ctx.write16(BP_TARGET, self.targets[2]);
        ctx.write16(BP_TARGET_HIGH, self.targets[3]);
        ctx.set_registers(self.registers);
    }
}

/// The members other than the chosen one for whom the effect applies now, in the party's order.
fn members(ctx: &Ctx<'_>, fits: impl Fn(&Ctx<'_>, u16, u8) -> bool) -> Vec<u16> {
    let chosen = ctx.peek16(TARGET);
    let size = ctx.peek8(PARTY_SIZE);
    others(
        size,
        chosen,
        |offset| ctx.peek8(party_record(0) + u32::from(offset) + FLAGS),
        |offset, flags| fits(ctx, offset, flags),
    )
}

/// Runs the original's `code` for the member at `offset` (`$0748` points at him) and leaves the registers as they were.
fn run_for(ctx: &mut Ctx<'_>, code: u32, offset: u16) -> bool {
    ctx.write16(TARGET, offset);
    ctx.set_y(offset);
    ctx.call_original(code, ReturnKind::Rtl).is_ok()
}

/// The work of a hook that serves `boost` for the object `item` (any object when `None`): `each` is run for every member, after
/// the chosen member's variables are saved, and they are put back after the last. Declines (`resume_at` the hook's own `key`)
/// when the feature, its switch or the object do not apply.
fn serve(
    ctx: &mut Ctx<'_>,
    key: u32,
    boost: Boost,
    item: Option<u8>,
    fits: impl Fn(&Ctx<'_>, u16, u8) -> bool,
    each: impl Fn(&mut Ctx<'_>, u16),
) {
    ctx.mark_gated();
    if !on(ctx, boost) || item.is_some_and(|item| ctx.peek8(ITEM_ID) != item) {
        ctx.resume_at(key);
        return;
    }
    let list = members(ctx, fits);
    if list.is_empty() {
        ctx.resume_at(key);
        return;
    }
    let saved = Saved::take(ctx);
    for offset in list {
        each(ctx, offset);
        if ctx.failed() {
            return;
        }
    }
    saved.put_back(ctx);
    ctx.set_cost(0);
}

/// `01:DF8A`: Guru's boost is applied to the chosen member; the same step is run for every other member who is not boosted, and
/// his BP is put at the target the step computed (where the count-up of the chosen member will end).
fn guru_wrapper(ctx: &mut Ctx<'_>) {
    serve(
        ctx,
        GURU_KEY,
        Boost::Guru,
        Some(GURU),
        |_, _, flags| guru_fits(flags),
        |ctx, offset| {
            if run_for(ctx, GURU_STEP, offset) {
                let record = party_record(0) + u32::from(offset);
                let (low, high) = (ctx.peek16(BP_TARGET), ctx.peek8(BP_TARGET_HIGH));
                ctx.write16(record + BP, low);
                ctx.write8(record + BP + 2, high);
            }
        },
    );
}

/// `01:E0A0`: Launch's normal effect is applied to the chosen member; the same step is run for every other member who is not
/// marked. The step has no animation: it is complete for each member.
fn lunch_wrapper(ctx: &mut Ctx<'_>) {
    serve(
        ctx,
        LUNCH_KEY,
        Boost::Lunch,
        Some(LUNCH),
        |_, _, flags| lunch_fits(flags),
        |ctx, offset| {
            run_for(ctx, LUNCH_STEP, offset);
        },
    );
}

/// Whether the level cap check (`03:84EE`) says the member at `offset` is below his cap. Runs the original's routine.
fn below_cap(ctx: &mut Ctx<'_>, offset: u16) -> bool {
    ctx.set_y(offset);
    ctx.call_original(CAP_CHECK, ReturnKind::Rtl).is_ok() && ctx.c()
}

/// `01:E4C1`: Porunga's level wish is applied to the chosen member; the other members get the same tests and then the same code.
fn porunga_wrapper(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    if !on(ctx, Boost::Porunga) {
        ctx.resume_at(PORUNGA_KEY);
        return;
    }
    let list = members(ctx, |ctx, offset, flags| {
        let character = ctx.peek8(party_record(0) + u32::from(offset) + CHARACTER);
        wish_fits(flags, character, ctx.peek8(KAIOKEN) & 0x80 != 0)
    });
    if list.is_empty() {
        ctx.resume_at(PORUNGA_KEY);
        return;
    }
    let saved = Saved::take(ctx);
    for offset in list {
        if !below_cap(ctx, offset) {
            continue;
        }
        if !run_for(ctx, PORUNGA_COMMIT, offset) {
            return;
        }
        // The animation of the chosen member ends with the maxima at the targets; the extra members get them now.
        let record = party_record(0) + u32::from(offset);
        let (hp, ki) = (ctx.peek16(HP_TARGET), ctx.peek16(KI_TARGET));
        ctx.write16(record + MAX_HP, hp);
        ctx.write16(record + MAX_KI, ki);
    }
    saved.put_back(ctx);
    ctx.set_cost(0);
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

/// The hooks of `party_boosts`: Guru, Porunga's level wish and Launch.
pub(super) fn hooks() -> Vec<HookEntry> {
    vec![
        patch("mod_party_guru", GURU_KEY, guru_wrapper),
        patch("mod_party_porunga_level", PORUNGA_KEY, porunga_wrapper),
        patch("mod_party_lunch", LUNCH_KEY, lunch_wrapper),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags_of(table: &'static [u8]) -> impl Fn(u16) -> u8 {
        move |offset| table[usize::from(offset / 0x20)]
    }

    #[test]
    fn the_other_members_are_the_party_records_in_use_but_the_chosen_one() {
        let flags: &'static [u8] = &[0xC0, 0xC0, 0x80, 0x00, 0xC0, 0xC0];
        // Size 5: the record 3 is empty, the record 5 is past the size.
        let all = others(5, 0x20, flags_of(flags), |_, _| true);
        assert_eq!(
            all,
            [0x00, 0x40, 0x80],
            "the chosen member (record 1) and the empty one are left out"
        );
        // A knocked-out member (flags 0x80: bit 6 clear) is in the party.
        assert!(all.contains(&0x40));
        assert_eq!(
            others(1, 0x00, flags_of(flags), |_, _| true),
            Vec::<u16>::new(),
            "a party of one"
        );
        assert_eq!(others(0, 0x00, flags_of(flags), |_, _| true), Vec::<u16>::new());
        // The size is capped at the nine records the game has.
        let nine: &'static [u8] = &[0xC0; 12];
        assert_eq!(others(12, 0x00, flags_of(nine), |_, _| true).len(), 8);
    }

    #[test]
    fn a_member_the_originals_test_refuses_is_skipped() {
        let flags: &'static [u8] = &[0xC0, 0xE0, 0xC0, 0xE0];
        let guru = others(4, 0x00, flags_of(flags), |_, f| guru_fits(f));
        assert_eq!(guru, [0x40], "record 1 is in the party and not boosted; 3 is boosted already");
        assert!(guru_fits(0xC0) && !guru_fits(0xE0));
        // The wish: bit 1 or bit 5 refuses; Goku with the Kaioken on is refused, the others are not.
        assert!(wish_fits(0xC0, 1, true));
        assert!(!wish_fits(0xC2, 1, false));
        assert!(!wish_fits(0xE0, 1, false));
        assert!(!wish_fits(0xC0, 0, true), "Goku with the Kaioken on");
        assert!(wish_fits(0xC0, 0, false), "Goku with the Kaioken off");
        assert!(wish_fits(0xC0, 2, true));
        assert!(wish_fits(0x80, 2, false), "a knocked-out member is not refused");
        // Launch: refused only for a member whose flags have bit 4.
        assert!(lunch_fits(0xC0) && lunch_fits(0x80) && lunch_fits(0xE0));
        assert!(!lunch_fits(0xD0) && !lunch_fits(0xFF));
        let flags: &'static [u8] = &[0xC0, 0xD0, 0xC0, 0xD0];
        assert_eq!(
            others(4, 0x00, flags_of(flags), |_, f| lunch_fits(f)),
            [0x40],
            "record 1 is marked already"
        );
    }

    #[test]
    fn the_hooks_are_at_the_documented_addresses_and_are_patches() {
        assert_eq!((GURU_KEY, PORUNGA_KEY, LUNCH_KEY), (0x01_DF8A, 0x01_E4C1, 0x01_E0A0));
        let entries = hooks();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["mod_party_guru", "mod_party_porunga_level", "mod_party_lunch"]);
        for entry in &entries {
            assert_eq!(entry.mode, HookMode::Patch, "{}", entry.name);
            assert_eq!(entry.cycles, Some(0), "{}", entry.name);
        }
        // Each step the hooks run is before its own key, in the same routine.
        const {
            assert!(GURU_STEP < GURU_KEY && PORUNGA_COMMIT < PORUNGA_KEY && LUNCH_STEP < LUNCH_KEY);
        };
    }

    #[test]
    fn each_effect_reads_its_own_byte_of_the_feature() {
        assert_eq!(
            [Boost::Guru, Boost::Porunga, Boost::Lunch].map(Boost::parameter),
            [0, 1, 2],
            "guru, porunga_level, lunch: the order of ssd_mod::feature"
        );
    }
}
