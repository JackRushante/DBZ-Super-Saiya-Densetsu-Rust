use super::*;
use crate::cheats::CheatSidecar;
use crate::draw::{Align, DrawList, Op, Rect};
use crate::layout::{HEIGHT, SMALL_TEXT, WIDTH};
use crate::mods_state::{ModState, ModsSidecar};
use crate::options::Values;
use crate::view::{CharacterLabel, ObjectLabel};

/// The table of a test character: BP of level `n` is `100 n`, but level 20 is 8050 and level 30 is 16409 (the owner's example
/// for Gohan), HP is `10 n`, KI is `2 n`, EXP starts at `n * n * 10`.
fn growth(cap: u8, min: u8) -> Growth {
    let levels = 1..=u32::from(cap);
    // Rising: 402.5 a level up to 20 (8050 at 20), then 835.9 up to 30 (16409), then 900.
    let bp: Vec<u32> = levels
        .clone()
        .map(|n| match n {
            ..=20 => n * 402 + n / 2,
            21..=30 => 8050 + (n - 20) * 8359 / 10,
            _ => 16409 + (n - 30) * 900,
        })
        .collect();
    Growth {
        bp,
        max_hp: levels.clone().map(|n| (n * 10) as u16).collect(),
        max_ki: levels.clone().map(|n| (n * 2) as u16).collect(),
        exp: levels.clone().map(|n| n * n * 10).collect(),
        byte_0e: levels.map(|n| (n % 7) as u8).collect(),
        cap,
        min,
    }
}

fn labels() -> Labels {
    let names = [
        "Goku", "Gohan", "Piccolo", "Krillin", "Tien", "Yamcha", "Chiaotzu", "Vegeta", "Nail",
        "Tsumuri", "Maima", "Dende",
    ];
    let characters = names
        .iter()
        .enumerate()
        .map(|(id, name)| CharacterLabel {
            name: (*name).to_owned(),
            face: Some(id as u8),
            class: None,
        })
        .collect();
    let objects = (0..OBJECTS as u8)
        .map(|id| ObjectLabel {
            name: format!("Obj{id}"),
            attribute: if id == 26 { 0x21 } else { 0xC9 },
            story: id == 26,
            ally: id % 2 == 0,
            help: vec![format!("Help {id}")],
            ..ObjectLabel::default()
        })
        .collect();
    let mut growth_table = vec![growth(47, 1), growth(48, 1), growth(47, 1)];
    growth_table.extend((3..7).map(|_| growth(48, 1)));
    growth_table.push(growth(47, 17)); // Vegeta
    growth_table.extend([Growth::default(), Growth::default(), Growth::default()]); // Nail, Tsumuri, Maima
    growth_table.push(growth(3, 1)); // Dende
    Labels { characters, objects, growth: growth_table, ..Labels::default() }
}

fn record(id: u8, level: u8, bp: u32) -> Record {
    let mut r = Record::blank();
    r.set_flags(0xC0);
    r.set_unit_id(id);
    r.set_character_id(id);
    r.set_level(level);
    r.set_bp(bp);
    r.set_max_hp(1);
    r.set_hp(1);
    r
}

/// A cartridge whose slot 0 holds Goku (LV 10) and Gohan (LV 20, 8050 BP, the game's own), five items and 120 zeni.
fn cartridge() -> Vec<u8> {
    let mut file = SaveFile::blank();
    let mut slot = Slot::empty();
    slot.set_record(0, &record(0, 10, 1000)).unwrap();
    slot.set_record(1, &record(1, 20, 8050)).unwrap();
    slot.set_party(&[0, 1]).unwrap();
    let items: Vec<(Item, u16)> = [2_u8, 2, 4, 33, 15]
        .iter()
        .map(|&id| (Item { id, attr: 0xC8 }, if id == 15 { 0x9600 } else { 0 }))
        .collect();
    slot.set_item_list(&items).unwrap();
    slot.set_zeni(120).unwrap();
    file.set_slot(0, &slot).unwrap();
    file.as_bytes().to_vec()
}

fn editor() -> Editor {
    Editor::open(&cartridge(), &labels(), &Sidecar::empty(), [false; 3]).unwrap()
}

/// An editor on slot 0, on Gohan.
fn on_gohan() -> Editor {
    let mut e = editor();
    e.key(Key::A);
    e.key(Key::R);
    assert_eq!(e.session().unwrap().members[e.session().unwrap().member].id, 1);
    e
}

fn gohan(e: &Editor) -> Record {
    e.session().unwrap().members[1].record
}

#[test]
fn the_picker_shows_the_slots_and_opens_a_game_only() {
    let mut e = editor();
    assert!(matches!(e.cards()[0], Card::Game { .. }));
    assert_eq!(e.cards()[1], Card::Empty);
    let Card::Game { summary, cheats } = &e.cards()[0] else { panic!() };
    assert_eq!((summary.level, summary.zeni, summary.items, *cheats), (10, 120, 5, false));
    e.key(Key::Down);
    e.key(Key::A);
    assert_eq!(e.stage(), Stage::Slots, "an empty slot does not open");
    e.key(Key::Up);
    e.key(Key::A);
    assert_eq!(e.stage(), Stage::Edit);
    assert_eq!(e.session().unwrap().members.len(), 2);
}

#[test]
fn a_damaged_slot_is_refused_with_its_line() {
    let mut sram = cartridge();
    sram[SLOT_BASES[0] + 5] ^= 0xFF;
    sram[SLOT_BASES[0] + ssd_save::BACKUP_OFFSET + 5] ^= 0xFF;
    let mut e = Editor::open(&sram, &labels(), &Sidecar::empty(), [false; 3]).unwrap();
    assert_eq!(e.cards()[0], Card::Damaged);
    e.key(Key::A);
    assert_eq!(e.stage(), Stage::Slots);
    assert_eq!(e.notice(), Some("Slot 01 is damaged and cannot be edited."));
}

#[test]
fn a_level_change_sets_the_stats_from_the_rows_and_the_bp_to_the_table_plus_the_delta() {
    let mut e = on_gohan();
    // 10 presses up from 20 to 30: BP is the table's 16409, HP and KI full, EXP at the start of the level.
    for _ in 0..10 {
        e.key(Key::Right);
    }
    let r = gohan(&e);
    assert_eq!((r.level(), r.bp()), (30, 16409));
    assert_eq!((r.max_hp(), r.hp(), r.max_ki(), r.ki(), r.exp()), (300, 300, 60, 60, 9000));
    // Down to BP, +3000 by hand.
    e.key(Key::Down);
    for _ in 0..3000 {
        e.key(Key::Right);
    }
    assert_eq!(gohan(&e).bp(), 19409);
    let g = e.labels().growth[1].clone();
    assert_eq!(delta_of(&g, &gohan(&e)), 3000);
    // Back on the level row: the delta follows the level.
    e.key(Key::Up);
    e.key(Key::Left);
    let r = gohan(&e);
    let table29 = e.labels().growth[1].bp[28];
    assert_eq!((r.level(), r.bp()), (29, table29 + 3000));
    // Y on the BP row: the level's own value.
    e.key(Key::Down);
    e.key(Key::Y);
    assert_eq!(gohan(&e).bp(), table29);
    assert_eq!(delta_of(&g, &gohan(&e)), 0);
}

#[test]
fn the_originals_level_up_adds_the_row_so_the_delta_survives_it() {
    // The game's level-up adds the row of the new level to the record's BP (`03:851B`).
    let g = growth(48, 1);
    let mut r = record(1, 20, 8050 + 3000);
    let before = delta_of(&g, &r);
    for level in 21..=30_u8 {
        let gain = g.bp[usize::from(level) - 1] - g.bp[usize::from(level) - 2];
        // The original's add, with the 24-bit wrap of the record.
        r.set_bp((r.bp() + gain) & 0xFF_FFFF);
        r.set_level(level);
    }
    assert_eq!(r.bp(), 16409 + 3000);
    assert_eq!(delta_of(&g, &r), before, "the delta is the same after ten level-ups");
    // A negative delta is kept too.
    let mut low = record(1, 20, 8050 - 1000);
    let gain = g.bp[20] - g.bp[19];
    low.set_bp(low.bp() + gain);
    low.set_level(21);
    assert_eq!(delta_of(&g, &low), -1000);
}

#[test]
fn the_caps_the_floor_and_the_fixed_levels_are_refused_with_their_lines() {
    let mut e = editor();
    e.key(Key::A);
    // Goku (cap 47) at 10: up to 47, then refused.
    for _ in 0..40 {
        e.key(Key::Right);
    }
    assert_eq!(e.session().unwrap().members[0].record.level(), 47);
    assert_eq!(e.notice(), Some("Goku stops at LV 47."));
    // Gohan stops at 48.
    e.key(Key::R);
    for _ in 0..40 {
        e.key(Key::Right);
    }
    assert_eq!(gohan(&e).level(), 48);
    assert_eq!(e.notice(), Some("Gohan stops at LV 48."));
    // The first row is LV 1.
    for _ in 0..60 {
        e.key(Key::Left);
    }
    assert_eq!(gohan(&e).level(), 1);
    assert_eq!(e.notice(), Some("Gohan starts at LV 1."));
    // Vegeta starts at 17, Dende stops at 3, Nail has no table.
    let g = |id: usize| e.labels().growth[id].clone();
    assert_eq!((g(7).min, g(11).cap, g(8).has_levels()), (17, 3, false));
}

#[test]
fn a_member_without_a_level_table_edits_the_bp_only() {
    let mut sram = SaveFile::from_bytes(&cartridge()).unwrap();
    let mut slot = sram.slot(0).unwrap();
    slot.set_record(8, &record(8, 36, 5000)).unwrap();
    slot.set_party(&[0, 8]).unwrap();
    sram.set_slot(0, &slot).unwrap();
    let mut e = Editor::open(sram.as_bytes(), &labels(), &Sidecar::empty(), [false; 3]).unwrap();
    e.key(Key::A);
    e.key(Key::R);
    e.key(Key::Right);
    assert_eq!(e.notice(), Some("Nail has no level table."));
    assert_eq!(e.session().unwrap().members[1].record.level(), 36);
    e.key(Key::Down);
    e.key(Key::Right);
    assert_eq!(e.session().unwrap().members[1].record.bp(), 5001);
}

#[test]
fn the_bp_is_held_between_zero_and_nine_million_nine_hundred_ninety_nine_thousand() {
    let mut e = on_gohan();
    e.key(Key::Down);
    e.change_bp(i64::from(BP_MAX));
    assert_eq!(gohan(&e).bp(), BP_MAX);
    assert_eq!(e.notice(), Some("BP stops at 9999999."));
    e.change_bp(-20_000_000);
    assert_eq!(gohan(&e).bp(), 0);
    // Holding the right key accelerates: after the delay the steps are 1, then 10, 100 and 1000.
    e.change_bp(1000);
    e.key(Key::Right);
    let start = gohan(&e).bp();
    for _ in 0..200 {
        e.held(1);
    }
    assert!(gohan(&e).bp() > start + 7_000, "{}", gohan(&e).bp());
    // A level change keeps the value at the cap too.
    e.change_bp(i64::from(BP_MAX));
    e.key(Key::Up);
    e.key(Key::Left);
    assert!(gohan(&e).bp() <= BP_MAX);
}

fn on_bag() -> Editor {
    let mut e = editor();
    e.key(Key::A);
    e.key(Key::L); // Party, first member: L goes to the last page (Tools)
    e
}

#[test]
fn the_pages_are_walked_with_l_and_r_through_the_members_and_the_tabs() {
    let mut e = editor();
    e.key(Key::A);
    assert_eq!(e.session().unwrap().page, Page::Party);
    e.key(Key::R);
    assert_eq!(e.session().unwrap().member, 1);
    e.key(Key::R);
    assert_eq!(e.session().unwrap().page, Page::All);
    e.key(Key::R);
    e.key(Key::R);
    assert_eq!(e.session().unwrap().page, Page::Tools);
    e.key(Key::R);
    assert_eq!((e.session().unwrap().page, e.session().unwrap().member), (Page::Party, 0));
    e.key(Key::L);
    assert_eq!(e.session().unwrap().page, Page::Tools);
    e.key(Key::L);
    e.key(Key::L);
    e.key(Key::L);
    assert_eq!((e.session().unwrap().page, e.session().unwrap().member), (Page::Party, 1));
}

#[test]
fn the_count_stepper_removes_at_zero_stops_at_ninety_nine_and_leaves_key_items_alone() {
    let mut e = editor();
    e.key(Key::A);
    e.key(Key::R);
    e.key(Key::R); // All
    // Stacks in bag order: the key items first (Dragon Ball 33), then Obj2 x2, Obj4, Obj15.
    let all = e.stacks(Page::All);
    assert_eq!(
        all.iter().map(|s| (s.object, s.count)).collect::<Vec<_>>(),
        [(33, 1), (2, 2), (4, 1), (15, 1)]
    );
    // Cursor on the Dragon Ball: one at a time.
    e.key(Key::Right);
    assert_eq!(e.notice(), Some("\"Obj33\": one at a time."));
    e.key(Key::Down);
    for _ in 0..200 {
        e.key(Key::Right);
    }
    assert_eq!(e.stacks(Page::All)[1].count, 99);
    assert_eq!(e.notice(), Some("Obj2: 99 is the most a stack holds."));
    for _ in 0..98 {
        e.key(Key::Left);
    }
    assert_eq!(e.stacks(Page::All)[1].count, 1);
    e.key(Key::Left);
    assert_eq!(e.stacks(Page::All).len(), 3, "0 removes the stack");
}

#[test]
fn y_removes_a_stack_without_a_question_but_a_story_stack_asks_with_the_cursor_on_cancel() {
    let mut e = editor();
    e.key(Key::A);
    e.key(Key::R);
    e.key(Key::R);
    e.key(Key::Down); // Obj2
    e.key(Key::Y);
    assert_eq!(e.stage(), Stage::Edit);
    assert!(e.stacks(Page::All).iter().all(|s| s.object != 2));
    e.key(Key::Up); // the Dragon Ball
    e.key(Key::Y);
    assert!(matches!(e.stage(), Stage::Ask(Ask::Remove { .. }, 1)), "the cursor starts on Cancel");
    e.key(Key::A);
    assert_eq!(e.stage(), Stage::Edit);
    assert!(e.stacks(Page::All).iter().any(|s| s.object == 33), "Cancel keeps it");
    e.key(Key::Y);
    e.key(Key::Left);
    e.key(Key::A);
    assert!(e.stacks(Page::All).iter().all(|s| s.object != 33), "Remove anyway takes it");
}

#[test]
fn the_picker_adds_units_to_the_stack_and_asks_before_a_story_object() {
    let mut e = on_bag();
    e.key(Key::X);
    assert_eq!(e.stage(), Stage::Add);
    // Pick Obj2 (id 2): R twice from 0, three units.
    e.key(Key::R);
    e.key(Key::R);
    for _ in 0..2 {
        e.key(Key::Right);
    }
    e.key(Key::A);
    assert_eq!(e.session().unwrap().bag.units_of(2), 5, "2 + 3 join the stack");
    assert_eq!(e.stage(), Stage::Add, "the picker stays");
    // Y back to 1.
    e.key(Key::Y);
    assert_eq!(e.session().unwrap().how_many, 1);
    // A story object (26, Gohan): a question, Cancel by default.
    for _ in 0..24 {
        e.key(Key::R);
    }
    assert_eq!(e.session().unwrap().pick, 26);
    e.key(Key::A);
    assert!(matches!(e.stage(), Stage::Ask(Ask::Add { object: 26, count: 1 }, 1)));
    e.key(Key::A);
    assert_eq!(e.session().unwrap().bag.units_of(26), 0, "Cancel adds nothing");
    e.key(Key::A);
    e.key(Key::Left);
    e.key(Key::A);
    assert_eq!(e.session().unwrap().bag.units_of(26), 1);
    // A key item already held is one at a time; the Moon (20) is a story object too.
    for _ in 0..7 {
        e.key(Key::R);
    }
    assert_eq!(e.session().unwrap().pick, 33);
    e.key(Key::A);
    assert_eq!(e.notice(), Some("\"Obj33\": one at a time."));
    assert!(story(e.labels(), 20) && story(e.labels(), 23) && !story(e.labels(), 2));
}

#[test]
fn a_bag_of_sixty_stacks_refuses_a_new_object_but_still_stacks_the_ones_it_has() {
    let mut e = on_bag();
    {
        let session = e.session.as_mut().unwrap();
        session.bag = Bag::new(Rules { slots: 2, stack: 99 });
        session.bag.add(2, 0, Class::of(2, false)).unwrap();
        session.bag.add(4, 0, Class::of(4, false)).unwrap();
        session.pick = 6;
    }
    e.stage = Stage::Add;
    e.key(Key::A);
    assert_eq!(e.notice(), Some("Bag full: no slot left for a new object."));
    assert_eq!(e.session().unwrap().bag.units_of(6), 0);
    e.session.as_mut().unwrap().pick = 4;
    e.key(Key::A);
    assert_eq!(e.session().unwrap().bag.units_of(4), 2);
}

#[test]
fn finishing_with_no_change_leaves_with_a_line_and_with_a_change_asks_with_the_cursor_on_keep_editing()
 {
    let mut e = editor();
    e.key(Key::A);
    e.key(Key::Start);
    assert_eq!((e.stage(), e.notice()), (Stage::Slots, Some("Nothing changed.")));
    e.key(Key::A);
    e.key(Key::B);
    assert_eq!(e.stage(), Stage::Slots, "B with nothing changed just leaves");
    e.key(Key::A);
    e.key(Key::Right);
    assert!(e.changed());
    e.key(Key::B);
    assert_eq!(e.stage(), Stage::Write(2), "Keep editing is the first choice");
    e.key(Key::A);
    assert_eq!(e.stage(), Stage::Edit);
    e.key(Key::Start);
    e.key(Key::Up);
    e.key(Key::A);
    assert_eq!(e.stage(), Stage::Slots);
    assert!(e.take_commit().is_none(), "Write was not chosen");
    e.key(Key::A);
    assert!(!e.changed(), "Discard dropped the edit");
}

#[test]
fn writing_the_slot_projects_the_bag_with_attributes_state_words_count_and_a_good_checksum() {
    let mut e = on_gohan();
    for _ in 0..10 {
        e.key(Key::Right);
    }
    e.key(Key::Down);
    for _ in 0..3000 {
        e.key(Key::Right);
    }
    // Bag: remove the Dragon Ball? No: add 2 of Obj6 and one Obj8.
    e.key(Key::R);
    e.key(Key::X);
    for _ in 0..6 {
        e.key(Key::R);
    }
    e.key(Key::Right);
    e.key(Key::A);
    e.key(Key::Y);
    e.key(Key::R);
    e.key(Key::R);
    e.key(Key::A);
    e.key(Key::B);
    e.key(Key::Start);
    assert_eq!(e.stage(), Stage::Write(2));
    e.key(Key::Up);
    e.key(Key::Up);
    e.key(Key::A);
    let commit = e.take_commit().expect("a write");
    assert_eq!((commit.slot, commit.edited.clone()), (0, vec![(1, 1)]));
    let file = SaveFile::from_bytes(&commit.sram).unwrap();
    assert_eq!(file.status(0), Ok(SlotStatus::Good));
    let slot = file.slot(0).unwrap();
    let g = slot.record(1).unwrap();
    assert_eq!((g.level(), g.bp(), g.exp(), g.max_hp()), (30, 16409 + 3000, 9000, 300));
    assert_eq!(slot.record(0).unwrap().bp(), 1000, "an unchanged member is untouched");
    let items = slot.items();
    assert_eq!(slot.page()[0xB2], items.len() as u8);
    assert_eq!(
        items.iter().map(|i| i.id).collect::<Vec<_>>(),
        [33, 2, 4, 15, 6, 8],
        "key items first, then in order"
    );
    assert_eq!(items[0].attr, 0xC8, "the ROM attribute without its low bit");
    assert_eq!(slot.item_words()[3], 0x9600, "Porunga's state word follows its entry");
    assert_eq!(slot.page()[0xB3], 6, "the rows of the first page");
    assert_eq!(slot.zeni(), 120);
    // The backup copy is the same.
    let b = SLOT_BASES[0];
    assert_eq!(
        commit.sram[b..b + ssd_save::SLOT_LEN],
        commit.sram[b + ssd_save::BACKUP_OFFSET..b + ssd_save::BACKUP_OFFSET + ssd_save::SLOT_LEN]
    );
    assert_ne!(commit.old_checksum, commit.new_checksum);
    assert_eq!(commit.new_checksum, checksum_word(&commit.sram, 0));
    assert_eq!(commit.bag.units_of(6), 2);
    // The picker shows the new slot with the CHEATS badge and "Slot 01 written."
    assert_eq!(e.stage(), Stage::Slots);
    assert_eq!(e.written(), Some(0));
    assert!(matches!(e.cards()[0], Card::Game { cheats: true, .. }));
}

#[test]
fn at_most_twenty_one_entries_are_projected_and_twenty_four_when_the_bag_is_full() {
    let mut e = editor();
    e.key(Key::A);
    {
        let session = e.session.as_mut().unwrap();
        session.bag = Bag::new(Rules { slots: 30, stack: 99 });
        for object in 0..30 {
            session.bag.add(object, 0, Class::of(object, false)).unwrap();
        }
        // Make the bag count as changed.
        session.members[0].record.set_bp(1);
    }
    e.stage = Stage::Write(0);
    e.key(Key::A);
    let commit = e.take_commit().unwrap();
    let slot = SaveFile::from_bytes(&commit.sram).unwrap().slot(0).unwrap();
    assert_eq!(slot.items().len(), 24, "a full bag shows the original's 24");
    assert_eq!(commit.bag.stacks().len(), 30);
}

#[test]
fn a_write_rekeys_every_sidecar_to_the_new_checksum_and_drops_the_wide_bp_of_the_edited_members() {
    let mut e = on_gohan();
    e.key(Key::Right);
    e.stage = Stage::Write(0);
    e.key(Key::A);
    let commit = e.take_commit().unwrap();
    let (old, new) = (commit.old_checksum, commit.new_checksum);
    let mut bags = Sidecar::empty();
    let mut cheats = CheatSidecar::empty();
    cheats.store(0, old, Cheats::default());
    let mut mods = ModsSidecar::empty();
    let mut high = vec![0_u8; HIGH_BYTES];
    high[2] = 5; // party position 1 (Gohan)
    high[0x20 + 2] = 5; // snapshot of character 1
    high[0] = 7; // party position 0 stays
    mods.store(
        0,
        old,
        ModState { flags: Some(vec![1; crate::mods_state::FLAG_BYTES]), high: Some(high) },
    );
    let mut options = SlotOptions::empty();
    let mut values = Values::new();
    values.set("difficulty.preset", 2);
    options.store(0, old, values.clone());
    // Another slot is not touched.
    options.store(2, 9, values.clone());
    commit.rekey(&mut Sidecars {
        bags: &mut bags,
        cheats: &mut cheats,
        mods: &mut mods,
        options: &mut options,
    });
    assert_eq!(bags.bag_for(0, new).map(Bag::units), Some(5));
    assert!(bags.bag_for(0, old).is_none());
    assert!(cheats.entry_for(0, old).is_none() && cheats.marked(0, new), "the badge");
    assert_eq!(options.entry_for(0, new), Some(&values));
    assert!(options.entry_for(0, old).is_none());
    assert_eq!(options.entry_for(2, 9), Some(&values));
    let state = mods.entry_for(0, new).unwrap();
    let high = state.high.as_ref().unwrap();
    assert_eq!(
        (high[0], high[2], high[0x22]),
        (7, 0, 0),
        "Gohan's high parts are gone, Goku's stay"
    );
    assert!(state.flags.is_some(), "the extended flags follow the slot");
    // A slot with no entries gets a bag and the badge, nothing else.
    let (mut bags, mut cheats, mut mods, mut options) =
        (Sidecar::empty(), CheatSidecar::empty(), ModsSidecar::empty(), SlotOptions::empty());
    commit.rekey(&mut Sidecars {
        bags: &mut bags,
        cheats: &mut cheats,
        mods: &mut mods,
        options: &mut options,
    });
    assert!(bags.bag_for(0, new).is_some() && cheats.marked(0, new));
    assert!(mods.entry_for(0, new).is_none() && options.entry_for(0, new).is_none());
}

#[test]
fn a_slot_that_changed_under_the_editor_is_not_written_over() {
    let mut e = on_gohan();
    e.key(Key::Right);
    let mut other = cartridge();
    other[SLOT_BASES[0] + 0x10] ^= 1;
    e.observe(&other);
    assert_eq!(e.stage(), Stage::Slots);
    assert_eq!(e.notice(), Some("Slot 01 changed. Open it again."));
    assert!(e.session().is_none());
}

#[test]
fn the_saved_bag_of_a_slot_is_the_one_the_editor_edits() {
    let sram = cartridge();
    let checksum = checksum_word(&sram, 0);
    let mut bag = Bag::new(Rules::default());
    bag.add(2, 0, Class::of(2, false)).unwrap();
    for _ in 0..40 {
        bag.add(7, 0, Class::of(7, false)).unwrap();
    }
    let mut sidecar = Sidecar::empty();
    sidecar.store(0, checksum, &bag);
    let mut e = Editor::open(&sram, &labels(), &sidecar, [false; 3]).unwrap();
    let Card::Game { summary, .. } = &e.cards()[0] else { panic!() };
    assert_eq!(summary.items, 41, "the units of the bag");
    e.key(Key::A);
    assert_eq!(e.session().unwrap().bag.units_of(7), 40);
    // A sidecar for another checksum is not the slot's: the bag is rebuilt from the list.
    let mut stale = Sidecar::empty();
    stale.store(0, checksum ^ 1, &bag);
    let mut e = Editor::open(&sram, &labels(), &stale, [false; 3]).unwrap();
    e.key(Key::A);
    assert_eq!(e.session().unwrap().bag.units_of(7), 0);
    assert_eq!(e.session().unwrap().bag.units_of(2), 2);
}

#[test]
fn every_screen_draws_inside_the_space_with_the_texts_of_the_design() {
    let mut e = editor();
    let texts =
        |e: &Editor| -> Vec<String> { scene(e).texts().map(|(_, _, t)| t.to_owned()).collect() };
    assert!(texts(&e).contains(&"Edit saves".to_owned()));
    e.key(Key::A);
    let party = texts(&e);
    assert!(
        party.iter().any(|t| t == "Slot 01") && party.iter().any(|t| t.starts_with("Left Right"))
    );
    e.key(Key::R);
    e.key(Key::Right);
    e.key(Key::Down);
    e.key(Key::Right);
    let party = texts(&e);
    assert!(party.iter().any(|t| t.contains("over the game's")), "{party:?}");
    for _ in 0..2 {
        e.key(Key::R);
    }
    assert!(texts(&e).iter().any(|t| t == "Bag"));
    e.key(Key::X);
    assert!(texts(&e).iter().any(|t| t == "-- Add object --"));
    e.key(Key::B);
    e.key(Key::Start);
    let sheet = texts(&e);
    assert!(sheet.iter().any(|t| t.starts_with("Write the changes to slot 01")), "{sheet:?}");
    assert!(sheet.iter().any(|t| t == "Keep editing"));
    // Nothing leaves the 1280 x 720 space.
    for op in scene(&e).ops() {
        if let Op::Text { x, y, .. } = op {
            assert!((0.0..=WIDTH).contains(x) && (0.0..=HEIGHT).contains(y), "{x} {y}");
        }
    }
    assert_eq!(title_strip().texts().count(), 2);
}

/// The horizontal extent of each text run: (left, right, y, text).
fn extents(list: &DrawList) -> Vec<(f32, f32, f32, String)> {
    list.ops()
        .iter()
        .filter_map(|op| match op {
            Op::Text { x, y, text, size, align, .. } => {
                let width = crate::layout::text_width(text, *size);
                let left = match align {
                    Align::Left => *x,
                    Align::Right => x - width,
                    Align::Centre => x - width / 2.0,
                };
                Some((left, left + width, *y, text.clone()))
            }
            _ => None,
        })
        .collect()
}

/// No two runs of one line touch each other (the old value, the arrow and the new value of a row were one blur).
fn assert_no_collision(list: &DrawList) {
    let runs = extents(list);
    for (n, a) in runs.iter().enumerate() {
        for b in &runs[n + 1..] {
            if (a.2 - b.2).abs() < 1.0 {
                assert!(a.1 + 2.0 <= b.0 || b.1 + 2.0 <= a.0, "{a:?} runs into {b:?}");
            }
        }
    }
}

#[test]
fn the_old_value_the_arrow_and_the_new_value_of_a_row_keep_their_own_columns() {
    // Six digits of the new value (28 px) and of the old one (16 px) still sit apart, whatever the figures.
    use crate::layout::text_width;
    let old_left = super::draw::OLD_RIGHT - text_width("999999", SMALL_TEXT);
    assert!(old_left > super::draw::RIGHT_WINDOW.x, "{old_left}");
    let (old_right, arrow_left) = (super::draw::OLD_RIGHT, super::draw::ARROW_LEFT);
    assert!(old_right + 8.0 <= arrow_left);
    assert!(
        super::draw::ARROW_LEFT + text_width(">", SMALL_TEXT) + 8.0
            <= super::draw::NEW_RIGHT - text_width("999999", 28.0)
    );
    // And a member whose level, HP, KI, EXP and next-level figures all change draws no run into another.
    let mut e = on_gohan();
    e.key(Key::Right);
    e.key(Key::Right);
    assert_no_collision(&scene(&e));
}

#[test]
fn the_write_sheet_and_the_story_question_keep_their_lines_in_their_windows() {
    let mut e = on_bag();
    e.key(Key::X);
    for _ in 0..3 {
        e.key(Key::R);
    }
    e.key(Key::A);
    e.key(Key::B);
    e.key(Key::Start);
    // The runs painted after the window itself (what lies under it is the editor's page).
    let inside = |list: &DrawList, window: Rect, what: &str| {
        let from = list
            .ops()
            .iter()
            .position(|op| matches!(op, Op::Window { rect } if *rect == window))
            .expect(what);
        let mut above = DrawList::new();
        // Up to the next window (the foot of the picker comes after the question).
        for op in list.ops()[from..]
            .iter()
            .enumerate()
            .take_while(|(n, op)| *n == 0 || !matches!(op, Op::Window { .. }))
            .map(|(_, op)| op)
        {
            above.push(op.clone());
        }
        for (left, right, _, text) in extents(&above) {
            assert!(
                left >= window.x && right <= window.right() - 8.0,
                "{what}: {text:?} {left} {right}"
            );
        }
    };
    assert!(matches!(e.stage(), Stage::Write(_)), "{:?}", e.stage());
    inside(&scene(&e), Rect::new(320.0, 150.0, 640.0, 400.0), "write sheet");
    // The question of the picker (26 is the story object of the fixture).
    let mut e = on_bag();
    e.key(Key::X);
    for _ in 0..26 {
        e.key(Key::R);
    }
    e.key(Key::A);
    assert!(matches!(e.stage(), Stage::Ask(..)));
    let list = scene(&e);
    inside(&list, Rect::new(760.0, 438.0, 480.0, 170.0), "story question");
    assert!(extents(&list).iter().any(|(_, _, _, text)| text == "! STORY ITEM"));
    assert!(list.texts().any(|(_, _, t)| t.contains("Add anyway or Cancel")));
}
