//! The bag of Enhanced mode: the items the party carries, as stacks, and the projection of them into
//! the original's item list. Pure Rust: the stacks, their rules and the sidecar file's codec; the
//! game's list is written by `ssd-game` from [`Bag::project`].
//!
//! The original keeps one entry per unit in a list of 24. The bag keeps one stack per object up to a
//! limit of units (99 by default) and up to 60 stacks. The list shows the key items first (the Dragon
//! Balls and the story's objects), then one entry per stack, at most [`PROJECTED`] entries so the
//! original's own refusal at [`LIST_FULL`] never fires while the bag has room. A full bag shows
//! [`LIST_FULL`] entries: the original then refuses every pickup, purchase and battle drop by its own
//! check, which is how the 61st stack is refused (`docs/re/data/menus.md`, "The capacity spike").

use serde::{Deserialize, Serialize};

/// Entries of the original list the projection shows while the bag has room: three are left free, so
/// a battle's three drops always fit.
pub const PROJECTED: usize = 21;
/// Entries of the original list: its capacity, and the count at which it refuses an add.
pub const LIST_FULL: usize = 24;
/// Default number of stacks (slots) of the bag.
pub const SLOTS: u16 = 60;
/// Default number of units in one stack.
pub const STACK: u16 = 99;
/// The Dragon Balls: object ids 33 to 39.
const DRAGON_BALLS: std::ops::RangeInclusive<u8> = 33..=39;
/// Objects whose list entry keeps state of its own (`item_extra`): the card changers (the mask of the
/// choices used) and Porunga (the wishes left). Their units stack by state word: a unit joins a stack
/// only when the stack holds the same word, so a unit whose state changed (a wish used, a choice of a
/// card changer) leaves its stack and becomes a stack of its own.
const KEEPS_STATE: [u8; 6] = [9, 10, 11, 12, 13, 15];
/// Porunga's object id.
const PORUNGA: u8 = 15;
/// The state word the original gives Porunga with the object (`04:D37E`): 150 wish points in the high byte (a wish costs
/// 50, so three wishes), no wish used in the low byte. A word of 0 is never Porunga's own (the original takes the object
/// away when its points reach 0, `01:E335`): it is a unit that came without its word (the item spawner, a list rebuilt from
/// an older version), and it gets the original's, or each wish would wrap the points and the wishes would never end.
const PORUNGA_WORD: u16 = 0x9600;

/// The state word a unit of `object` is kept with: `extra`, or Porunga's own for a Porunga without one.
fn state_word(object: u8, extra: u16) -> u16 {
    if object == PORUNGA && extra == 0 { PORUNGA_WORD } else { extra }
}

/// The rules of a bag: its slots and the units of a stack. Both are options (`Ruleset`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    /// Stacks the bag holds.
    pub slots: u16,
    /// Units one stack holds.
    pub stack: u16,
}

impl Default for Rules {
    fn default() -> Self {
        Self { slots: SLOTS, stack: STACK }
    }
}

/// How an object is kept: a key item (pinned at the top of the list, never dropped) and whether its
/// units share a stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Class {
    pub key: bool,
    pub stackable: bool,
}

impl Class {
    /// The class of an object from its attribute byte's story bit (AID, bit 5) and its id.
    #[must_use]
    pub fn of(object: u8, aid: bool) -> Self {
        let key = aid || DRAGON_BALLS.contains(&object);
        Self { key, stackable: !key }
    }
}

/// A stack of one object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stack {
    /// Identifies the stack for as long as it exists (the projection and the drain refer to it).
    pub id: u32,
    /// The object id.
    pub object: u8,
    /// Units in the stack, from 1 to the stack limit.
    pub count: u16,
    /// The list entry's state word (`item_extra`), kept with the stack.
    pub extra: u16,
    pub class: Class,
}

/// A unit that does not fit: the bag has no slot for a new stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full;

/// The bag: stacks in the order they were acquired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bag {
    rules: Rules,
    stacks: Vec<Stack>,
    next_id: u32,
}

impl Bag {
    /// The bag's bytes (bincode, as the sidecar file encodes it): what a replay keeps of it.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(self, config()).unwrap_or_default()
    }

    /// A bag from its bytes.
    ///
    /// # Errors
    ///
    /// The bytes are not a bag.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let (bag, _): (Self, usize) = bincode::serde::decode_from_slice(bytes, config())
            .map_err(|error| format!("not a bag: {error}"))?;
        if !bag.is_consistent() {
            return Err("not a bag: its stacks are inconsistent".to_owned());
        }
        Ok(bag)
    }

    /// Whether the bag's stacks are what [`Bag::add`] and [`Bag::take`] make: ids that are unique
    /// and below the next one, and at least one unit in each. A file that was edited or damaged may
    /// not be, and the bag it holds is not used.
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        let mut ids: Vec<u32> = self.stacks.iter().map(|stack| stack.id).collect();
        ids.sort_unstable();
        ids.windows(2).all(|pair| pair[0] != pair[1])
            && ids.last().is_none_or(|&last| last < self.next_id)
            && self.stacks.iter().all(|stack| stack.count > 0)
            && self.stacks.len() <= usize::from(self.rules.slots.max(SLOTS))
    }

    /// The bag with the rules of the run (its slots and units of a stack are options, which may have
    /// changed since the bag was saved); the stacks it holds are kept.
    pub fn set_rules(&mut self, rules: Rules) {
        self.rules = rules;
    }

    /// An empty bag with `rules`.
    #[must_use]
    pub fn new(rules: Rules) -> Self {
        Self { rules, stacks: Vec::new(), next_id: 1 }
    }

    /// The rules of the bag.
    #[must_use]
    pub fn rules(&self) -> Rules {
        self.rules
    }

    /// The stacks, in the order they were acquired.
    #[must_use]
    pub fn stacks(&self) -> &[Stack] {
        &self.stacks
    }

    /// The stack with `id`, if it exists.
    #[must_use]
    pub fn stack(&self, id: u32) -> Option<&Stack> {
        self.stacks.iter().find(|stack| stack.id == id)
    }

    /// Whether every slot is taken.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.stacks.len() >= usize::from(self.rules.slots)
    }

    /// Units of every object in the bag.
    #[must_use]
    pub fn units(&self) -> u32 {
        self.stacks.iter().map(|stack| u32::from(stack.count)).sum()
    }

    /// Units of `object` in the bag.
    #[must_use]
    pub fn units_of(&self, object: u8) -> u32 {
        self.stacks
            .iter()
            .filter(|stack| stack.object == object)
            .map(|stack| u32::from(stack.count))
            .sum()
    }

    /// Brings a bag saved by an older version up to today's rules: each stack's class is computed again (state objects
    /// became stackable by their state word), and the units of stacks that may share one are joined into the first of
    /// them, up to the stack limit. Called when a saved bag is bound, so a bag kept before a rule change looks as one
    /// made under it.
    pub fn coalesce(&mut self) {
        for stack in &mut self.stacks {
            stack.class =
                Class::of(stack.object, stack.class.key && !DRAGON_BALLS.contains(&stack.object));
            stack.extra = state_word(stack.object, stack.extra);
        }
        let limit = self.rules.stack;
        let mut joined: Vec<Stack> = Vec::with_capacity(self.stacks.len());
        for stack in self.stacks.drain(..) {
            let mut stack = stack;
            if stack.class.stackable {
                let by_state = KEEPS_STATE.contains(&stack.object);
                for earlier in joined.iter_mut().filter(|earlier| {
                    earlier.object == stack.object && (!by_state || earlier.extra == stack.extra)
                }) {
                    let moved = stack.count.min(limit.saturating_sub(earlier.count));
                    earlier.count += moved;
                    stack.count -= moved;
                    if stack.count == 0 {
                        break;
                    }
                }
            }
            if stack.count > 0 {
                joined.push(stack);
            }
        }
        self.stacks = joined;
    }

    /// Adds one unit of `object` with the list entry's state `extra`. It joins the first stack of the
    /// object that has room (a stackable object only), else it takes a new slot. A state object (see
    /// [`KEEPS_STATE`]) joins only a stack with the same state word.
    ///
    /// # Errors
    ///
    /// [`Full`] when the unit needs a new slot and the bag has none; nothing is changed.
    pub fn add(&mut self, object: u8, extra: u16, class: Class) -> Result<u32, Full> {
        let extra = state_word(object, extra);
        if class.stackable {
            let limit = self.rules.stack;
            let by_state = KEEPS_STATE.contains(&object);
            if let Some(stack) = self.stacks.iter_mut().find(|stack| {
                stack.object == object && stack.count < limit && (!by_state || stack.extra == extra)
            }) {
                stack.count += 1;
                return Ok(stack.id);
            }
        }
        if self.is_full() {
            return Err(Full);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.stacks.push(Stack { id, object, count: 1, extra, class });
        Ok(id)
    }

    /// Removes up to `units` from the stack `id`; a stack that empties is gone. Returns the units
    /// removed (0 for a stack that does not exist).
    pub fn take(&mut self, id: u32, units: u16) -> u16 {
        let Some(index) = self.stacks.iter().position(|stack| stack.id == id) else {
            return 0;
        };
        let stack = &mut self.stacks[index];
        let removed = units.min(stack.count);
        stack.count -= removed;
        if stack.count == 0 {
            self.stacks.remove(index);
        }
        removed
    }

    /// The entries the original list shows, as stack ids in list order: `front` first when given
    /// (the stack the HD stages for its use), then the key items, then the other stacks, in the
    /// order they were acquired. At most [`PROJECTED`] entries, or [`LIST_FULL`] when the bag is full.
    #[must_use]
    pub fn project(&self, front: Option<u32>) -> Vec<u32> {
        let cap = if self.is_full() { LIST_FULL } else { PROJECTED };
        let front = front.filter(|&id| self.stack(id).is_some());
        let rest = |key: bool| {
            self.stacks
                .iter()
                .filter(move |stack| stack.class.key == key && Some(stack.id) != front)
                .map(|stack| stack.id)
        };
        let mut order: Vec<u32> = front.into_iter().chain(rest(true)).chain(rest(false)).collect();
        order.truncate(cap);
        order
    }
}

/// The bag of one save slot, as the sidecar file keeps it: the slot's checksum word says which save
/// the bag belongs to. A save changed by another way (its checksum no longer matches) rebuilds the
/// bag from the original list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedBag {
    /// The save slot, 0 to 2.
    pub slot: u8,
    /// The checksum word of the slot when the bag was saved.
    pub checksum: u16,
    pub bag: Bag,
}

/// The sidecar file (`saves/ssd.bag`): the bags of the save slots, versioned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    /// The file layout's version.
    pub version: u32,
    pub saves: Vec<SavedBag>,
}

/// The layout version of [`Sidecar`].
pub const SIDECAR_VERSION: u32 = 1;

/// The encoding of the sidecar file: bincode, like the save states.
fn config() -> bincode::config::Configuration<
    bincode::config::LittleEndian,
    bincode::config::Varint,
    bincode::config::Limit<{ 1024 * 1024 }>,
> {
    bincode::config::standard().with_limit::<{ 1024 * 1024 }>()
}

impl Sidecar {
    /// The empty sidecar of this version.
    #[must_use]
    pub fn empty() -> Self {
        Self { version: SIDECAR_VERSION, saves: Vec::new() }
    }

    /// The file's bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(self, config()).unwrap_or_default()
    }

    /// Reads the file's bytes.
    ///
    /// # Errors
    ///
    /// The bytes are not a sidecar, or another version of it.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let (sidecar, _): (Self, usize) = bincode::serde::decode_from_slice(bytes, config())
            .map_err(|error| format!("not a bag file: {error}"))?;
        if sidecar.version != SIDECAR_VERSION {
            return Err(format!(
                "bag file version {}, expected {SIDECAR_VERSION}",
                sidecar.version
            ));
        }
        // A bag that is not consistent is left out: its save rebuilds the bag from the list.
        let saves = sidecar.saves.into_iter().filter(|saved| saved.bag.is_consistent()).collect();
        Ok(Self { saves, ..sidecar })
    }

    /// The bag saved for `slot` with `checksum`, if the sidecar has one for exactly that save.
    #[must_use]
    pub fn bag_for(&self, slot: u8, checksum: u16) -> Option<&Bag> {
        self.saves
            .iter()
            .find(|saved| saved.slot == slot && saved.checksum == checksum)
            .map(|saved| &saved.bag)
    }

    /// Stores `bag` for `slot` with `checksum`, replacing the slot's older bag.
    pub fn store(&mut self, slot: u8, checksum: u16, bag: &Bag) {
        self.saves.retain(|saved| saved.slot != slot);
        self.saves.push(SavedBag { slot, checksum, bag: bag.clone() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_porunga_without_its_word_gets_the_originals_three_wishes() {
        let mut bag = Bag::new(Rules::default());
        let id = bag.add(PORUNGA, 0, Class::of(PORUNGA, false)).unwrap();
        assert_eq!(
            bag.stack(id).map(|stack| stack.extra),
            Some(PORUNGA_WORD),
            "150 points, no wish used"
        );
        let used = bag.add(PORUNGA, 0x9C40, Class::of(PORUNGA, false)).unwrap();
        assert_ne!(used, id, "a Porunga with a wish used keeps its own word and stack");
        assert_eq!(bag.stack(used).map(|stack| stack.extra), Some(0x9C40));
        // A bag kept with a word of 0 is brought to the original's word, and joins the stack that has it
        let mut old = Bag::new(Rules::default());
        old.add(PORUNGA, PORUNGA_WORD, Class::of(PORUNGA, false)).unwrap();
        old.stacks.push(Stack {
            id: 99,
            object: PORUNGA,
            count: 1,
            extra: 0,
            class: Class::of(PORUNGA, false),
        });
        old.coalesce();
        assert_eq!(old.stacks().len(), 1);
        assert_eq!((old.stacks()[0].count, old.stacks()[0].extra), (2, PORUNGA_WORD));
    }

    fn normal(object: u8) -> Class {
        Class::of(object, false)
    }

    /// A bag with `stacks` stacks of distinct normal objects (ids from 0), one unit each.
    fn bag_with(stacks: u8) -> Bag {
        let mut bag = Bag::new(Rules::default());
        for object in 0..stacks {
            bag.add(object, 0, normal(object)).unwrap();
        }
        bag
    }

    /// A small deterministic generator for the property tests (no dependency needed).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn the_default_rules_are_sixty_slots_of_ninety_nine() {
        assert_eq!(Rules::default(), Rules { slots: 60, stack: 99 });
        assert!(Class::of(33, false).key && !Class::of(33, false).stackable);
        assert!(Class::of(4, true).key, "an AID object is a key item");
        assert!(Class::of(10, false).stackable, "a card changer stacks by its state word");
        assert!(Class::of(15, false).stackable, "Porunga stacks by its wishes left");
        assert!(Class::of(2, false).stackable);
    }

    #[test]
    fn units_of_one_object_share_a_stack_up_to_the_limit() {
        let mut bag = Bag::new(Rules { slots: 60, stack: 3 });
        let ids: Vec<u32> = (0..7).map(|_| bag.add(2, 0, normal(2)).unwrap()).collect();
        assert_eq!(bag.stacks().len(), 3, "3 + 3 + 1 units");
        assert_eq!(&ids[..3], [ids[0]; 3]);
        assert_eq!(bag.units_of(2), 7);
        assert_eq!(bag.stacks().iter().map(|s| s.count).collect::<Vec<_>>(), [3, 3, 1]);
    }

    #[test]
    fn a_stack_that_empties_is_removed_and_take_never_goes_below_zero() {
        let mut bag = bag_with(0);
        let id = bag.add(2, 0, normal(2)).unwrap();
        bag.add(2, 0, normal(2)).unwrap();
        assert_eq!(bag.take(id, 5), 2, "only two units were there");
        assert_eq!(bag.stacks().len(), 0);
        assert_eq!(bag.take(id, 1), 0, "a stack that is gone takes nothing");
    }

    #[test]
    fn a_new_stack_needs_a_slot_and_a_full_bag_refuses_it_untouched() {
        let mut bag = Bag::new(Rules { slots: 2, stack: 99 });
        bag.add(1, 0, normal(1)).unwrap();
        bag.add(2, 0, normal(2)).unwrap();
        assert!(bag.is_full());
        assert_eq!(bag.add(3, 0, normal(3)), Err(Full));
        assert_eq!(
            bag.add(1, 0, normal(1)),
            Ok(bag.stacks()[0].id),
            "a stack with room still takes"
        );
        assert_eq!(bag.units(), 3);
    }

    #[test]
    fn a_bag_saved_before_state_objects_stacked_is_joined_when_bound() {
        // Before the rule change each Porunga was a stack of its own, marked not stackable.
        let old = Class { key: false, stackable: false };
        let mut bag = Bag::new(Rules::default());
        bag.stacks =
            (0..3).map(|id| Stack { id, object: 15, count: 1, extra: 0, class: old }).collect();
        bag.stacks.push(Stack { id: 3, object: 15, count: 1, extra: 7, class: old });
        bag.stacks.push(Stack {
            id: 4,
            object: 33,
            count: 1,
            extra: 0,
            class: Class::of(33, false),
        });
        bag.next_id = 5;
        bag.coalesce();
        let counts: Vec<(u8, u16, u16)> =
            bag.stacks().iter().map(|s| (s.object, s.count, s.extra)).collect();
        assert_eq!(
            counts,
            [(15, 3, PORUNGA_WORD), (15, 1, 7), (33, 1, 0)],
            "same state joins (a Porunga without its word has the story's), another state and a key item stay"
        );
        assert!(bag.is_consistent());
    }

    #[test]
    fn two_fresh_state_objects_stack_and_another_state_takes_a_stack_of_its_own() {
        let mut bag = Bag::new(Rules::default());
        // The wishes left are the high byte of the state word (the price box reads them so).
        let fresh = bag.add(15, 0x0300, Class::of(15, false)).unwrap();
        assert_eq!(bag.add(15, 0x0300, Class::of(15, false)), Ok(fresh));
        assert_eq!(bag.stacks().len(), 1, "two fresh Porunga are one stack");
        assert_eq!(bag.units_of(15), 2);
        let other = bag.add(15, 0x0200, Class::of(15, false)).unwrap();
        assert_ne!(other, fresh, "a Porunga with other wishes left is a stack of its own");
        assert_eq!(bag.stacks().len(), 2);
    }

    #[test]
    fn a_used_unit_of_a_state_object_leaves_its_stack_with_its_new_state() {
        let mut bag = Bag::new(Rules::default());
        let fresh = bag.add(15, 0x0300, Class::of(15, false)).unwrap();
        bag.add(15, 0x0300, Class::of(15, false)).unwrap();
        // The keeper's split: one unit leaves the stack it was in, and one unit comes in with the new word.
        assert_eq!(bag.take(fresh, 1), 1);
        let used = bag.add(15, 0x0200, Class::of(15, false)).unwrap();
        let stacks = bag.stacks();
        assert_eq!(stacks.len(), 2);
        assert_eq!(
            (stacks[0].id, stacks[0].count, stacks[0].extra),
            (fresh, 1, 0x0300),
            "the rest keep the old state"
        );
        assert_eq!(
            (stacks[1].id, stacks[1].count, stacks[1].extra),
            (used, 1, 0x0200),
            "one unit takes the new state"
        );
        assert_eq!(bag.project(None), [fresh, used], "the projection shows both stacks");
        let mut sidecar = Sidecar::empty();
        sidecar.store(0, 0xCAFE, &bag);
        let back = Sidecar::from_bytes(&sidecar.to_bytes()).unwrap();
        assert_eq!(back.bag_for(0, 0xCAFE), Some(&bag), "the sidecar keeps both stacks");
    }

    #[test]
    fn the_projection_puts_key_items_first_then_the_stacks_in_order() {
        let mut bag = Bag::new(Rules::default());
        let bulma = bag.add(2, 0, normal(2)).unwrap();
        let ball = bag.add(33, 0, Class::of(33, false)).unwrap();
        let scouter = bag.add(16, 0, normal(16)).unwrap();
        assert_eq!(bag.project(None), [ball, bulma, scouter]);
        assert_eq!(
            bag.project(Some(scouter)),
            [scouter, ball, bulma],
            "the staged stack goes first"
        );
    }

    #[test]
    fn the_projection_holds_twenty_one_entries_until_the_bag_is_full() {
        let bag = bag_with(30);
        assert_eq!(bag.project(None).len(), PROJECTED);
        let full = bag_with(60);
        assert!(full.is_full());
        assert_eq!(full.project(None).len(), LIST_FULL, "the original refuses the next add");
    }

    #[test]
    fn a_hidden_stack_can_be_staged_into_the_list() {
        let bag = bag_with(25);
        let hidden = bag.stacks()[24].id;
        assert!(!bag.project(None).contains(&hidden));
        let staged = bag.project(Some(hidden));
        assert_eq!(staged[0], hidden);
        assert_eq!(staged.len(), PROJECTED);
    }

    #[test]
    fn property_the_projection_never_exceeds_its_cap_and_shows_every_key_item_first() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..300 {
            let rules = Rules { slots: 1 + rng.below(60) as u16, stack: 1 + rng.below(99) as u16 };
            let mut bag = Bag::new(rules);
            for _ in 0..rng.below(200) {
                let object = rng.below(40) as u8;
                let class = Class::of(object, object == 16);
                let _ = bag.add(object, 0, class);
            }
            let order = bag.project(None);
            let cap = if bag.is_full() { LIST_FULL } else { PROJECTED };
            assert!(order.len() <= cap && order.len() <= LIST_FULL);
            let keys = bag.stacks().iter().filter(|s| s.class.key).count();
            let shown_keys =
                order.iter().take_while(|id| bag.stack(**id).is_some_and(|s| s.class.key)).count();
            assert_eq!(shown_keys, keys.min(order.len()), "key items are listed first");
            for id in &order {
                assert!(bag.stack(*id).is_some(), "every entry is a stack of the bag");
            }
            let mut ids = order.clone();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), order.len(), "no stack is listed twice");
        }
    }

    #[test]
    fn property_units_are_kept_by_adds_and_takes() {
        let mut rng = Rng(0xDEAD_BEEF_1234_5678);
        for _ in 0..200 {
            let mut bag = Bag::new(Rules { slots: 60, stack: 1 + rng.below(99) as u16 });
            let mut expected: u32 = 0;
            for _ in 0..300 {
                if rng.below(3) > 0 {
                    let object = rng.below(40) as u8;
                    if bag.add(object, 0, Class::of(object, false)).is_ok() {
                        expected += 1;
                    }
                } else if let Some(stack) = bag.stacks().first().copied() {
                    let units = 1 + rng.below(5) as u16;
                    let removed = bag.take(stack.id, units);
                    assert!(removed >= 1 && removed <= units);
                    expected -= u32::from(removed);
                }
                assert_eq!(bag.units(), expected);
                assert!(bag.stacks().iter().all(|s| s.count >= 1 && s.count <= bag.rules().stack));
                assert!(bag.stacks().len() <= usize::from(bag.rules().slots));
            }
        }
    }

    #[test]
    fn the_sidecar_round_trips_and_a_checksum_selects_its_bag() {
        let bag = bag_with(5);
        let mut sidecar = Sidecar::empty();
        sidecar.store(1, 0xBEEF, &bag);
        let bytes = sidecar.to_bytes();
        let back = Sidecar::from_bytes(&bytes).unwrap();
        assert_eq!(back, sidecar);
        assert_eq!(back.bag_for(1, 0xBEEF), Some(&bag));
        assert_eq!(back.bag_for(1, 0xBEEE), None, "another save's checksum has no bag");
        assert_eq!(back.bag_for(0, 0xBEEF), None, "another slot has no bag");
    }

    #[test]
    fn a_bag_with_repeated_ids_or_empty_stacks_is_not_used() {
        let good = bag_with(3);
        assert!(good.is_consistent());
        assert!(Bag::from_bytes(&good.to_bytes()).is_ok());
        let mut twice = good.clone();
        twice.stacks[1].id = twice.stacks[0].id;
        assert!(!twice.is_consistent());
        assert!(Bag::from_bytes(&twice.to_bytes()).is_err(), "a replay's bag must be sound");
        let mut empty = good.clone();
        empty.stacks[2].count = 0;
        assert!(!empty.is_consistent());
        let mut sidecar = Sidecar::empty();
        sidecar.store(0, 1, &good);
        sidecar.store(1, 2, &twice);
        let back = Sidecar::from_bytes(&sidecar.to_bytes()).unwrap();
        assert!(back.bag_for(0, 1).is_some(), "the sound bag stays");
        assert!(back.bag_for(1, 2).is_none(), "the damaged one is left out and its save rebuilds");
    }

    #[test]
    fn a_sidecar_of_another_version_or_garbage_is_refused() {
        let mut sidecar = Sidecar::empty();
        sidecar.version = 9;
        assert!(Sidecar::from_bytes(&sidecar.to_bytes()).unwrap_err().contains("version 9"));
        assert!(Sidecar::from_bytes(b"\x01\x02").is_err());
    }
}
