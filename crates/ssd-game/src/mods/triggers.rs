//! The pack triggers (M10, Task 6, `docs/mods/triggers.md`): a pack's `[[triggers]]` file starts a scene when the leader stands
//! in a rectangle of a map and its conditions hold. The set's triggers are one data item of the image (`ssd_mod::trigger`,
//! `Family::Triggers` slot 0), read here through the machine's bus.
//!
//! The evaluator is a Patch hook at `03:F163`, the per-frame trigger dispatch (`docs/re/data/maps.md`, section 6), entered
//! from `00:97A4` every frame. It runs before the original's gates (`$0716`, the game mode, the flight gate, the town gate):
//!
//! - It evaluates only while the party is free: the original's gates of the dispatch (`$0716` zero, `game_mode` bit 7 clear,
//!   not a town screen, `$01A1` bit 7 clear), the field state at 0 (`$25`: "0 free, the party walks or flies",
//!   `docs/re/data/menus.md`) and no scene on (`$1648` bit 7, the test of the dispatcher at `03:AC2B`).
//! - It does not evaluate on a frame where the frame counter `$4F` is a multiple of 64: the original runs its zone check
//!   (`02:D37D`) on those frames at the start of `03:F163`, and a start would skip that call. The trigger waits a frame.
//! - The first trigger of the map (in the order of the set) whose rectangle holds the leader's position (`$01A2`, `$01A4`)
//!   and whose conditions hold starts its scene. It writes what `03:F7DE` writes: the scene's number to `scene_id` (`$01D8`,
//!   one byte) and `$80` to `state_1648`. The dispatch of that frame then returns at once (`03:F197`, the RTL of its own
//!   gate): no original routine of the map starts a second scene in the same frame.
//! - A trigger does not fire every frame while the party stands in it. The number of the trigger started (from 1) is kept in
//!   `$7F:FCF8`, under the extended flags' marker (`scene_ext`), as the stay. While the leader is inside that trigger's
//!   rectangle on its map, no trigger starts; when the leader leaves it, the stay is cleared and the set is evaluated again.
//!   The stay is not saved: after a Continue, the trigger the party stood in last keeps its stay until the party leaves it.
//!
//! A condition is tested as `ssd_mod::trigger::Test` says: a story flag (`$0170`), an extended flag (`scene_ext`), a party
//! member (`03:F021`'s search: a record of the party with its roster byte at `$C0` or more and that character id), or the
//! flight bit (`$01BC` bit 0); a `not` negates it. A set without triggers, or a map without any, declines at once.

use super::redirect::overlay;
use super::scene_ext::{self, Bus, Cpu};
use crate::data as d;
use ssd_mod::Family;
use ssd_mod::trigger::{CONDITION_LEN, HEADER_LEN, KIND_NOT, Test};
use ssd_oracle::Ctx;

/// `03:F163`: the per-frame trigger dispatch (RTL), the key of the hook.
pub(super) const DISPATCH_KEY: u32 = 0x03_F163;
/// `03:F197`: the RTL of the dispatch's `$0716` gate, where a start resumes (the original's own return).
const RETURN: u32 = 0x03_F197;
/// `$4F`, the frame counter the NMI adds to each frame. On a frame where its low six bits are zero the original runs its zone
/// check (`02:D37D`) at the start of the dispatch, so no trigger starts on that frame.
const FRAME: u32 = 0x7E_004F;
const FRAME_MASK: u8 = 0x3F;
/// The stay: the number of the trigger the party stands in (from 1), 0 for none. It is valid under the extended flags'
/// marker only (`scene_ext`), the spare bytes before it.
const STAY: u32 = 0x7F_FCF8;
/// The bit of `state_1648` that a scene on sets, and of the game mode and the town gate that the dispatch tests.
const SCENE_ON: u8 = 0x80;
const MODE_JUST_ENTERED: u8 = 0x80;
const TOWN_SCREEN: u8 = 0x80;
/// The party records (`$0B00`): nine records of 32 bytes; the roster byte at `$C0` or more is a member, and the character id
/// is at `+$1C` (the search of `03:F021`).
const PARTY_SLOTS: u32 = 9;
const PARTY_RECORD: u32 = 0x20;
const CHARACTER_AT: u32 = 0x1C;
const IN_PARTY: u8 = 0xC0;

/// One trigger of the set as the image holds it: its number from 1 (its position in the item), its map, its scene, its
/// rectangle and its conditions. A trigger whose conditions do not decode has `None` and never fires.
struct Record {
    index: u8,
    map: u8,
    scene: u8,
    rect: [u16; 4],
    conditions: Option<Vec<(Test, bool)>>,
}

/// What the dispatch does for the frame: the number of the trigger the party stands in afterwards (0 for none), and the
/// scene of the trigger that starts, if one does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Decision {
    stay: u8,
    start: Option<u8>,
}

/// The little-endian word at `address`.
fn word(bus: &impl Bus, address: u32) -> u16 {
    u16::from_le_bytes([bus.read(address), bus.read(address + 1)])
}

/// The records of the item at `list` (the layout of `ssd_mod::trigger`).
fn read_set(bus: &impl Bus, list: u32) -> Vec<Record> {
    let count = bus.read(list);
    let mut at = list + 1;
    let mut records = Vec::with_capacity(usize::from(count));
    for index in 1..=count {
        let rect = [word(bus, at + 2), word(bus, at + 4), word(bus, at + 6), word(bus, at + 8)];
        let (map, scene, conditions_len) = (bus.read(at), bus.read(at + 1), bus.read(at + 10));
        at += HEADER_LEN as u32;
        let mut conditions = Some(Vec::with_capacity(usize::from(conditions_len)));
        for _ in 0..conditions_len {
            let kind = bus.read(at);
            let test = Test::decode(kind & !KIND_NOT, word(bus, at + 1));
            conditions = match (conditions, test) {
                (Some(mut list), Some(test)) => {
                    list.push((test, kind & KIND_NOT != 0));
                    Some(list)
                }
                _ => None,
            };
            at += CONDITION_LEN as u32;
        }
        records.push(Record { index, map, scene, rect, conditions });
    }
    records
}

/// Whether the position `(x, y)` is in the rectangle `[x, y, width, height]`: from its `x` up to, not including, `x + width`,
/// and likewise for `y`.
fn inside(rect: [u16; 4], (x, y): (u16, u16)) -> bool {
    let [left, top, width, height] = rect.map(u32::from);
    let (x, y) = (u32::from(x), u32::from(y));
    left <= x && x < left + width && top <= y && y < top + height
}

/// Whether character `character` is in the party: a record of the nine with the roster byte at `$C0` or more and that
/// character id (the test of the original's search, `03:F021`).
fn member(bus: &impl Bus, character: u8) -> bool {
    (0..PARTY_SLOTS).any(|slot| {
        let record = d::PARTY_RECORDS + PARTY_RECORD * slot;
        bus.read(record) >= IN_PARTY && bus.read(record + CHARACTER_AT) == character
    })
}

/// Whether `test` holds, before any `not`.
fn test_holds(bus: &impl Bus, test: Test) -> bool {
    match test {
        Test::Flag(flag) => bus.read(d::STORY_FLAGS + u32::from(flag / 8)) & (1 << (flag % 8)) != 0,
        Test::FlagX(flag) => scene_ext::is_set(bus, flag),
        Test::Member(character) => member(bus, character),
        Test::Flying => bus.read(d::GATE_FLAG_01BC) & 1 != 0,
    }
}

/// What the dispatch does for this frame: the trigger the party stands in afterwards, and the scene of the first trigger of
/// `map` whose rectangle holds the leader and whose conditions hold, unless the party stands in a trigger already (its stay).
fn decide(bus: &impl Bus, records: &[Record], map: u8, leader: (u16, u16), stay: u8) -> Decision {
    let standing = records
        .iter()
        .any(|record| record.index == stay && record.map == map && inside(record.rect, leader));
    if stay != 0 && standing {
        return Decision { stay, start: None };
    }
    for record in records.iter().filter(|record| record.map == map) {
        let holds = inside(record.rect, leader)
            && record.conditions.as_ref().is_some_and(|conditions| {
                conditions.iter().all(|&(test, negated)| test_holds(bus, test) != negated)
            });
        if holds {
            return Decision { stay: record.index, start: Some(record.scene) };
        }
    }
    Decision { stay: 0, start: None }
}

/// Whether the party is free for a scene: the gates of the dispatch, the field state at 0 and no scene on.
fn party_free(bus: &impl Bus) -> bool {
    bus.read(d::CONTROL_0716) == 0
        && bus.read(d::GAME_MODE) & MODE_JUST_ENTERED == 0
        && bus.read(d::COPY_GATE) & TOWN_SCREEN == 0
        && bus.read(d::FIELD_STATE) == 0
        && bus.read(d::STATE_1648) & SCENE_ON == 0
}

/// The stay as it is kept, 0 when the marker is not in place (then no state is valid).
fn read_stay(bus: &impl Bus) -> u8 {
    if scene_ext::marked(bus) { bus.read(STAY) } else { 0 }
}

/// Keeps `stay`. The marker is written first when a stay is kept and the marker is not in place (`scene_ext::mark`).
fn write_stay(bus: &mut impl Bus, stay: u8) {
    if !scene_ext::marked(&*bus) {
        if stay == 0 {
            return;
        }
        scene_ext::mark(bus);
    }
    bus.write(STAY, stay);
}

/// Evaluates the set for the frame and writes what it does: the stay, and for a start the scene (`scene_id` and
/// `state_1648`, as `03:F7DE` writes them). Returns the scene that starts, if one does.
fn evaluate(ctx: &mut Ctx<'_>) -> Option<u8> {
    let list = overlay(ctx, Family::Triggers, 0)?;
    let mut bus = Cpu(ctx);
    if !party_free(&bus) || bus.read(FRAME) & FRAME_MASK == 0 {
        return None;
    }
    let records = read_set(&bus, list);
    let map = bus.read(d::MAP_ID);
    let leader = (word(&bus, d::FOLLOW_TARGET_X), word(&bus, d::FOLLOW_TARGET));
    let stay = read_stay(&bus);
    let decision = decide(&bus, &records, map, leader, stay);
    if decision.stay != stay {
        write_stay(&mut bus, decision.stay);
    }
    let scene = decision.start?;
    bus.write(d::SCENE_ID, scene);
    bus.write(d::STATE_1648, SCENE_ON);
    Some(scene)
}

/// `03:F163`: the trigger dispatch (see the module documentation). A start resumes at the RTL `03:F197`, so the dispatch of
/// the frame does not run; every other frame the original's dispatch runs as it is.
pub(super) fn dispatch_wrapper(ctx: &mut Ctx<'_>) {
    if evaluate(ctx).is_some() {
        ctx.resume_at(RETURN);
    } else {
        ctx.resume_at(DISPATCH_KEY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_mod::trigger::{Condition, Trigger};
    use std::collections::BTreeMap;

    /// A bus of sparse bytes: an unwritten address reads zero.
    #[derive(Default)]
    struct Memory(BTreeMap<u32, u8>);

    impl Bus for Memory {
        fn read(&self, address: u32) -> u8 {
            self.0.get(&address).copied().unwrap_or(0)
        }

        fn write(&mut self, address: u32, value: u8) {
            self.0.insert(address, value);
        }
    }

    /// The bytes of `trigger` (the layout of `ssd_mod::trigger`) at `list` in `memory`.
    fn store(memory: &mut Memory, list: u32, triggers: &[Trigger]) {
        let bytes = ssd_mod::trigger::encode(triggers).unwrap();
        for (k, &byte) in bytes.iter().enumerate() {
            memory.write(list + k as u32, byte);
        }
    }

    fn trigger(map: u8, rect: [u16; 4], conditions: &[Condition], scene: u8) -> Trigger {
        Trigger { map, rect, conditions: conditions.to_vec(), scene }
    }

    fn flag(number: u8, negated: bool) -> Condition {
        Condition { test: Test::Flag(number), negated }
    }

    #[test]
    fn a_rectangle_holds_its_left_and_top_edges_but_not_its_right_and_bottom_ones() {
        let rect = [960, 704, 128, 128];
        assert!(inside(rect, (960, 704)), "the corner");
        assert!(inside(rect, (1087, 831)), "the last pixel");
        assert!(!inside(rect, (1088, 800)), "the right edge is outside");
        assert!(!inside(rect, (1000, 832)), "the bottom edge is outside");
        assert!(!inside(rect, (959, 704)), "left of it");
        assert!(inside([0, 0, 65535, 65535], (65534, 65534)), "a whole map");
    }

    #[test]
    fn the_records_read_back_what_the_packer_wrote() {
        let list = 0x7E_9000;
        let mut memory = Memory::default();
        let triggers = [
            trigger(64, [960, 704, 128, 128], &[flag(3, false), flag(29, true)], 82),
            trigger(0, [1, 2, 3, 4], &[Condition { test: Test::Flying, negated: false }], 7),
        ];
        store(&mut memory, list, &triggers);
        let records = read_set(&memory, list);
        assert_eq!(records.len(), 2);
        assert_eq!(
            (records[0].index, records[0].map, records[0].scene),
            (1, 0, 7),
            "sorted by map"
        );
        assert_eq!(records[1].rect, [960, 704, 128, 128]);
        assert_eq!(
            records[1].conditions.as_deref(),
            Some(&[(Test::Flag(3), false), (Test::Flag(29), true)][..])
        );
    }

    #[test]
    fn the_first_trigger_of_the_map_whose_rectangle_and_conditions_hold_starts_and_no_other_map_does()
     {
        let list = 0x7E_9000;
        let mut memory = Memory::default();
        memory.write(d::STORY_FLAGS, 0b1000); // flag 3 set
        store(
            &mut memory,
            list,
            &[
                trigger(64, [0, 0, 100, 100], &[flag(29, false)], 1), // holds the leader, flag 29 clear: no
                trigger(64, [0, 0, 100, 100], &[flag(3, false)], 2), // the first that holds: starts scene 2
                trigger(64, [0, 0, 100, 100], &[], 3),
                trigger(65, [0, 0, 100, 100], &[], 4), // another map
            ],
        );
        let records = read_set(&memory, list);
        let decision = decide(&memory, &records, 64, (5, 5), 0);
        assert_eq!(decision, Decision { stay: 2, start: Some(2) });
        let elsewhere = decide(&memory, &records, 66, (5, 5), 0);
        assert_eq!(elsewhere, Decision { stay: 0, start: None }, "no trigger of map 66");
        let outside = decide(&memory, &records, 64, (100, 5), 0);
        assert_eq!(outside, Decision { stay: 0, start: None }, "the right edge is outside");
    }

    #[test]
    fn a_trigger_does_not_start_again_while_the_party_stands_in_it_and_the_stay_ends_when_it_leaves()
     {
        let list = 0x7E_9000;
        let mut memory = Memory::default();
        store(
            &mut memory,
            list,
            &[trigger(64, [0, 0, 100, 100], &[], 9), trigger(64, [50, 50, 10, 10], &[], 8)],
        );
        let records = read_set(&memory, list);
        let first = decide(&memory, &records, 64, (5, 5), 0);
        assert_eq!(first, Decision { stay: 1, start: Some(9) });
        let standing = decide(&memory, &records, 64, (6, 6), first.stay);
        assert_eq!(standing, Decision { stay: 1, start: None }, "still in it: nothing starts");
        let into_the_other = decide(&memory, &records, 64, (55, 55), first.stay);
        assert_eq!(
            into_the_other,
            Decision { stay: 1, start: None },
            "the party is still inside trigger 1"
        );
        let left = decide(&memory, &records, 64, (150, 150), first.stay);
        assert_eq!(left, Decision { stay: 0, start: None }, "out of it: the stay ends");
        let back = decide(&memory, &records, 64, (5, 5), left.stay);
        assert_eq!(back, Decision { stay: 1, start: Some(9) }, "entering again starts it again");
    }

    #[test]
    fn a_stay_on_another_map_or_an_unknown_number_does_not_hold_the_party() {
        let list = 0x7E_9000;
        let mut memory = Memory::default();
        store(
            &mut memory,
            list,
            &[trigger(64, [0, 0, 100, 100], &[], 9), trigger(70, [0, 0, 100, 100], &[], 8)],
        );
        let records = read_set(&memory, list);
        assert_eq!(
            decide(&memory, &records, 70, (5, 5), 1),
            Decision { stay: 2, start: Some(8) },
            "trigger 1 is on map 64: it does not hold the party on map 70"
        );
        assert_eq!(
            decide(&memory, &records, 64, (5, 5), 7),
            Decision { stay: 1, start: Some(9) },
            "no trigger 7: the stay is cleared and trigger 1 starts"
        );
    }

    #[test]
    fn the_conditions_are_flags_extended_flags_members_and_flight_and_not_negates_each() {
        let mut memory = Memory::default();
        let flagx = Condition { test: Test::FlagX(12), negated: false };
        let flying = Condition { test: Test::Flying, negated: false };
        let member_two = Condition { test: Test::Member(2), negated: false };
        assert!(!test_holds(&memory, Test::Flying));
        memory.write(d::GATE_FLAG_01BC, 1);
        assert!(test_holds(&memory, Test::Flying), "the flight bit");
        memory.write(d::GATE_FLAG_01BC, 0x40);
        assert!(!test_holds(&memory, Test::Flying), "bit 6 is not the flight bit");
        // Extended flag 12 set under the marker.
        for (k, &byte) in b"SSDf".iter().enumerate() {
            memory.write(0x7F_FCFC + k as u32, byte);
        }
        memory.write(0x7F_FD00 + 1, 0x10); // flag 12: byte 1, bit 4
        assert!(test_holds(&memory, flagx.test));
        // A party record for character 2 with the roster byte at $C0.
        memory.write(d::PARTY_RECORDS + PARTY_RECORD + CHARACTER_AT, 2);
        assert!(!member(&memory, 2), "a roster byte below $C0 is not in the party");
        memory.write(d::PARTY_RECORDS + PARTY_RECORD, IN_PARTY);
        assert!(member(&memory, 2), "the record of character 2 is in the party");
        assert!(!member(&memory, 3));
        let conditions = [(flying.test, true), (flagx.test, false), (member_two.test, false)];
        let all = conditions.iter().all(|&(test, negated)| test_holds(&memory, test) != negated);
        assert!(all, "`not flying`, flag 12 and member 2 all hold here");
    }

    #[test]
    fn the_party_is_free_only_with_no_gate_no_hold_and_no_scene() {
        let mut memory = Memory::default();
        assert!(party_free(&memory));
        memory.write(d::FIELD_STATE, 3);
        assert!(!party_free(&memory), "a hold (a landing, a scene)");
        memory.write(d::FIELD_STATE, 0);
        memory.write(d::STATE_1648, SCENE_ON);
        assert!(!party_free(&memory), "a scene on");
        memory.write(d::STATE_1648, 0);
        memory.write(d::CONTROL_0716, 0xA1);
        assert!(!party_free(&memory), "a fade or a map change");
        memory.write(d::CONTROL_0716, 0);
        memory.write(d::COPY_GATE, 0x80);
        assert!(!party_free(&memory), "a town screen");
        memory.write(d::COPY_GATE, 0);
        memory.write(d::GAME_MODE, MODE_JUST_ENTERED);
        assert!(!party_free(&memory), "the game mode's just-entered bit");
        memory.write(d::GAME_MODE, 0);
        assert!(party_free(&memory));
    }

    #[test]
    fn a_stay_is_read_only_under_the_marker_and_written_with_it() {
        let mut memory = Memory::default();
        memory.write(STAY, 3);
        assert_eq!(read_stay(&memory), 0, "without the marker no stay is valid");
        write_stay(&mut memory, 0);
        assert!(!scene_ext::marked(&memory), "clearing with no marker writes nothing");
        write_stay(&mut memory, 3);
        assert!(scene_ext::marked(&memory), "a stay kept writes the marker");
        assert_eq!(read_stay(&memory), 3);
        write_stay(&mut memory, 0);
        assert_eq!(read_stay(&memory), 0);
    }

    #[test]
    fn the_addresses_are_the_documented_ones() {
        assert_eq!(DISPATCH_KEY, 0x03_F163);
        assert_eq!(RETURN, 0x03_F197);
        assert_eq!(FRAME, 0x7E_004F);
        assert_eq!(STAY, 0x7F_FCF8);
    }
}
