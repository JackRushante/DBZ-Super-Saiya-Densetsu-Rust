//! The save editor against the real game: a slot edited by `ssd_menus::saveedit` is loaded by the game's own Continue, the game's
//! own level-up routine runs on the edited member, and the bag comes back with its counts. With the example pack `free-progression`
//! (a level cap of 999) a member edited to level 150 loads with its wide level, EXP and BP.
//!
//! Needs the game's ROM and a save to edit (a copy: the test never writes it):
//! `SSD_ROM_PATH=... SSD_SAVE_PATH=... cargo test -p ssd-catalogue --release --test save_editor_real_rom -- --ignored --nocapture`.

use ssd_catalogue::{Catalogue, game};
use ssd_game::data as d;
use ssd_game::menu::driver::Driver;
use ssd_menus::bag::{Rules, Sidecar};
use ssd_menus::cheats::CheatSidecar;
use ssd_menus::mods_state::ModsSidecar;
use ssd_menus::options::SlotOptions;
use ssd_menus::saveedit::{Editor, Key, Page, Sidecars, story};
use ssd_oracle::{
    DEFAULT_WRAM_SEED, HookTable, HostEvent, Joypad, Machine, MemorySaveStore, NullAudio,
    StatusRegister, Stopped, Stops,
};
use ssd_save::{Item, SaveFile, Slot};

const WRAM: u32 = 0x7E_0000;
const PARTY: u32 = 0x7E_0B00;
const RECORD: usize = 0x20;
const GOHAN: u8 = 1;
const CALL: u32 = 0x7F_FE00;

fn rom() -> Vec<u8> {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let bytes = std::fs::read(path).unwrap();
    let rom = ssd_rom::normalize(&bytes, None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32, "not the canonical ROM");
    rom.bytes
}

/// The picker's cursor goes to `object`.
fn go_to(editor: &mut Editor, object: u8) {
    for _ in 0..40 {
        if editor.session().unwrap().pick == usize::from(object) {
            return;
        }
        editor.key(Key::R);
    }
    panic!("object {object} is not in the picker");
}

fn byte(machine: &Machine, address: u32) -> u8 {
    machine.wram()[(address - WRAM) as usize]
}

fn word(machine: &Machine, address: u32) -> u16 {
    u16::from_le_bytes([byte(machine, address), byte(machine, address + 1)])
}

/// The game's Continue of slot 1: the title, Continue, the slot and Yes (the timeline of `xtask replay-from-save`).
const CONTINUE: &str =
    "1700 -\n4 start\n60 -\n4 right\n20 -\n4 start\n100 -\n4 a\n200 -\n8 a\n400 -\n";

fn continue_machine(rom: &[u8], sram: &[u8]) -> Machine {
    let mut saves = MemorySaveStore::default();
    saves.0.insert("sav".to_owned(), sram.to_vec());
    let mut machine = Machine::new(rom.to_vec(), Box::new(saves)).unwrap();
    for pad in ssd_game::scripts::parse_timeline(CONTINUE).unwrap() {
        machine.run_frame(pad, &mut NullAudio).unwrap();
    }
    machine
}

/// The party record of `character` in work RAM: its offset.
fn record_of(machine: &Machine, character: u8) -> Option<u32> {
    (0..9_u32)
        .map(|slot| PARTY + slot * RECORD as u32)
        .find(|&at| byte(machine, at) >= 0xC0 && byte(machine, at + 0x1C) == character)
}

fn bp_of(machine: &Machine, at: u32) -> u32 {
    u32::from(word(machine, at + 7)) | u32::from(byte(machine, at + 9)) << 16
}

/// Runs the game's routine at `entry` as a subroutine from an instruction boundary, to its return.
fn call(machine: &mut Machine, entry: u32) {
    let input = &mut |_frame: u64| Joypad::default();
    let stops = Stops { instructions: Some(1), ..Stops::default() };
    assert!(matches!(
        machine.run(&stops, input, &mut NullAudio, 4).unwrap(),
        Stopped::Instructions
    ));
    let jsl = [0x22, entry as u8, (entry >> 8) as u8, (entry >> 16) as u8];
    machine.write_wram(CALL, &jsl).unwrap();
    let mut registers = machine.cpu_registers();
    registers.pbr = (CALL >> 16) as u8;
    registers.pc = (CALL & 0xFFFF) as u16;
    registers.dbr = 0;
    registers.d = 0;
    registers.p = StatusRegister::from(0x24);
    machine.set_cpu_registers(registers);
    machine.set_breakpoint(CALL + 4);
    let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
    for _ in 0..4 {
        if matches!(
            machine.run(&stops, input, &mut NullAudio, 1).unwrap(),
            Stopped::Breakpoint { .. }
        ) {
            machine.clear_breakpoint(CALL + 4);
            return;
        }
    }
    panic!("${entry:06X} did not return");
}

/// `levels` more levels through the story's own routine (`03:C64A`: `$164C` the character, `$164D` the levels less one).
fn raise(machine: &mut Machine, character: u8, levels: u8) {
    machine.write_wram(0x7E_164C, &[character]).unwrap();
    machine.write_wram(0x7E_164D, &[levels - 1]).unwrap();
    call(machine, 0x03_C64A);
}

#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH) and a save (SSD_SAVE_PATH)"]
fn an_edited_slot_loads_keeps_its_bag_and_its_bp_delta_on_every_level_up_of_the_game() {
    let rom = rom();
    let catalogue = Catalogue::from_rom(&rom).unwrap();
    let labels = ssd_catalogue::menu_labels(&catalogue);
    let gohan = &catalogue.characters[usize::from(GOHAN)];
    let table = |level: u8| game::stats_at(gohan, level).unwrap().bp;
    // Two ordinary objects (no story check): the editor adds them without a question.
    let ordinary: Vec<u8> = (0..40_u8).filter(|&o| !story(&labels, o)).collect();
    let (first, second) = (ordinary[1], ordinary[2]);

    // The slot: the save's slot 1 with Gohan in the party at level 20 with the game's own BP, and a short bag.
    let path = std::env::var("SSD_SAVE_PATH").expect("SSD_SAVE_PATH");
    let mut file = SaveFile::from_bytes(&std::fs::read(path).unwrap()).unwrap();
    let mut slot = file.slot(0).unwrap();
    let mut record = game::new_record(gohan, 20, 3).unwrap();
    assert_eq!(record.bp(), table(20));
    record.set_flags(0xC0);
    slot.set_record(GOHAN, &record).unwrap();
    let mut party = slot.party();
    if !party.contains(&GOHAN) {
        party.push(GOHAN);
    }
    slot.set_party(&party).unwrap();
    let attr = labels.objects[usize::from(first)].attribute & !1;
    slot.set_item_list(&[(Item { id: first, attr }, 0), (Item { id: first, attr }, 0)]).unwrap();
    file.set_slot(0, &slot).unwrap();
    let before = file.as_bytes().to_vec();

    // The editor: Gohan to level 30 (the table's BP), +3000 BP, two Dragon Balls' worth of bag work.
    let mut editor = Editor::open(&before, &labels, &Sidecar::empty(), [false; 3]).unwrap();
    editor.key(Key::A);
    while editor.session().unwrap().members[editor.session().unwrap().member].id != GOHAN {
        editor.key(Key::R);
    }
    for _ in 0..10 {
        editor.key(Key::Right);
    }
    assert_eq!(
        editor.session().unwrap().members[editor.session().unwrap().member].record.bp(),
        table(30)
    );
    editor.key(Key::Down);
    for _ in 0..3000 {
        editor.key(Key::Right);
    }
    // The bag: the All page, the picker, three more of object 2 and one of object 4; one Turtle-like stack removed later.
    while editor.session().unwrap().page != Page::All {
        editor.key(Key::R);
    }
    editor.key(Key::X);
    go_to(&mut editor, first);
    editor.key(Key::Right);
    editor.key(Key::Right);
    editor.key(Key::A); // 3 more of the first object
    editor.key(Key::Y);
    go_to(&mut editor, second);
    editor.key(Key::A); // one of the second
    editor.key(Key::B);
    editor.key(Key::Start);
    editor.key(Key::Up);
    editor.key(Key::Up);
    editor.key(Key::A);
    let commit = editor.take_commit().expect("a write");
    let edited = SaveFile::from_bytes(&commit.sram).unwrap();
    let edited_slot: Slot = edited.slot(0).unwrap();
    assert_eq!(edited_slot.record(GOHAN).unwrap().level(), 30);
    assert_eq!(edited_slot.record(GOHAN).unwrap().bp(), table(30) + 3000);
    // Other slots are untouched.
    assert_eq!(commit.sram[0x600..0x1000], before[0x600..0x1000]);

    // The sidecars, re-keyed as the app does.
    let (mut bags, mut cheats, mut mods, mut options) =
        (Sidecar::empty(), CheatSidecar::empty(), ModsSidecar::empty(), SlotOptions::empty());
    commit.rekey(&mut Sidecars {
        bags: &mut bags,
        cheats: &mut cheats,
        mods: &mut mods,
        options: &mut options,
    });
    assert!(cheats.marked(0, commit.new_checksum), "the CHEATS badge");

    // The game's own Continue loads the edited slot.
    let mut machine = continue_machine(&rom, &commit.sram);
    let at = record_of(&machine, GOHAN).expect("Gohan is in the loaded party");
    assert_eq!(
        (byte(&machine, at + 2), bp_of(&machine, at)),
        (30, table(30) + 3000),
        "loaded as edited"
    );
    assert_eq!(word(&machine, at + 10), edited_slot.record(GOHAN).unwrap().max_hp());
    let count = usize::from(byte(&machine, d::ITEM_COUNT));
    let listed: Vec<u8> = (0..count).map(|i| byte(&machine, d::ITEM_LIST + 2 * i as u32)).collect();
    let saved: Vec<u8> = edited_slot.items().iter().map(|item| item.id).collect();
    assert_eq!(listed, saved, "the game's item list is what the editor wrote");

    // The bag survives Continue with its counts: the driver binds to the sidecar's bag, not to a rebuild from the list.
    let mut driver = Driver::with_bag(labels.clone(), Rules::default(), bags.clone());
    for _ in 0..30 {
        let frame = driver.frame(machine.wram(), machine.sram(), Joypad::default());
        for event in &frame.events {
            if let HostEvent::Wram { address, bytes } = event {
                machine.write_wram(*address, bytes).unwrap();
            }
        }
        machine.run_frame(frame.pad, &mut NullAudio).unwrap();
    }
    let bag = driver.bag().expect("the bag is bound");
    assert_eq!(bag.units_of(first), commit.bag.units_of(first));
    assert_eq!(bag.units_of(first), 5, "2 + 3 units of the first object");
    assert_eq!(bag.units_of(second), 1);
    assert_eq!(bag, &commit.bag, "the sidecar's bag, not a rebuild");
    assert_eq!(usize::from(byte(&machine, d::ITEM_COUNT)), count, "the list was not rewritten");

    // The game's own level-up: five more levels add the five rows, so the +3000 stays.
    raise(&mut machine, GOHAN, 5);
    let at = record_of(&machine, GOHAN).unwrap();
    assert_eq!(byte(&machine, at + 2), 35);
    assert_eq!(
        bp_of(&machine, at),
        table(35) + 3000,
        "the delta over the game's table is kept by the level-up"
    );
    println!("level 35: BP {} = table {} + 3000", bp_of(&machine, at), table(35));
    // And the cap: the level-up stops at the character's cap.
    raise(&mut machine, GOHAN, 30);
    let at = record_of(&machine, GOHAN).unwrap();
    assert_eq!(byte(&machine, at + 2), gohan.level_cap);
    assert_eq!(bp_of(&machine, at), table(gohan.level_cap) + 3000);

    // A negative delta is kept the same way.
    let mut low = Editor::open(&before, &labels, &Sidecar::empty(), [false; 3]).unwrap();
    low.key(Key::A);
    while low.session().unwrap().members[low.session().unwrap().member].id != GOHAN {
        low.key(Key::R);
    }
    low.key(Key::Down);
    for _ in 0..1000 {
        low.key(Key::Left);
    }
    low.key(Key::Start);
    low.key(Key::Up);
    low.key(Key::Up);
    low.key(Key::A);
    let commit = low.take_commit().expect("a write");
    let mut machine = continue_machine(&rom, &commit.sram);
    assert_eq!(bp_of(&machine, record_of(&machine, GOHAN).unwrap()), table(20) - 1000);
    raise(&mut machine, GOHAN, 4);
    let at = record_of(&machine, GOHAN).unwrap();
    assert_eq!((byte(&machine, at + 2), bp_of(&machine, at)), (24, table(24) - 1000));
}

/// The machine of the example pack `examples/free-progression` (its `level_cap` is 999), with the game's hooks.
fn pack_machine(rom: &[u8], sram: Option<&[u8]>) -> Machine {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/free-progression");
    let packs = [ssd_mod::Pack::open(&dir).unwrap()];
    ssd_mod::check_set(rom, &packs, ssd_mod::CheckOptions { allow_rom_bytes: false }).unwrap();
    let image = ssd_mod::build(rom, &packs).unwrap().bytes;
    let mut saves = MemorySaveStore::default();
    if let Some(sram) = sram {
        saves.0.insert("sav".to_owned(), sram.to_vec());
    }
    let mut machine =
        Machine::with_image(image, ssd_rom::crc32(rom), Box::new(saves), DEFAULT_WRAM_SEED)
            .unwrap();
    let mut table = HookTable::new();
    ssd_game::mods::install(&mut table);
    machine.set_hook_table(Some(table));
    machine
}

#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH) and a save (SSD_SAVE_PATH)"]
fn with_the_pack_a_slot_edited_to_level_150_loads_with_its_wide_level_exp_and_bp() {
    let rom = rom();
    let catalogue = Catalogue::from_rom(&rom).unwrap();
    let labels = ssd_catalogue::menu_labels(&catalogue);
    let gohan = &catalogue.characters[usize::from(GOHAN)];

    // The slot: a copy of the save's slot 1 with Gohan in the party at level 20 with the game's own BP.
    let path = std::env::var("SSD_SAVE_PATH").expect("SSD_SAVE_PATH");
    let mut file = SaveFile::from_bytes(&std::fs::read(path).unwrap()).unwrap();
    let mut slot = file.slot(0).unwrap();
    let mut record = game::new_record(gohan, 20, 3).unwrap();
    record.set_flags(0xC0);
    slot.set_record(GOHAN, &record).unwrap();
    let mut party = slot.party();
    if !party.contains(&GOHAN) {
        party.push(GOHAN);
    }
    slot.set_party(&party).unwrap();
    file.set_slot(0, &slot).unwrap();
    let before = file.as_bytes().to_vec();

    // The editor's BP limit holds the curve of every character at the top level.
    for (id, growth) in labels.growth.iter().enumerate().take(8) {
        let top = growth.anchor().unwrap().at(999).bp;
        println!("character {id}: BP at level 999 {top}");
        assert!(top <= ssd_menus::saveedit::BP_MAX_WIDE, "character {id}");
    }

    // The level cap is the one the game reads from the pack's image.
    let probe = pack_machine(&rom, None);
    let cap = ssd_game::mods::level_cap_in(|address| probe.peek(address).unwrap_or(0));
    assert_eq!(cap, Some(999));
    let plain = Machine::new(rom.clone(), Box::new(MemorySaveStore::default())).unwrap();
    assert_eq!(ssd_game::mods::level_cap_in(|address| plain.peek(address).unwrap_or(0)), None);

    // The editor: Gohan to level 150 (one press a level), then a big BP delta with a held key.
    let mut editor = Editor::open(&before, &labels, &Sidecar::empty(), [false; 3]).unwrap();
    editor.set_progression(cap, &ModsSidecar::empty());
    editor.key(Key::A);
    while editor.session().unwrap().members[editor.session().unwrap().member].id != GOHAN {
        editor.key(Key::R);
    }
    let level = |editor: &Editor| {
        editor.session().unwrap().members[editor.session().unwrap().member].level()
    };
    while level(&editor) < 150 {
        editor.key(Key::Right);
    }
    let table = labels.growth[usize::from(GOHAN)].anchor().unwrap().at(150);
    let member = editor.session().unwrap().members[editor.session().unwrap().member].clone();
    assert_eq!((member.level(), member.record.level(), member.bp()), (150, 99, table.bp));
    editor.key(Key::Down);
    let bp =
        |editor: &Editor| editor.session().unwrap().members[editor.session().unwrap().member].bp();
    for _ in 0..200_000 {
        if bp(&editor) > table.bp + 20_000_000 {
            break;
        }
        editor.held(1);
    }
    let edited = editor.session().unwrap().members[editor.session().unwrap().member].clone();
    assert!(edited.bp() > table.bp + 20_000_000 && edited.bp() > 1 << 24, "{}", edited.bp());
    editor.key(Key::Start);
    editor.key(Key::Up);
    editor.key(Key::Up);
    editor.key(Key::A);
    let commit = editor.take_commit().expect("a write");

    // The record in the save holds the byte and the low 24 bits; the mod state holds the rest.
    let written =
        SaveFile::from_bytes(&commit.sram).unwrap().slot(0).unwrap().record(GOHAN).unwrap();
    assert_eq!((written.level(), u64::from(written.bp())), (99, edited.bp() & 0xFF_FFFF));
    let (mut bags, mut cheats, mut mods, mut options) =
        (Sidecar::empty(), CheatSidecar::empty(), ModsSidecar::empty(), SlotOptions::empty());
    commit.rekey(&mut Sidecars {
        bags: &mut bags,
        cheats: &mut cheats,
        mods: &mut mods,
        options: &mut options,
    });
    assert!(mods.entry_for(0, commit.new_checksum).is_some_and(|state| state.levels.is_some()));
    // `SSD_DUMP_DIR`: the edited save and its sidecar files go there, for a UI snapshot of the app on them.
    if let Ok(dir) = std::env::var("SSD_DUMP_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ssd.sav"), &commit.sram).unwrap();
        std::fs::write(dir.join("ssd.mods"), mods.to_bytes()).unwrap();
        std::fs::write(dir.join("ssd.bag"), bags.to_bytes()).unwrap();
        std::fs::write(dir.join("ssd.cheats"), cheats.to_bytes()).unwrap();
    }

    // The game's own Continue loads it, and the driver puts the mod state in work RAM as the app does.
    let mut machine = pack_machine(&rom, Some(&commit.sram));
    for pad in ssd_game::scripts::parse_timeline(CONTINUE).unwrap() {
        machine.run_frame(pad, &mut NullAudio).unwrap();
    }
    let mut driver = Driver::with_bag(labels.clone(), Rules::default(), bags).with_mods(mods);
    for _ in 0..30 {
        let frame = driver.frame(machine.wram(), machine.sram(), Joypad::default());
        for event in &frame.events {
            if let HostEvent::Wram { address, bytes } = event {
                machine.write_wram(*address, bytes).unwrap();
            }
        }
        machine.run_frame(frame.pad, &mut NullAudio).unwrap();
    }
    let at = record_of(&machine, GOHAN).expect("Gohan is in the loaded party");
    let slot_index = (at - PARTY) / RECORD as u32;
    let wide_level = word(&machine, ssd_game::mods::state::wide_level_at(GOHAN));
    let exp_high = byte(&machine, ssd_game::mods::state::exp_high_at(GOHAN));
    let bp_high = word(&machine, ssd_game::mods::state::BLOCK + 2 * slot_index);
    let loaded_bp = u64::from(bp_high) << 24 | u64::from(bp_of(&machine, at));
    let exp = u64::from(exp_high) << 24
        | u64::from(word(&machine, at + 0x0F))
        | u64::from(byte(&machine, at + 0x11)) << 16;
    println!("loaded: byte {} wide {wide_level} BP {loaded_bp} EXP {exp}", byte(&machine, at + 2));
    assert_eq!(byte(&machine, at + 2), 99, "the record's byte saturates at 99");
    assert_eq!(wide_level, 150, "the wide level");
    assert_eq!(loaded_bp, edited.bp(), "the wide BP");
    assert_eq!(exp, table.exp, "the EXP the level begins at");
    assert_eq!(word(&machine, at + 10), table.max_hp);

    // The HD Status screen reads the same.
    let view = ssd_game::menu::MenuProbe::new(machine.wram(), &labels).status_view(0);
    let shown = view.members.iter().find(|m| m.name == labels.characters[usize::from(GOHAN)].name);
    let shown = shown.expect("Gohan on the Status screen");
    assert_eq!((shown.level, shown.bp, shown.exp), (150, edited.bp(), table.exp));

    // The game's own level-up goes on from the edited member: the next level's row is the curve's, the delta is kept.
    raise(&mut machine, GOHAN, 1);
    let at = record_of(&machine, GOHAN).unwrap();
    let anchor = labels.growth[usize::from(GOHAN)].anchor().unwrap();
    let bp_high = word(&machine, ssd_game::mods::state::BLOCK + 2 * slot_index);
    let bp_now = u64::from(bp_high) << 24 | u64::from(bp_of(&machine, at));
    assert_eq!(word(&machine, ssd_game::mods::state::wide_level_at(GOHAN)), 151);
    assert_eq!(
        bp_now,
        edited.bp() + anchor.bp_gain(151),
        "BP = curve(level) + delta, after a level-up"
    );
}
