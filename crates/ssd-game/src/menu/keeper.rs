//! The keeper of the bag: it binds the bag to the save the game runs on, keeps the original's item
//! list in step with the bag, and writes the list the HD stages for a use. Pure: it reads work RAM
//! and the battery save it is given, and returns the host events the frame must apply.
//!
//! The original adds an entry for each unit it takes (a pickup, a purchase, a battle drop) and removes
//! one when a unit is used, dropped or sold. The keeper compares the list with the one it last wrote
//! (a difference of the two sequences, [`diff`]): an added entry is a unit for the bag, a removed one
//! is a unit gone from its stack. Then it writes the list back as the bag projects it
//! ([`Bag::project`]), so the original always sees the list the bag implies.

use crate::data as d;
use ssd_menus::Labels;
use ssd_menus::bag::{Bag, Class, LIST_FULL, Rules, Sidecar};
use ssd_oracle::HostEvent;
use ssd_save::{CHECKSUM_OFFSET, IMAGE_LEN, PAGE_LEN, SLOT_BASES, SaveFile, SlotStatus};

/// The tag of the application event that binds the bag: its payload is the bag's bytes.
pub const BIND_TAG: u8 = 1;
/// The tag of the application event of the item spawner: its payload is the object and the count, so a replay puts
/// the same units in the bag.
pub const SPAWN_TAG: u8 = 3;
/// Objects without the story bit that the story hands out or takes back itself, so that one spawned early can upset
/// it: the "Moon" card (gone after the duel with Vegeta) and the Dragon Radar (given on Namek when it was not picked
/// up). The item spawner warns before them as before a key item.
const SPAWNER_STORY: [u8; 2] = [20, 23];
/// The bus address of the item list's state words (`item_extra`), one word an entry.
const EXTRA: u32 = d::ITEM_EXTRA;
/// Offsets of the list and the count inside a save slot: the page starts at `$00:0100`, the image at
/// `$7E:2800`, and the state words lie in the image.
const PAGE_COUNT: usize = (d::ITEM_COUNT - 0x7E_0100) as usize;
const PAGE_ZENI: usize = (d::ZENI - 0x7E_0100) as usize;
const IMAGE_EXTRA: usize = (EXTRA - 0x7E_2800) as usize;
/// The bytes of the list: 24 entries of two bytes, and the state words of the same entries.
const LIST_BYTES: usize = 2 * LIST_FULL;
/// Objects of the game (ids 0 to 39).
const OBJECTS: usize = 40;

/// One entry of the list as the keeper last wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shown {
    id: u32,
    object: u8,
    extra: u16,
}

/// The save the bag belongs to: its slot and the checksum word the bag was last saved with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Save {
    slot: u8,
    checksum: u16,
}

/// The bag of a run and the list the original sees of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keeper {
    rules: Rules,
    bag: Option<Bag>,
    save: Option<Save>,
    shown: Vec<Shown>,
    /// The stack a use staged at the top of the list, kept there until the use is over ([`Keeper::release`]): the
    /// original's effect finds its entry by its row, so the list does not move under it (a wish of Porunga changes the
    /// entry's state word, and the list written back in the bag's order would put another object on that row).
    front: Option<u32>,
}

impl Keeper {
    /// A keeper with no bag yet: it binds when the game has a save or a list to keep.
    #[must_use]
    pub fn new(rules: Rules) -> Self {
        Self { rules, bag: None, save: None, shown: Vec::new(), front: None }
    }

    /// Whether the bag is bound.
    #[must_use]
    pub fn bound(&self) -> bool {
        self.bag.is_some()
    }

    /// The save slot and its checksum word the bag is bound to; `None` for a bag of a new game (or none).
    #[must_use]
    pub fn bound_save(&self) -> Option<(u8, u16)> {
        self.save.map(|save| (save.slot, save.checksum))
    }

    /// The bag, once bound.
    #[must_use]
    pub fn bag(&self) -> Option<&Bag> {
        self.bag.as_ref()
    }

    /// Binds the bag to the save the game runs on, when it is not bound yet. The game runs on a save
    /// when its work RAM is the save's copy: the item list, the count and the money of a slot match.
    /// Then the slot's sidecar bag is taken if the sidecar has one for the slot's checksum; a save
    /// changed by another way rebuilds the bag from the list. Without such a slot, a `fresh` bind (a
    /// new game in the field) rebuilds the bag from the list too. Returns the bind event and the
    /// list's writes, or nothing when there is nothing to bind yet.
    pub fn bind(
        &mut self,
        wram: &[u8],
        labels: &Labels,
        sram: Option<&[u8]>,
        sidecar: &Sidecar,
        fresh: bool,
    ) -> Vec<HostEvent> {
        if self.bag.is_some() {
            return Vec::new();
        }
        let listed = read_list(wram);
        let matched = sram.and_then(|sram| {
            (0..3_u8)
                .find(|&slot| good(sram, slot) && slot_matches(sram, slot, wram))
                .map(|slot| (slot, checksum_of(sram, slot)))
        });
        let bag = match matched {
            Some((slot, checksum)) => {
                self.save = Some(Save { slot, checksum });
                sidecar
                    .bag_for(slot, checksum)
                    .cloned()
                    .map(|mut bag| {
                        // The options of this run apply to the bag it saved.
                        bag.set_rules(self.rules);
                        // A bag saved under older rules is brought up to today's (state objects stack).
                        bag.coalesce();
                        bag
                    })
                    .unwrap_or_else(|| self.rebuild(&listed, labels))
            }
            None if fresh && list_is_sane(wram, labels) => self.rebuild(&listed, labels),
            None => return Vec::new(),
        };
        let payload = bag.to_bytes();
        self.bag = Some(bag);
        let mut events = vec![HostEvent::App { tag: BIND_TAG, payload }];
        events.extend(self.project(wram, labels, None));
        events
    }

    /// Binds to a bag the replay recorded (its bind event's payload), instead of the save and the
    /// sidecar. Returns the list's writes.
    ///
    /// # Errors
    ///
    /// The payload is not a bag.
    pub fn bind_recorded(
        &mut self,
        payload: &[u8],
        wram: &[u8],
        labels: &Labels,
    ) -> Result<Vec<HostEvent>, String> {
        self.bag = Some(Bag::from_bytes(payload)?);
        Ok(self.project(wram, labels, None))
    }

    /// A bag kept beside a save state of the window ([`super::driver::Driver::session_bytes`]), bound again after the
    /// state is loaded, with the save it belonged to: the list is written as the bag projects it.
    pub fn restore(
        &mut self,
        mut bag: Bag,
        save: Option<(u8, u16)>,
        wram: &[u8],
        labels: &Labels,
    ) -> Vec<HostEvent> {
        bag.set_rules(self.rules);
        bag.coalesce();
        let payload = bag.to_bytes();
        self.bag = Some(bag);
        self.save = save.map(|(slot, checksum)| Save { slot, checksum });
        self.front = None;
        let mut events = vec![HostEvent::App { tag: BIND_TAG, payload }];
        events.extend(self.project(wram, labels, None));
        events
    }

    /// The bag as the list's entries of the original: each stack as the list shows it.
    /// The list is compared with the one the keeper last wrote; what the original added or took
    /// is the bag's, and the list is written back as the bag projects it. The writes are the host's.
    pub fn sync(
        &mut self,
        wram: &[u8],
        labels: &Labels,
        sram: Option<&[u8]>,
        sidecar: &Sidecar,
    ) -> Vec<HostEvent> {
        if self.bag.is_none() {
            return Vec::new();
        }
        // The game clears or loads the list in steps (a save being loaded, for one, passes through
        // a list of 255s): an insane list is not a change of the bag, so the bag waits for it.
        if !list_is_sane(wram, labels) {
            return Vec::new();
        }
        let listed = read_list(wram);
        let same = listed.len() == self.shown.len()
            && listed
                .iter()
                .zip(&self.shown)
                .all(|(&(object, extra), shown)| shown.object == object && shown.extra == extra);
        if same {
            return Vec::new();
        }
        // The game copied a save into work RAM (a Continue after a game over, which goes through
        // the title screen again). Of the save the bag is bound to, the list is written back as the
        // bag has it. Of another save, the game now holds that save, so the bag is the save's (the
        // sidecar's bag for its checksum, else a rebuild from its list), as at the first bind.
        let copied = sram.and_then(|sram| {
            (0..3_u8)
                .find(|&slot| good(sram, slot) && slot_matches(sram, slot, wram))
                .map(|slot| Save { slot, checksum: checksum_of(sram, slot) })
        });
        if let Some(copied) = copied {
            if self.save == Some(copied) {
                return self.project(wram, labels, None);
            }
            self.bag = None;
            return self.bind(wram, labels, sram, sidecar, false);
        }
        let Some(bag) = self.bag.as_mut() else { return Vec::new() };
        let (removed, added) = diff(&self.shown, &listed);
        // The row of the staged stack: a unit the original changes there (a new state word) stays the front.
        let front_row = self.shown.iter().position(|shown| Some(shown.id) == self.front);
        for index in removed {
            bag.take(self.shown[index].id, 1);
        }
        for index in added {
            let (object, extra) = listed[index];
            let class = class_of(labels, object);
            // A unit the bag cannot take is not kept: the list is written back without it below.
            let id = bag.add(object, extra, class);
            if let (Ok(id), true) = (id, front_row == Some(index)) {
                self.front = Some(id);
            }
        }
        let front = self.front;
        self.project(wram, labels, front)
    }

    /// The item spawner: `count` units of `object` go into the bag (as many as it has room for), and the list is
    /// written back as the bag projects it. Returns the units added and the writes.
    pub fn spawn(
        &mut self,
        wram: &[u8],
        labels: &Labels,
        object: u8,
        count: u8,
    ) -> (u8, Vec<HostEvent>) {
        let Some(bag) = self.bag.as_mut() else { return (0, Vec::new()) };
        let class = class_of(labels, object);
        let mut added = 0;
        while added < count && bag.add(object, 0, class).is_ok() {
            added += 1;
        }
        (added, self.project(wram, labels, None))
    }

    /// Whether the item spawner warns before `object`: a key item of the bag (the story bit, or a Dragon Ball), or one
    /// the story hands out itself ([`SPAWNER_STORY`]).
    #[must_use]
    pub fn story(labels: &Labels, object: u8) -> bool {
        class_of(labels, object).key || SPAWNER_STORY.contains(&object)
    }

    /// The list writes that show `front` first, as the HD stages a use: the stack goes to the first
    /// row, where the original's own use takes it.
    pub fn stage(&mut self, wram: &[u8], labels: &Labels, front: u32) -> Vec<HostEvent> {
        self.front = Some(front);
        self.project(wram, labels, Some(front))
    }

    /// Whether a use's stack is held at the top of the list.
    #[must_use]
    pub fn staged(&self) -> bool {
        self.front.is_some()
    }

    /// The use is over: the list follows the bag's order again from its next change.
    pub fn release(&mut self) {
        self.front = None;
    }

    /// Whether the bag's save changed: `Some` with the slot, the new checksum word and the bag, when
    /// the bound slot holds a new save (its checksum word changed) or a save that is the bag's list
    /// is written for the first time. The sidecar keeps that bag for the save.
    pub fn saved(&mut self, sram: Option<&[u8]>, wram: &[u8]) -> Option<(u8, u16, Bag)> {
        let sram = sram?;
        let bag = self.bag.as_ref()?;
        match self.save {
            Some(save) if good(sram, save.slot) => {
                let checksum = checksum_of(sram, save.slot);
                if checksum == save.checksum {
                    return None;
                }
                self.save = Some(Save { slot: save.slot, checksum });
                Some((save.slot, checksum, bag.clone()))
            }
            Some(_) => None,
            None => {
                let slot =
                    (0..3_u8).find(|&slot| good(sram, slot) && slot_matches(sram, slot, wram))?;
                let checksum = checksum_of(sram, slot);
                self.save = Some(Save { slot, checksum });
                Some((slot, checksum, bag.clone()))
            }
        }
    }

    /// A bag of the list's entries, as a new game's bag: each entry is a unit, and the entries the
    /// bag cannot take are left out.
    fn rebuild(&self, listed: &[(u8, u16)], labels: &Labels) -> Bag {
        let mut bag = Bag::new(self.rules);
        for &(object, extra) in listed {
            let _ = bag.add(object, extra, class_of(labels, object));
        }
        bag
    }

    /// Projects the bag (with `front` first) and writes the list when it is not the projection.
    fn project(&mut self, wram: &[u8], labels: &Labels, front: Option<u32>) -> Vec<HostEvent> {
        let Some(bag) = self.bag.as_ref() else { return Vec::new() };
        let shown: Vec<Shown> = bag
            .project(front)
            .into_iter()
            .filter_map(|id| bag.stack(id))
            .map(|stack| Shown { id: stack.id, object: stack.object, extra: stack.extra })
            .collect();
        let listed = read_list(wram);
        let same = listed.len() == shown.len()
            && listed
                .iter()
                .zip(&shown)
                .all(|(&(object, extra), shown)| shown.object == object && shown.extra == extra);
        let writes = if same { Vec::new() } else { list_writes(&shown, labels) };
        self.shown = shown;
        writes
    }
}

/// The class of an object of the list: its story bit comes from the labels.
fn class_of(labels: &Labels, object: u8) -> Class {
    let story = labels.objects.get(usize::from(object)).is_some_and(|label| label.story);
    Class::of(object, story)
}

/// Whether work RAM holds a list the game could have made: a count of at most 24 entries, each of
/// a known object. Work RAM is random until the game loads something, so a fresh bind waits for this.
fn list_is_sane(wram: &[u8], labels: &Labels) -> bool {
    let count = usize::from(wram.get(d::ITEM_COUNT as usize - 0x7E_0000).copied().unwrap_or(0));
    let objects = labels.objects.len().max(OBJECTS);
    count <= LIST_FULL && read_list(wram).iter().all(|&(object, _)| usize::from(object) < objects)
}

/// The list's entries in work RAM: the count (at most 24) and each entry's object and state word.
fn read_list(wram: &[u8]) -> Vec<(u8, u16)> {
    let count = usize::from(wram.get(d::ITEM_COUNT as usize - 0x7E_0000).copied().unwrap_or(0));
    (0..count.min(LIST_FULL))
        .map(|index| {
            let object =
                wram.get(d::ITEM_LIST as usize - 0x7E_0000 + 2 * index).copied().unwrap_or(0);
            let at = EXTRA as usize - 0x7E_0000 + 2 * index;
            let extra = u16::from_le_bytes([
                wram.get(at).copied().unwrap_or(0),
                wram.get(at + 1).copied().unwrap_or(0),
            ]);
            (object, extra)
        })
        .collect()
}

/// The writes of the list `shown` in work RAM: the entries and their state words, and the count.
fn list_writes(shown: &[Shown], labels: &Labels) -> Vec<HostEvent> {
    let mut entries = vec![0_u8; LIST_BYTES];
    let mut extras = vec![0_u8; LIST_BYTES];
    for (index, entry) in shown.iter().enumerate() {
        let attribute =
            labels.objects.get(usize::from(entry.object)).map_or(0, |label| label.attribute);
        entries[2 * index] = entry.object;
        entries[2 * index + 1] = attribute & !0x01;
        extras[2 * index..2 * index + 2].copy_from_slice(&entry.extra.to_le_bytes());
    }
    vec![
        HostEvent::Wram { address: d::ITEM_LIST, bytes: entries },
        HostEvent::Wram { address: EXTRA, bytes: extras },
        HostEvent::Wram { address: d::ITEM_COUNT, bytes: vec![shown.len() as u8] },
    ]
}

/// Pairs the entries of `shown` and `listed` that are the same in order (their longest common
/// subsequence): the indices of the shown entries that are gone, and of the listed ones that are new.
fn diff(shown: &[Shown], listed: &[(u8, u16)]) -> (Vec<usize>, Vec<usize>) {
    let (n, m) = (shown.len(), listed.len());
    let same = |i: usize, j: usize| shown[i].object == listed[j].0 && shown[i].extra == listed[j].1;
    // lengths[i][j]: the common subsequence of shown[i..] and listed[j..].
    let mut lengths = vec![vec![0_usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lengths[i][j] = if same(i, j) {
                lengths[i + 1][j + 1] + 1
            } else {
                lengths[i + 1][j].max(lengths[i][j + 1])
            };
        }
    }
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if same(i, j) {
            i += 1;
            j += 1;
        } else if lengths[i + 1][j] >= lengths[i][j + 1] {
            removed.push(i);
            i += 1;
        } else {
            added.push(j);
            j += 1;
        }
    }
    removed.extend(i..n);
    added.extend(j..m);
    (removed, added)
}

/// The checksum word of save slot `slot` of the battery save, when the slot is a good save.
#[must_use]
pub fn good_checksum(sram: &[u8], slot: u8) -> Option<u16> {
    good(sram, slot).then(|| checksum_of(sram, slot))
}

/// Whether the save slot `slot` is a good save (its mark and its checksum).
fn good(sram: &[u8], slot: u8) -> bool {
    SaveFile::from_bytes(sram)
        .ok()
        .and_then(|file| file.status(usize::from(slot)).ok())
        .is_some_and(|status| status == SlotStatus::Good)
}

/// The checksum word of a slot of the battery save.
fn checksum_of(sram: &[u8], slot: u8) -> u16 {
    let at = SLOT_BASES[usize::from(slot)] + CHECKSUM_OFFSET;
    u16::from_le_bytes([sram[at], sram[at + 1]])
}

/// Whether the save slot holds the game's list and money: the count, the entries and their state
/// words, and the money, as work RAM has them when the game runs on the save.
fn slot_matches(sram: &[u8], slot: u8, wram: &[u8]) -> bool {
    let base = SLOT_BASES[usize::from(slot)];
    let page = base + IMAGE_LEN;
    let image = base;
    let count = sram[page + PAGE_COUNT];
    let wram_count = wram.get(d::ITEM_COUNT as usize - 0x7E_0000).copied().unwrap_or(0);
    if count != wram_count || count as usize > LIST_FULL {
        return false;
    }
    let listed = read_list(wram);
    let entries_match = listed.iter().enumerate().all(|(index, &(object, extra))| {
        sram[page + 2 * index] == object
            && u16::from_le_bytes([
                sram[image + IMAGE_EXTRA + 2 * index],
                sram[image + IMAGE_EXTRA + 2 * index + 1],
            ]) == extra
    });
    let zeni = u16::from_le_bytes([sram[page + PAGE_ZENI], sram[page + PAGE_ZENI + 1]]);
    let wram_zeni = u16::from_le_bytes([
        wram.get(d::ZENI as usize - 0x7E_0000).copied().unwrap_or(0),
        wram.get(d::ZENI as usize - 0x7E_0000 + 1).copied().unwrap_or(0),
    ]);
    entries_match && zeni == wram_zeni && PAGE_LEN > PAGE_COUNT
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_menus::bag::Rules;

    fn shown(objects: &[u8]) -> Vec<Shown> {
        objects
            .iter()
            .enumerate()
            .map(|(i, &object)| Shown { id: i as u32 + 1, object, extra: 0 })
            .collect()
    }

    fn listed(objects: &[u8]) -> Vec<(u8, u16)> {
        objects.iter().map(|&object| (object, 0)).collect()
    }

    #[test]
    fn a_new_entry_at_the_end_is_the_only_added_one() {
        let (removed, added) = diff(&shown(&[2, 3, 4]), &listed(&[2, 3, 4, 9]));
        assert_eq!(removed.len(), 0);
        assert_eq!(added, [3]);
    }

    #[test]
    fn a_compacted_entry_is_the_removed_one() {
        let (removed, added) = diff(&shown(&[2, 3, 4]), &listed(&[2, 4]));
        assert_eq!(removed, [1]);
        assert_eq!(added.len(), 0);
    }

    #[test]
    fn a_removal_and_an_addition_in_one_frame_are_both_found() {
        let (removed, added) = diff(&shown(&[2, 3, 4]), &listed(&[4, 7]));
        assert_eq!(removed, [0, 1]);
        assert_eq!(added, [1]);
    }

    #[test]
    fn the_same_list_has_no_difference() {
        assert_eq!(diff(&shown(&[1, 2]), &listed(&[1, 2])), (Vec::new(), Vec::new()));
    }

    /// Work RAM with the list of `objects` (state words zero), the count and the money.
    fn wram_with(objects: &[u8], zeni: u16) -> Vec<u8> {
        let mut ram = vec![0_u8; 0x2_0000];
        ram[d::ITEM_COUNT as usize - 0x7E_0000] = objects.len() as u8;
        for (i, &object) in objects.iter().enumerate() {
            ram[d::ITEM_LIST as usize - 0x7E_0000 + 2 * i] = object;
        }
        let at = d::ZENI as usize - 0x7E_0000;
        ram[at..at + 2].copy_from_slice(&zeni.to_le_bytes());
        ram
    }

    fn apply(ram: &mut [u8], events: &[HostEvent]) {
        for event in events {
            if let HostEvent::Wram { address, bytes } = event {
                let at = *address as usize - 0x7E_0000;
                ram[at..at + bytes.len()].copy_from_slice(bytes);
            }
        }
    }

    #[test]
    fn a_fresh_bind_takes_the_list_and_the_list_is_written_back_as_the_bag_projects_it() {
        let labels = Labels::default();
        let mut ram = wram_with(&[2, 2, 16], 40);
        let mut keeper = Keeper::new(Rules::default());
        let events = keeper.bind(&ram, &labels, None, &Sidecar::empty(), true);
        assert!(matches!(events[0], HostEvent::App { tag: BIND_TAG, .. }));
        apply(&mut ram, &events);
        let bag = keeper.bag().unwrap();
        assert_eq!(bag.stacks().len(), 2, "two Bulmas are one stack");
        assert_eq!(bag.units_of(2), 2);
        assert_eq!(read_list(&ram), [(2, 0), (16, 0)], "the list shows the stacks");
        assert!(
            keeper.sync(&ram, &labels, None, &Sidecar::empty()).is_empty(),
            "nothing changed since"
        );
    }

    #[test]
    fn a_pickup_is_drained_into_the_bag_and_the_list_keeps_its_projection() {
        let labels = Labels::default();
        let mut ram = wram_with(&[2], 0);
        let mut keeper = Keeper::new(Rules::default());
        let events = keeper.bind(&ram, &labels, None, &Sidecar::empty(), true);
        apply(&mut ram, &events);
        // The original adds a Bulma (a second entry) and a Scouter.
        ram[d::ITEM_LIST as usize - 0x7E_0000 + 2] = 2;
        ram[d::ITEM_LIST as usize - 0x7E_0000 + 4] = 16;
        ram[d::ITEM_COUNT as usize - 0x7E_0000] = 3;
        let events = keeper.sync(&ram, &labels, None, &Sidecar::empty());
        apply(&mut ram, &events);
        assert_eq!(keeper.bag().unwrap().units_of(2), 2);
        assert_eq!(keeper.bag().unwrap().stacks().len(), 2);
        assert_eq!(read_list(&ram), [(2, 0), (16, 0)], "the second Bulma is merged");
    }

    #[test]
    fn a_use_takes_one_unit_from_the_stack_the_list_showed_first() {
        let labels = Labels::default();
        let mut ram = wram_with(&[2, 16], 0);
        let mut keeper = Keeper::new(Rules::default());
        let events = keeper.bind(&ram, &labels, None, &Sidecar::empty(), true);
        apply(&mut ram, &events);
        keeper.bag.as_mut().unwrap().add(2, 0, Class::of(2, false)).unwrap();
        let events = keeper.sync(&ram, &labels, None, &Sidecar::empty());
        apply(&mut ram, &events);
        let staged = keeper.bag().unwrap().stacks()[0].id;
        let events = keeper.stage(&ram, &labels, staged);
        apply(&mut ram, &events);
        assert_eq!(read_list(&ram)[0].0, 2, "the staged stack is the first row");
        // The original's use removes row 0.
        ram[d::ITEM_LIST as usize - 0x7E_0000] = 16;
        ram[d::ITEM_COUNT as usize - 0x7E_0000] = 1;
        let events = keeper.sync(&ram, &labels, None, &Sidecar::empty());
        apply(&mut ram, &events);
        assert_eq!(keeper.bag().unwrap().units_of(2), 1, "one Bulma was used");
        assert_eq!(keeper.bag().unwrap().units_of(16), 1);
    }

    #[test]
    fn a_full_bag_writes_back_the_list_without_the_unit_it_cannot_take() {
        let labels = Labels::default();
        let rules = Rules { slots: 1, stack: 99 };
        let mut ram = wram_with(&[2], 0);
        let mut keeper = Keeper::new(rules);
        let events = keeper.bind(&ram, &labels, None, &Sidecar::empty(), true);
        apply(&mut ram, &events);
        ram[d::ITEM_LIST as usize - 0x7E_0000 + 2] = 16;
        ram[d::ITEM_COUNT as usize - 0x7E_0000] = 2;
        let events = keeper.sync(&ram, &labels, None, &Sidecar::empty());
        apply(&mut ram, &events);
        assert_eq!(keeper.bag().unwrap().stacks().len(), 1, "the Scouter has no slot");
        assert_eq!(read_list(&ram), [(2, 0)], "and the list does not show it");
    }

    /// Sets the state word of list entry `index` in work RAM.
    fn set_extra(ram: &mut [u8], index: usize, extra: u16) {
        let at = EXTRA as usize - 0x7E_0000 + 2 * index;
        ram[at..at + 2].copy_from_slice(&extra.to_le_bytes());
    }

    #[test]
    fn two_fresh_porunga_are_one_stack_and_a_used_wish_splits_it_in_the_list() {
        let labels = Labels::default();
        let mut ram = wram_with(&[15, 15], 0);
        set_extra(&mut ram, 0, 0x0300);
        set_extra(&mut ram, 1, 0x0300);
        let mut keeper = Keeper::new(Rules::default());
        let events = keeper.bind(&ram, &labels, None, &Sidecar::empty(), true);
        apply(&mut ram, &events);
        assert_eq!(keeper.bag().unwrap().stacks().len(), 1, "two fresh Porunga are one stack");
        assert_eq!(keeper.bag().unwrap().units_of(15), 2);
        assert_eq!(read_list(&ram), [(15, 0x0300)], "the list shows the stack as one entry");
        // The original uses one wish: the entry's state word goes to two wishes left.
        set_extra(&mut ram, 0, 0x0200);
        let events = keeper.sync(&ram, &labels, None, &Sidecar::empty());
        apply(&mut ram, &events);
        let bag = keeper.bag().unwrap();
        assert_eq!(bag.stacks().len(), 2, "one unit takes the new state, the rest keep the old");
        assert_eq!(bag.units_of(15), 2);
        assert_eq!(read_list(&ram), [(15, 0x0300), (15, 0x0200)], "the list shows both stacks");
        let mut sidecar = Sidecar::empty();
        sidecar.store(0, 7, bag);
        let back = Sidecar::from_bytes(&sidecar.to_bytes()).unwrap();
        assert_eq!(back.bag_for(0, 7), Some(bag), "the sidecar keeps both stacks");
        assert!(
            keeper.sync(&ram, &labels, None, &Sidecar::empty()).is_empty(),
            "nothing changed since"
        );
    }

    #[test]
    fn a_bag_is_not_bound_before_the_game_has_a_list_to_keep() {
        let labels = Labels::default();
        let ram = wram_with(&[], 0);
        let mut keeper = Keeper::new(Rules::default());
        assert_eq!(keeper.bind(&ram, &labels, None, &Sidecar::empty(), false).len(), 0);
        assert!(!keeper.bound());
    }

    #[test]
    fn the_recorded_bag_binds_without_the_save() {
        let labels = Labels::default();
        let mut bag = Bag::new(Rules::default());
        bag.add(33, 0, Class::of(33, false)).unwrap();
        let ram = wram_with(&[], 0);
        let mut keeper = Keeper::new(Rules::default());
        let writes = keeper.bind_recorded(&bag.to_bytes(), &ram, &labels).unwrap();
        assert_eq!(keeper.bag(), Some(&bag));
        assert_eq!(writes.len(), 3, "the list of the one Dragon Ball is written");
    }
}
