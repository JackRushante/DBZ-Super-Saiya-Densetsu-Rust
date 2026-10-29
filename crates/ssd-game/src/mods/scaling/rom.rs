//! The two hooks of `enemy_scaling` run headless on the example pack `examples/enemy-scaling`, against the real game: a battle's
//! records are built with the party at two levels, the difficulty moves them, the EXP award is scaled, and the cheats act on
//! the scaled numbers. Every test needs the game's ROM (`SSD_ROM_PATH`).

use super::*;
use crate::cheats;
use crate::mods::state::wide;
use ssd_menus::cheats::{Cheats, Multiplier, Setting};
use ssd_mod::{Pack, ParamValue};
use ssd_oracle::{
    HookTable, HostEvent, Joypad, Machine, MemorySaveStore, NullAudio, StatusRegister, Stopped,
    Stops,
};

/// Where the call stub is staged: the upper work RAM, which the game's boot does not clear.
const STUB: u32 = 0x7F_FE00;
/// `$7E:14C5`: the offset of the enemy record the EXP award is for (`battle_enemy_offset`), and `$7E:17C1`: the unit of each slot.
const ENEMY_OFFSET: u32 = 0x7E_14C5;
const SLOT_UNITS: u32 = 0x7E_17C1;

/// The five enemy records after a battle's set-up, as the test reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Record {
    flags: u8,
    unit: u8,
    level: u8,
    hp: u16,
    max_hp: u16,
    ki: u16,
    max_ki: u16,
    bp: u64,
}

fn rom() -> Vec<u8> {
    let raw = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
    ssd_rom::normalize(&raw, None).unwrap().bytes
}

/// The example pack, with its difficulty replaced when `difficulty` is given.
fn pack(difficulty: Option<&str>) -> Pack {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/enemy-scaling");
    let mut pack = Pack::from_dir(&dir).unwrap();
    if let Some(name) = difficulty {
        pack.manifest.feature[0].params.insert("difficulty".into(), ParamValue::from(name));
    }
    pack
}

/// The game after its boot, on the image of `packs`, with the hooks of the packs when `hooks` and, when `cheats` is given, the
/// cheats' wrappers and block.
fn boot(packs: &[Pack], hooks: bool, cheats: Option<&Cheats>) -> Machine {
    let rom = rom();
    let image = ssd_mod::build(&rom, packs).unwrap().bytes;
    let mut machine = Machine::with_image(
        image,
        ssd_rom::crc32(&rom),
        Box::new(MemorySaveStore::default()),
        ssd_oracle::DEFAULT_WRAM_SEED,
    )
    .unwrap();
    let mut table = HookTable::new();
    if hooks {
        crate::mods::install(&mut table);
    }
    if cheats.is_some() {
        crate::cheats::install(&mut table);
    }
    machine.set_hook_table(Some(table));
    let frame = Stops { frame_end: true, ..Stops::default() };
    for _ in 0..300 {
        machine.run(&frame, &mut |_| Joypad::default(), &mut NullAudio, 1).unwrap();
    }
    if let Some(cheats) = cheats {
        for event in cheats::block_events(cheats) {
            if let HostEvent::Wram { address, bytes } = event {
                machine.write_wram(address, &bytes).unwrap();
            }
        }
    }
    machine
}

/// Puts a party in the nine party records: `(character, level)` for the first records, the rest out of the party.
fn stage_party(machine: &mut Machine, party: &[(u8, u8)]) {
    for slot in 0..PARTY_SLOTS {
        let base = party_record(slot);
        match party.get(slot as usize) {
            Some(&(character, level)) => {
                machine.write_wram(base + FLAGS, &[0xC0]).unwrap();
                machine.write_wram(base + UNIT, &[character]).unwrap();
                machine.write_wram(base + LEVEL, &[level]).unwrap();
                machine.write_wram(base + CHARACTER, &[character]).unwrap();
            }
            None => machine.write_wram(base + FLAGS, &[0]).unwrap(),
        }
    }
}

/// Runs the routines at `routines` in turn, as `JSL`s from the stub, and returns when the last one is done.
fn run_stub(machine: &mut Machine, routines: &[u32]) {
    let mut code = Vec::new();
    for &routine in routines {
        code.extend_from_slice(&[0x22, routine as u8, (routine >> 8) as u8, (routine >> 16) as u8]);
    }
    let back = STUB + code.len() as u32;
    machine.write_wram(STUB, &code).unwrap();
    let mut registers = machine.cpu_registers();
    registers.pbr = (STUB >> 16) as u8;
    registers.pc = (STUB & 0xFFFF) as u16;
    registers.dbr = 0;
    registers.d = 0;
    registers.p = StatusRegister::from(0x24);
    machine.set_cpu_registers(registers);
    // A stub of another length left its own breakpoint, which this one's code would run into.
    for stale in machine.breakpoints().clone() {
        machine.clear_breakpoint(stale);
    }
    machine.set_breakpoint(back);
    let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
    for _ in 0..6 {
        match machine.run(&stops, &mut |_| Joypad::default(), &mut NullAudio, 2).unwrap() {
            Stopped::Breakpoint { key } if key == back => return,
            Stopped::FrameEnd => {}
            other => panic!("the routines did not return: {other:?}"),
        }
    }
    panic!("the routines did not return within the frames of the call");
}

fn word(machine: &Machine, address: u32) -> u16 {
    let at = (address - 0x7E_0000) as usize;
    u16::from_le_bytes([machine.wram()[at], machine.wram()[at + 1]])
}

fn byte(machine: &Machine, address: u32) -> u8 {
    machine.wram()[(address - 0x7E_0000) as usize]
}

/// The 24 bits at `address`.
fn triple(machine: &Machine, address: u32) -> u32 {
    u32::from(word(machine, address)) | u32::from(byte(machine, address + 2)) << 16
}

/// Whether the mod state block's marker is in work RAM.
fn block_is_live(machine: &Machine) -> bool {
    MARKER.iter().enumerate().all(|(k, &b)| byte(machine, MARKER_AT + k as u32) == b)
}

/// The five enemy records, with the high part of each BP from the mod state block when it is live.
fn records(machine: &Machine) -> [Record; 5] {
    let live = block_is_live(machine);
    std::array::from_fn(|slot| {
        let base = enemy_record(slot as u32);
        let high = if live { word(machine, BLOCK + (ENEMY_HIGH + 2 * slot) as u32) } else { 0 };
        Record {
            flags: byte(machine, base + FLAGS),
            unit: byte(machine, base + UNIT),
            level: byte(machine, base + LEVEL),
            hp: word(machine, base + HP),
            max_hp: word(machine, base + MAX_HP),
            ki: word(machine, base + KI),
            max_ki: word(machine, base + MAX_KI),
            bp: wide(triple(machine, base + BP), high),
        }
    })
}

/// A battle's set-up for `group` with the party as staged: the group copied into the slots, and the records filled.
fn battle(machine: &mut Machine, group: u8) -> [Record; 5] {
    machine.write_wram(GROUP_INDEX, &[group]).unwrap();
    run_stub(machine, &[0x01_F692, 0x01_F719]);
    records(machine)
}

/// A party record as the level is read from it.
fn party_fighter((character, level): (u8, u8)) -> Fighter {
    Fighter { flags: 0xC0, unit: character, level: level.into(), character }
}

/// What the scaling must make of an original record `r` for a party at `sixths` and a difficulty.
fn expected(r: &Record, sixths: u32, difficulty: Difficulty) -> Record {
    if r.flags == 0 || r.level == 0 {
        return *r;
    }
    let ratio = ratio(sixths, difficulty, r.level);
    Record {
        level: scaled_level(sixths, difficulty).min(99) as u8,
        hp: scale_hp(r.hp, ratio),
        max_hp: scale_hp(r.max_hp, ratio),
        ki: scale_ki(r.ki, ratio),
        max_ki: scale_ki(r.max_ki, ratio),
        bp: scale_bp(r.bp, ratio),
        ..*r
    }
}

/// The unit row's level and EXP reward in the ROM (`02:B214`, 14 bytes a unit).
fn unit_row(rom: &[u8], unit: u8) -> (u8, u32) {
    let at = 0x02 * 0x8000 + 0xB214 - 0x8000 + 14 * usize::from(unit);
    (rom[at + 1], u32::from_le_bytes([rom[at + 11], rom[at + 12], rom[at + 13], 0]))
}

/// A lone Saibaman (group 3, an odd count), a pair of Saibamen (group 4, an even count: slot 0 is not visited), five mixed
/// enemies (group 16) and Raditz (group 5).
const GROUPS: [u8; 4] = [3, 4, 16, 5];

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn the_records_of_a_battle_follow_the_party_at_a_low_and_a_high_level() {
    let low = [(0_u8, 3_u8), (2, 3)];
    let high = [(0_u8, 40_u8), (1, 38), (2, 35), (3, 10)];
    for group in GROUPS {
        for (party, name) in [(&low[..], "low"), (&high[..], "high")] {
            let sixths =
                party_sixths(&party.iter().copied().map(party_fighter).collect::<Vec<_>>())
                    .unwrap();
            let mut machine = boot(&[pack(None)], true, None);
            stage_party(&mut machine, party);
            let scaled = battle(&mut machine, group);
            let mut reference = boot(&[], true, None);
            stage_party(&mut reference, party);
            let plain = battle(&mut reference, group);
            assert!(plain.iter().any(|r| r.flags != 0), "group {group} has enemies");
            assert_ne!(
                scaled, plain,
                "group {group}, party {name}: the scaling changes the records"
            );
            for (slot, (now, then)) in scaled.iter().zip(&plain).enumerate() {
                assert_eq!(
                    *now,
                    expected(then, sixths, Difficulty::Normal),
                    "group {group}, party {name}, slot {slot}"
                );
            }
        }
    }
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn at_the_enemys_own_level_the_records_are_the_originals_and_a_higher_party_makes_them_stronger() {
    // Group 3 is a lone Saibaman: a party whose mean is the level of its row changes nothing.
    let rom = rom();
    let (level, _) = unit_row(&rom, 4);
    let party = [(0_u8, level), (2, level)];
    let mut original = boot(&[], true, None);
    stage_party(&mut original, &party);
    let plain = battle(&mut original, 3);
    let mut machine = boot(&[pack(None)], true, None);
    stage_party(&mut machine, &party);
    let same = battle(&mut machine, 3);
    assert_eq!(same, plain, "the party at the enemy's own level: every number is the original's");
    assert_eq!(plain[0].level, level);
    let mut stronger = boot(&[pack(None)], true, None);
    stage_party(&mut stronger, &[(0, level + 20), (2, level + 20)]);
    let grown = battle(&mut stronger, 3);
    assert!(grown[0].hp > plain[0].hp && grown[0].bp > plain[0].bp && grown[0].ki > plain[0].ki);
    assert_eq!(grown[0].level, level + 20, "the record's level is the party's");
    assert_eq!(grown[0].unit, plain[0].unit);
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn a_party_with_no_fighter_leaves_the_records_as_the_original_makes_them() {
    let mut original = boot(&[], true, None);
    stage_party(&mut original, &[]);
    let plain = battle(&mut original, 16);
    let mut machine = boot(&[pack(None)], true, None);
    stage_party(&mut machine, &[]);
    assert_eq!(battle(&mut machine, 16), plain, "no fighter, no level to follow");
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn easy_and_hard_move_the_records_below_and_above_normal() {
    let party = [(0_u8, 20_u8), (2, 20), (3, 20)];
    let mut original = boot(&[], true, None);
    stage_party(&mut original, &party);
    let plain = battle(&mut original, 16);
    let mut by_difficulty = Vec::new();
    for (name, difficulty) in
        [("easy", Difficulty::Easy), ("normal", Difficulty::Normal), ("hard", Difficulty::Hard)]
    {
        let mut machine = boot(&[pack(Some(name))], true, None);
        stage_party(&mut machine, &party);
        let scaled = battle(&mut machine, 16);
        for (slot, (now, then)) in scaled.iter().zip(&plain).enumerate() {
            assert_eq!(*now, expected(then, 120, difficulty), "{name}, slot {slot}");
        }
        by_difficulty.push(scaled);
    }
    let [easy, normal, hard] = &by_difficulty[..] else { unreachable!("three difficulties") };
    for (slot, ((easy, normal), hard)) in easy.iter().zip(normal).zip(hard).enumerate() {
        if normal.flags != 0 {
            assert!(easy.bp < normal.bp && normal.bp < hard.bp, "slot {slot}: BP");
            assert!(easy.hp <= normal.hp && normal.hp <= hard.hp, "slot {slot}: HP");
        }
    }
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn a_scripted_loss_is_left_as_the_original_makes_it_and_so_is_everything_without_the_feature() {
    let party = [(0_u8, 45_u8), (1, 44), (2, 43)];
    let loss = cheats::SCRIPTED_LOSS[0];
    let mut original = boot(&[], true, None);
    stage_party(&mut original, &party);
    let plain = battle(&mut original, loss);
    let mut machine = boot(&[pack(None)], true, None);
    stage_party(&mut machine, &party);
    assert_eq!(battle(&mut machine, loss), plain, "the scripted loss is not scaled");
    // With no pack the hooks are installed and decline: the whole work RAM after a set-up is the one of a machine that has no
    // hook at all.
    let mut bare = boot(&[], false, None);
    stage_party(&mut bare, &party);
    let bare_records = battle(&mut bare, 16);
    let mut hooked = boot(&[], true, None);
    stage_party(&mut hooked, &party);
    let hooked_records = battle(&mut hooked, 16);
    assert_eq!(hooked_records, bare_records);
    assert_eq!(
        hooked.wram(),
        bare.wram(),
        "declining leaves the work RAM exactly as the original does"
    );
}

/// One defeated enemy's EXP award for the unit in slot 0, from a machine with the party staged.
fn award(machine: &mut Machine, party: &[(u8, u8)], unit: u8, group: u8) -> u32 {
    stage_party(machine, party);
    machine.write_wram(GROUP_INDEX, &[group]).unwrap();
    machine.write_wram(ENEMY_OFFSET, &[0, 0]).unwrap();
    machine.write_wram(SLOT_UNITS, &[unit]).unwrap();
    machine.write_wram(BATTLE_EXP, &[0, 0, 0]).unwrap();
    run_stub(machine, &[0x1D_8C33]);
    triple(machine, BATTLE_EXP)
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn the_exp_of_a_defeated_enemy_is_scaled_and_the_exp_cheat_multiplies_the_scaled_award() {
    let rom = rom();
    let unit = 4_u8; // a Saibaman
    let (level, reward) = unit_row(&rom, unit);
    let party = [(0_u8, 30_u8), (2, 30)];
    let want = scale_exp(reward, ratio(6 * 30, Difficulty::Normal, level));
    assert_ne!(want, reward, "a level 30 party changes the reward of a level {level} unit");
    let mut original = boot(&[], true, None);
    assert_eq!(
        award(&mut original, &party, unit, 3),
        reward,
        "without the pack: the unit row's reward"
    );
    let mut scaled = boot(&[pack(None)], true, None);
    assert_eq!(award(&mut scaled, &party, unit, 3), want, "with the pack: the scaled reward");
    // The EXP cheat at x3 multiplies the award, scaled.
    let mut cheating = Cheats::neutral();
    let steps = Setting::Exp.steps();
    cheating
        .set_index(Setting::Exp, steps.iter().position(|m| *m == Multiplier::new(3, 1)).unwrap());
    let mut both = boot(&[pack(None)], true, Some(&cheating));
    assert_eq!(award(&mut both, &party, unit, 3), 3 * want, "the cheat acts on the scaled award");
    // A fight of the scripted-loss list pays the original's reward.
    let mut loss = boot(&[pack(None)], true, None);
    assert_eq!(award(&mut loss, &party, unit, cheats::SCRIPTED_LOSS[0]), reward);
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn the_enemy_power_cheat_multiplies_the_scaled_records_wide_part_included() {
    // The last Super Vegeta's group ($6F) against a level 100 party needs more than 24 bits of BP: the high part is in the block.
    let party = [(0_u8, 100_u8), (1, 100), (2, 100)];
    let group = 0x6F;
    let mut original = boot(&[], true, None);
    stage_party(&mut original, &party);
    let plain = battle(&mut original, group);
    let mut machine = boot(&[pack(None)], true, None);
    stage_party(&mut machine, &party);
    let scaled = battle(&mut machine, group);
    let want = expected(&plain[0], 600, Difficulty::Normal);
    assert_eq!(scaled[0], want);
    assert!(want.bp > 0xFF_FFFF, "the BP is wider than 24 bits: {}", want.bp);
    assert!(block_is_live(&machine), "the block was made live for the wide part");
    // The cheat at x2 doubles the HP and the BP (the wide one) and leaves the level.
    let mut cheating = Cheats::neutral();
    let steps = Setting::Power.steps();
    cheating
        .set_index(Setting::Power, steps.iter().position(|m| *m == Multiplier::new(2, 1)).unwrap());
    let mut both = boot(&[pack(None)], true, Some(&cheating));
    stage_party(&mut both, &party);
    let doubled = battle(&mut both, group);
    assert_eq!(doubled[0].level, want.level);
    assert_eq!(doubled[0].hp, want.hp * 2);
    assert_eq!(doubled[0].max_hp, want.max_hp * 2);
    assert_eq!(doubled[0].bp, want.bp * 2, "the wide BP is doubled, not its low part");
}

/// A pack's own unit (id 105, level 5) and a group of two of them (group 112): the data tables' redirector writes the unit's row
/// into the records, and the scaling then takes the level of the record.
const TABLES: &str = r#"
[[unit]]
id = 105
name = "Guard"
class = 1
level = 5
hp = 300
bp = 1000
ki = 50
defence = 30
tie = 7
exp = 0
sprite_like = 4

[[unit.command]]
id = 0
attr = 16
factor = 4

[[group]]
id = 112
units = [105, 105]
"#;

/// The example pack with the unit and the group above, and without the feature when `feature` is false.
fn pack_with_unit(feature: bool) -> Pack {
    let mut pack = pack(None);
    pack.manifest.tables.push(ssd_mod::TablesEntry { file: "tables.toml".into() });
    pack.files.insert("tables.toml".into(), TABLES.as_bytes().to_vec());
    if !feature {
        pack.manifest.feature.clear();
    }
    pack
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn a_unit_of_a_pack_is_scaled_from_the_level_its_own_row_gives_the_record() {
    let party = [(0_u8, 40_u8), (1, 38), (2, 35)];
    let mut plain = boot(&[pack_with_unit(false)], true, None);
    stage_party(&mut plain, &party);
    let unscaled = battle(&mut plain, 112);
    let mut machine = boot(&[pack_with_unit(true)], true, None);
    stage_party(&mut machine, &party);
    let scaled = battle(&mut machine, 112);
    let live: Vec<&Record> = unscaled.iter().filter(|r| r.flags != 0).collect();
    assert_eq!(live.len(), 2, "two pack units");
    assert!(
        live.iter().all(|r| r.unit == 105 && r.level == 5 && r.hp == 300),
        "the pack's row: {live:?}"
    );
    let sixths = 6 * 113 / 3;
    for (slot, (now, then)) in scaled.iter().zip(&unscaled).enumerate() {
        assert_eq!(*now, expected(then, sixths, Difficulty::Normal), "slot {slot}");
    }
    let first = scaled.iter().find(|r| r.flags != 0).unwrap();
    assert_eq!(
        (first.level, first.hp),
        (38, 1280),
        "level 5 made for a party of 37.67: q = 256/60, and 300 * q = 1280"
    );
}

// ---- The offsets: a random encounter at the random offset, every other battle at the boss offset ----

/// `$7E:0D8B`: the zone the pick reads, and `$7E:0051`: the random byte it indexes the zone's eight groups with.
const ZONE: u32 = 0x7E_0D8B;
/// `$7E:01A0`: the map.
const MAP_ID: u32 = 0x7E_01A0;
const RANDOM_BYTE: u32 = 0x7E_0051;

/// The example pack with the two offsets in place of the difficulty.
fn pack_offsets(random: i64, story: i64) -> Pack {
    let mut pack = pack(None);
    let params = &mut pack.manifest.feature[0].params;
    params.clear();
    params.insert("random_offset".into(), ParamValue::Number(random));
    params.insert("boss_offset".into(), ParamValue::Number(story));
    pack
}

/// What the scaling must make of an original record `r` for a party at `sixths`, a scaling and a kind of battle.
fn expected_at(r: &Record, sixths: u32, scaling: Scaling, kind: Kind) -> Record {
    if r.flags == 0 || r.level == 0 {
        return *r;
    }
    let aim = target(sixths, scaling, kind);
    let ratio = aim.ratio(r.level);
    Record {
        level: aim.level().min(99) as u8,
        hp: scale_hp(r.hp, ratio),
        max_hp: scale_hp(r.max_hp, ratio),
        ki: scale_ki(r.ki, ratio),
        max_ki: scale_ki(r.max_ki, ratio),
        bp: scale_bp(r.bp, ratio),
        ..*r
    }
}

/// A random encounter as the game starts it: the group is picked by `02:D34A` from the map, the zone and the random byte, and
/// the records are built from it. Returns the group and the records.
fn random_battle(machine: &mut Machine, map: u8, zone: u8, roll: u8) -> (u8, [Record; 5]) {
    machine.write_wram(MAP_ID, &[map]).unwrap();
    machine.write_wram(ZONE, &[zone]).unwrap();
    machine.write_wram(RANDOM_BYTE, &[roll]).unwrap();
    run_stub(machine, &[0x02_D34A, 0x01_F692, 0x01_F719]);
    (byte(machine, GROUP_INDEX), records(machine))
}

/// The map and group of a Namek random encounter: the first map whose zone 1 picks one of the groups `$1E` to `$23` (the
/// soldiers of Frieza on Namek: Appule, Banan, Vug, Bundt, Baimen) with the random byte 0.
fn namek_map() -> (u8, u8) {
    let mut machine = boot(&[], true, None);
    for map in 0..48_u8 {
        machine.write_wram(MAP_ID, &[map]).unwrap();
        machine.write_wram(ZONE, &[1]).unwrap();
        machine.write_wram(RANDOM_BYTE, &[0]).unwrap();
        run_stub(&mut machine, &[0x02_D34A]);
        let group = byte(&machine, GROUP_INDEX);
        if (0x1E..=0x23).contains(&group) {
            return (map, group);
        }
    }
    panic!("no map picks a Namek group");
}

/// Vegeta (group 21) and Nappa (group 20) against Goku: story groups, built without the random pick.
const VEGETA: u8 = 21;
const NAPPA: u8 = 20;

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn a_random_encounter_is_made_for_the_random_offset_and_a_story_battle_for_the_boss_offset() {
    let scaling = Scaling::Offsets { random: -3, story: 3 };
    let party = [(0_u8, 46_u8), (1, 46), (2, 46)];
    let sixths = 6 * 46;
    let (map, namek_group) = namek_map();
    // The original's records, and the pack's (and the old pack's, for the numbers).
    let mut original = boot(&[], true, None);
    stage_party(&mut original, &party);
    let (group, plain) = random_battle(&mut original, map, 1, 0);
    assert_eq!(group, namek_group);
    let mut machine = boot(&[pack_offsets(-3, 3)], true, None);
    stage_party(&mut machine, &party);
    let (group, random) = random_battle(&mut machine, map, 1, 0);
    assert_eq!(group, namek_group, "the pick is the original's");
    for (slot, (now, then)) in random.iter().zip(&plain).enumerate() {
        assert_eq!(*now, expected_at(then, sixths, scaling, Kind::Random), "random, slot {slot}");
        if now.flags != 0 {
            assert_eq!(now.level, 43, "the random enemy is at P - 3");
        }
    }
    assert!(random.iter().any(|r| r.flags != 0 && r.bp != 0), "the encounter has enemies");
    // The same group, started without the pick (a script's battle), is a story battle at P + 3.
    let mut story = boot(&[pack_offsets(-3, 3)], true, None);
    stage_party(&mut story, &party);
    let same_group = battle(&mut story, namek_group);
    for (slot, (now, then)) in same_group.iter().zip(&plain).enumerate() {
        assert_eq!(*now, expected_at(then, sixths, scaling, Kind::Story), "story, slot {slot}");
        if now.flags != 0 {
            assert_eq!(now.level, 49, "the story enemy is at P + 3");
        }
    }
    // Vegeta and Nappa: story groups, at P + 3.
    let mut numbers = Vec::new();
    for (name, group) in [("Nappa", NAPPA), ("Vegeta", VEGETA)] {
        let mut original = boot(&[], true, None);
        stage_party(&mut original, &party);
        let plain = battle(&mut original, group);
        let mut old = boot(&[pack(None)], true, None);
        stage_party(&mut old, &party);
        let old = battle(&mut old, group);
        let mut machine = boot(&[pack_offsets(-3, 3)], true, None);
        stage_party(&mut machine, &party);
        let scaled = battle(&mut machine, group);
        let boss = scaled.iter().find(|r| r.flags != 0).unwrap();
        assert_eq!(boss.level, 49, "{name} is made for P + 3");
        for (slot, (now, then)) in scaled.iter().zip(&plain).enumerate() {
            assert_eq!(
                *now,
                expected_at(then, sixths, scaling, Kind::Story),
                "{name}, slot {slot}"
            );
        }
        let first = |records: &[Record; 5]| *records.iter().find(|r| r.flags != 0).unwrap();
        numbers.push(format!(
            "{name}: original L{} HP {} BP {} | old pack (normal) L{} HP {} BP {} | offsets L{} HP {} BP {}",
            first(&plain).level,
            first(&plain).max_hp,
            first(&plain).bp,
            first(&old).level,
            first(&old).max_hp,
            first(&old).bp,
            boss.level,
            boss.max_hp,
            boss.bp
        ));
    }
    let mut old = boot(&[pack(None)], true, None);
    stage_party(&mut old, &party);
    let (_, old_random) = random_battle(&mut old, map, 1, 0);
    let first = |records: &[Record; 5]| *records.iter().find(|r| r.flags != 0).unwrap();
    numbers.push(format!(
        "Namek random (group {namek_group}, map {map}): original L{} HP {} BP {} | old pack (normal) L{} HP {} BP {} | offsets L{} HP {} BP {}",
        first(&plain).level,
        first(&plain).max_hp,
        first(&plain).bp,
        first(&old_random).level,
        first(&old_random).max_hp,
        first(&old_random).bp,
        first(&random).level,
        first(&random).max_hp,
        first(&random).bp,
    ));
    println!("party level 46:\n{}", numbers.join("\n"));
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn the_random_mark_is_taken_by_the_battle_it_belongs_to_and_the_next_one_is_a_story_battle() {
    let scaling = Scaling::Offsets { random: -3, story: 3 };
    let party = [(0_u8, 30_u8), (1, 30), (2, 30)];
    let sixths = 6 * 30;
    let (map, group) = namek_map();
    let mut original = boot(&[], true, None);
    stage_party(&mut original, &party);
    let plain = battle(&mut original, group);
    let mut machine = boot(&[pack_offsets(-3, 3)], true, None);
    stage_party(&mut machine, &party);
    let (_, random) = random_battle(&mut machine, map, 1, 0);
    let live: Vec<&Record> = random.iter().filter(|r| r.flags != 0).collect();
    assert!(live.iter().all(|r| r.level == 27), "random: P - 3");
    // The next battle of the same machine, the same group, without a pick: a story battle, though a random one came before.
    let after = battle(&mut machine, group);
    for (slot, (now, then)) in after.iter().zip(&plain).enumerate() {
        assert_eq!(
            *now,
            expected_at(then, sixths, scaling, Kind::Story),
            "story after, slot {slot}"
        );
    }
    // And a builder run again without a pick (a form change) stays a story battle.
    run_stub(&mut machine, &[0x01_F692, 0x01_F719]);
    assert_eq!(records(&machine), after);
    // Two random battles one after the other are both random.
    let (_, again) = random_battle(&mut machine, map, 1, 0);
    assert_eq!(again, random);
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn the_scripted_loss_is_not_scaled_with_the_offsets_either() {
    let party = [(0_u8, 45_u8), (1, 44), (2, 43)];
    let loss = cheats::SCRIPTED_LOSS[0];
    let mut original = boot(&[], true, None);
    stage_party(&mut original, &party);
    let plain = battle(&mut original, loss);
    let mut machine = boot(&[pack_offsets(-3, 3)], true, None);
    stage_party(&mut machine, &party);
    assert_eq!(
        battle(&mut machine, loss),
        plain,
        "Nail against Frieza keeps the original's numbers"
    );
    // And a fight after it is scaled again, at the boss offset.
    let next = battle(&mut machine, VEGETA);
    let boss = next.iter().find(|r| r.flags != 0).unwrap();
    assert_eq!(boss.level, 47, "P = 44, +3");
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn the_exp_of_a_defeated_enemy_follows_the_kind_of_its_battle() {
    let rom = rom();
    let unit = 4_u8; // a Saibaman
    let (level, reward) = unit_row(&rom, unit);
    let party = [(0_u8, 40_u8), (2, 40), (3, 40)];
    let scaling = Scaling::Offsets { random: -3, story: 3 };
    let want = |kind| scale_exp(reward, target(6 * 40, scaling, kind).ratio(level));
    assert!(want(Kind::Random) < want(Kind::Story));
    let (map, _) = namek_map();
    // The pack's EXP after a random battle was built, and after a story battle was built.
    let award_after = |random: bool| {
        let mut machine = boot(&[pack_offsets(-3, 3)], true, None);
        stage_party(&mut machine, &party);
        if random {
            random_battle(&mut machine, map, 1, 0);
        } else {
            battle(&mut machine, 3);
        }
        machine.write_wram(ENEMY_OFFSET, &[0, 0]).unwrap();
        machine.write_wram(SLOT_UNITS, &[unit]).unwrap();
        machine.write_wram(BATTLE_EXP, &[0, 0, 0]).unwrap();
        run_stub(&mut machine, &[0x1D_8C33]);
        triple(&machine, BATTLE_EXP)
    };
    assert_eq!(award_after(true), want(Kind::Random));
    assert_eq!(award_after(false), want(Kind::Story));
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn the_difficulty_pack_does_not_tell_the_kinds_apart_and_leaves_work_ram_alone() {
    let party = [(0_u8, 46_u8), (1, 46), (2, 46)];
    let (map, group) = namek_map();
    let mut old = boot(&[pack(None)], true, None);
    stage_party(&mut old, &party);
    let (_, random) = random_battle(&mut old, map, 1, 0);
    let mut reference = boot(&[pack(None)], true, None);
    stage_party(&mut reference, &party);
    let story = battle(&mut reference, group);
    assert_eq!(random, story, "a random encounter and a story battle are alike for the difficulty");
    for (slot, (now, then)) in random.iter().zip(&story).enumerate() {
        if now.flags != 0 {
            assert_eq!(now.level, 46, "normal: the party's level, slot {slot}");
        }
        assert_eq!(now, then);
    }
    let marker = (KIND_MARKER_AT - 0x7E_0000) as usize;
    let mut bare = boot(&[], true, None);
    stage_party(&mut bare, &party);
    random_battle(&mut bare, map, 1, 0);
    assert_eq!(
        old.wram()[marker..marker + 6],
        bare.wram()[marker..marker + 6],
        "the bytes of the kind are not written"
    );
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn the_cheats_wrappers_around_the_pick_and_the_builder_leave_the_kinds_apart() {
    // A typical setup: enemy power x2 on, so the cheat wrappers of the pick (`02:D34A`), the group and the
    // builder are all in the path of a random encounter.
    let scaling = Scaling::Offsets { random: -3, story: 3 };
    let party = [(0_u8, 46_u8), (1, 46), (2, 46)];
    let sixths = 6 * 46;
    let (map, group) = namek_map();
    let mut cheating = Cheats::neutral();
    let steps = Setting::Power.steps();
    cheating
        .set_index(Setting::Power, steps.iter().position(|m| *m == Multiplier::new(2, 1)).unwrap());
    let mut original = boot(&[], true, None);
    stage_party(&mut original, &party);
    let plain = battle(&mut original, group);
    let mut machine = boot(&[pack_offsets(-3, 3)], true, Some(&cheating));
    stage_party(&mut machine, &party);
    let (picked, random) = random_battle(&mut machine, map, 1, 0);
    assert_eq!(picked, group);
    let story = battle(&mut machine, group);
    for (slot, (now, then)) in random.iter().zip(&plain).enumerate() {
        let want = expected_at(then, sixths, scaling, Kind::Random);
        assert_eq!((now.level, now.hp, now.bp), (want.level, want.hp * 2, want.bp * 2), "{slot}");
    }
    for (slot, (now, then)) in story.iter().zip(&plain).enumerate() {
        let want = expected_at(then, sixths, scaling, Kind::Story);
        assert_eq!((now.level, now.hp, now.bp), (want.level, want.hp * 2, want.bp * 2), "{slot}");
    }
    assert_eq!(random.iter().find(|r| r.flags != 0).unwrap().level, 43);
    assert_eq!(story.iter().find(|r| r.flags != 0).unwrap().level, 49);
}
