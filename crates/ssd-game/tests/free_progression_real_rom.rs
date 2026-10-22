//! The feature `free_progression` against the real game (M10, Task 19): each gate of the story that read a level is run in the
//! game's own code, staged by a `JSL` written in the upper work RAM as the other real-ROM tests do, with the hooks of the feature
//! (the example pack `examples/free-progression`) and without them, and the story's variable that the gate decides is read after.
//!
//! The first two worlds are the original's: the plain machine, and the machine with the redirectors installed but no pack (the
//! hooks decline: it must give the plain machine's numbers). The third is the machine of the pack.
//!
//! Needs the game's ROM:
//! `SSD_ROM_PATH=... cargo test -p ssd-game --release --test free_progression_real_rom -- --ignored --nocapture`.

use ssd_oracle::{
    DEFAULT_WRAM_SEED, HookTable, Joypad, Machine, MemorySaveStore, NullAudio, StatusRegister,
    Stopped, Stops,
};
use std::path::{Path, PathBuf};

/// Where the test stages its call: the upper work RAM, which the game's boot does not clear.
const CALL: u32 = 0x7F_FE00;
const BACK: u32 = CALL + 4;
/// The frames the game boots for before a case is staged.
const BOOT_FRAMES: u64 = 600;

/// The work RAM the cases stage and read (bus addresses).
const PARTY: u32 = 0x7E_0B00;
const RECORD: u32 = 0x20;
const STORY_FLAGS: u32 = 0x7E_0170;
const SCENE_ID: u32 = 0x7E_01D8;
const SCENE_STATE: u32 = 0x7E_1648;
const MAP_ID: u32 = 0x7E_01A0;
const ENCOUNTER_LEVEL: u32 = 0x7E_0D8A;
const ENCOUNTER_ZONE: u32 = 0x7E_0D8B;
const ENCOUNTER_GROUP: u32 = 0x7E_0EA3;
const MAP_PARAM: u32 = 0x7E_01BB;
const PARTY_COUNT: u32 = 0x7E_0D9A;
const ROSTER: u32 = 0x7E_2800;
const PARTY_ORDER_TAIL: u32 = 0x7E_29A0;

const GOKU: u8 = 0;
const GOHAN: u8 = 1;
const PICCOLO: u8 = 2;
/// The group of the last fight (Frieza's final form), and the unit of the Super Saiyan Goku.
const LAST_GROUP: u8 = 0x56;
const SUPER_SAIYAN: u8 = 0x3B;

fn rom() -> Vec<u8> {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let bytes = std::fs::read(path).unwrap();
    let rom = ssd_rom::normalize(&bytes, None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32, "not the canonical ROM");
    rom.bytes
}

/// The example pack, with its `ginyu_arrival` replaced when `arrival` is given, as the folder of a pack in the build's scratch
/// directory (the example is read-only).
fn pack_dir(arrival: Option<u8>) -> PathBuf {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/free-progression");
    let Some(arrival) = arrival else { return example };
    let text = std::fs::read_to_string(example.join("pack.toml")).unwrap();
    assert!(text.contains("ginyu_arrival = 3"), "the example pack gives the last arrival");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("free-progression-{arrival}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("pack.toml"),
        text.replace("ginyu_arrival = 3", &format!("ginyu_arrival = {arrival}")),
    )
    .unwrap();
    dir
}

/// The image of the machine that runs the pack folder `dir`, checked as the game checks a pack at load.
fn image(rom: &[u8], dir: &Path) -> Vec<u8> {
    let packs = [ssd_mod::Pack::open(dir).unwrap()];
    let options = ssd_mod::CheckOptions { allow_rom_bytes: false };
    if let Err(errors) = ssd_mod::check_set(rom, &packs, options) {
        panic!("the pack is refused: {errors:?}");
    }
    ssd_mod::build(rom, &packs).unwrap().bytes
}

/// What runs the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum World {
    /// The plain machine: no hooks at all.
    Plain,
    /// The redirectors of the packs installed, no pack: every hook declines.
    NoPack,
    /// The pack of the example, with this `ginyu_arrival` (3 is the example's own).
    Pack(u8),
}

/// A booted machine of a world and the state to start every case from.
struct Rig {
    machine: Machine,
    state: Vec<u8>,
}

impl Rig {
    fn new(rom: &[u8], world: World) -> Self {
        let saves = Box::new(MemorySaveStore::default());
        let mut machine = match world {
            World::Plain | World::NoPack => Machine::new(rom.to_vec(), saves).unwrap(),
            World::Pack(arrival) => {
                let dir = pack_dir((arrival != 3).then_some(arrival));
                Machine::with_image(image(rom, &dir), ssd_rom::crc32(rom), saves, DEFAULT_WRAM_SEED)
                    .unwrap()
            }
        };
        if world != World::Plain {
            let mut table = HookTable::new();
            ssd_game::mods::install(&mut table);
            machine.set_hook_table(Some(table));
        }
        let input = &mut |_frame: u64| Joypad::default();
        while machine.frame_count() < BOOT_FRAMES {
            machine.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        }
        // Between two instructions: the registers are not taken in the middle of one or of an interrupt's entry.
        let stops = Stops { instructions: Some(1), ..Stops::default() };
        let stopped = machine.run(&stops, input, &mut NullAudio, 4).unwrap();
        assert!(matches!(stopped, Stopped::Instructions), "{stopped:?}");
        let state = machine.save_state();
        Self { machine, state }
    }

    /// Back to the booted state, with the work RAM the cases stage cleared: the party, the story flags and the roster.
    fn reset(&mut self) {
        self.machine.load_state(&self.state).unwrap();
        self.write(PARTY, &[0; 0x120]);
        self.write(STORY_FLAGS, &[0; 16]);
        self.write(ROSTER, &[0; 0x180]);
        self.write(PARTY_ORDER_TAIL, &[0xFF]);
    }

    fn write(&mut self, address: u32, bytes: &[u8]) {
        self.machine.write_wram(address, bytes).unwrap();
    }

    fn byte(&self, address: u32) -> u8 {
        self.machine.wram()[(address - 0x7E_0000) as usize]
    }

    fn bytes(&self, address: u32, len: usize) -> Vec<u8> {
        let at = (address - 0x7E_0000) as usize;
        self.machine.wram()[at..at + len].to_vec()
    }

    /// A party record: in the party (`$C0`), the character, the level; the unit is the character's own.
    fn member(&mut self, slot: u32, character: u8, level: u8) {
        let mut record = [0_u8; RECORD as usize];
        record[0] = 0xC0;
        record[1] = character;
        record[2] = level;
        record[0x1C] = character;
        self.write(PARTY + slot * RECORD, &record);
    }

    /// Story flags set, by number (0 to 127).
    fn flags(&mut self, numbers: &[u8]) {
        let mut bytes = [0_u8; 16];
        for &n in numbers {
            bytes[usize::from(n / 8)] |= 1 << (n % 8);
        }
        self.write(STORY_FLAGS, &bytes);
    }

    /// Runs the routine at `entry` as a subroutine (`JSL`) from an instruction boundary, to its return.
    fn call(&mut self, entry: u32) {
        let jsl = [0x22, entry as u8, (entry >> 8) as u8, (entry >> 16) as u8];
        self.write(CALL, &jsl);
        let mut registers = self.machine.cpu_registers();
        registers.pbr = (CALL >> 16) as u8;
        registers.pc = (CALL & 0xFFFF) as u16;
        registers.dbr = 0;
        registers.d = 0;
        registers.p = StatusRegister::from(0x24);
        self.machine.set_cpu_registers(registers);
        self.machine.set_breakpoint(BACK);
        let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
        let input = &mut |_frame: u64| Joypad::default();
        for _ in 0..4 {
            let stopped = self.machine.run(&stops, input, &mut NullAudio, 1).unwrap();
            if matches!(stopped, Stopped::Breakpoint { .. }) {
                self.machine.clear_breakpoint(BACK);
                return;
            }
        }
        panic!("${entry:06X} did not return");
    }

    /// The arrivals at each hook of the pack, by name (the hooks of `free_progression`), and the contract mismatches.
    fn hook_calls(&self) -> Vec<(String, u64, u64)> {
        self.machine
            .hook_stats()
            .into_iter()
            .filter(|(name, _)| name.starts_with("mod_fp_"))
            .map(|(name, stats)| (name, stats.calls, stats.contract_mismatch))
            .collect()
    }

    fn calls_of(&self, name: &str) -> u64 {
        self.hook_calls().iter().find(|(n, _, _)| n == name).map_or(0, |(_, calls, _)| *calls)
    }

    /// No hook of the feature was run with a width of the accumulator other than the one its instruction is read with.
    fn assert_contracts(&self) {
        for (name, calls, mismatch) in self.hook_calls() {
            assert_eq!(mismatch, 0, "{name} ({calls} arrivals) met its width contract");
        }
    }
}

/// The three worlds booted once each.
struct Rigs {
    plain: Rig,
    no_pack: Rig,
    pack: Rig,
}

impl Rigs {
    fn new() -> Self {
        let rom = rom();
        Self {
            plain: Rig::new(&rom, World::Plain),
            no_pack: Rig::new(&rom, World::NoPack),
            pack: Rig::new(&rom, World::Pack(3)),
        }
    }

    /// Runs `case` in the three worlds and returns what it read, `(plain, no_pack, pack)`. The machine with no pack must read
    /// what the plain machine reads.
    fn run<T: PartialEq + std::fmt::Debug>(
        &mut self,
        label: &str,
        case: impl Fn(&mut Rig) -> T,
    ) -> (T, T) {
        let plain = case(&mut self.plain);
        let no_pack = case(&mut self.no_pack);
        assert_eq!(plain, no_pack, "{label}: the redirectors with no pack are the original's");
        let pack = case(&mut self.pack);
        (plain, pack)
    }
}

/// The map 0 routine (`03:F212`, the Raditz check) with Goku and Piccolo at the given levels, flag 3 set (the Gohan card), the
/// imposter beaten or not, and flag `0x0B` as given. Returns whether the forced fight started (scene 2).
fn raditz(
    rig: &mut Rig,
    goku: u8,
    piccolo: u8,
    imposter: bool,
    card: bool,
    started_already: bool,
) -> bool {
    rig.reset();
    rig.member(0, GOKU, goku);
    rig.member(1, PICCOLO, piccolo);
    let mut flags = vec![0, 2];
    if card {
        flags.push(3);
    }
    if imposter {
        flags.push(0x0A);
    }
    if started_already {
        flags.push(0x0B);
    }
    rig.flags(&flags);
    rig.write(SCENE_ID, &[0xFF]);
    rig.write(SCENE_STATE, &[0]);
    rig.call(0x03_F212);
    let started = rig.byte(SCENE_STATE) == 0x80;
    if started {
        assert_eq!(rig.byte(SCENE_ID), 2, "the forced fight is scene 2");
    }
    started
}

/// Every key of the feature is the instruction the note `docs/re/data/free-progression.md` names, and so is the one it resumes at:
/// `(address, the opcode and operand bytes)`, read from the ROM (a load of a level or a compare that follows it).
#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn the_hooks_sit_on_the_instructions_the_note_names() {
    let rom = rom();
    let at = |address: u32, len: usize| {
        let offset = (address >> 16) as usize * 0x8000 + (address as usize & 0x7FFF);
        rom[offset..offset + len].to_vec()
    };
    let sites: [(u32, &[u8]); 15] = [
        (0x03_F22C, &[0xB9, 0x02, 0x0B]),       // LDA $0B02,Y: Goku's level
        (0x03_F22F, &[0xC9, 0x05]),             // CMP #$05
        (0x03_F239, &[0xB9, 0x02, 0x0B]),       // LDA $0B02,Y: Piccolo's level
        (0x03_F23C, &[0xC9, 0x04]),             // CMP #$04
        (0x02_D477, &[0xA9, 0x00]),             // LDA #$00: the entry of the stop
        (0x03_F30B, &[0xC9, 0x09]),             // CMP #$09
        (0x03_F31B, &[0xC9, 0x0A]),             // CMP #$0A
        (0x05_8794, &[0xC9, 0x2F]),             // CMP #$2F
        (0x03_868E, &[0xC9, 0x2F]),             // CMP #$2F
        (0x03_EDF2, &[0xAF, 0x02, 0x28, 0x7E]), // LDA $7E2802: Goku's roster level
        (0x03_EDF6, &[0xC9, 0x22]),             // CMP #$22
        (0x03_C59C, &[0xBF, 0xBA, 0xC5, 0x03]), // LDA $03C5BA,X
        (0x03_C5A0, &[0x99, 0x07, 0x0B]),       // STA $0B07,Y
        (0x03_C5A7, &[0xBF, 0xBC, 0xC5, 0x03]), // LDA $03C5BC,X
        (0x03_C5AB, &[0x99, 0x07, 0x0B]),       // STA $0B07,Y
    ];
    for (address, bytes) in sites {
        assert_eq!(at(address, bytes.len()), bytes, "${address:06X}");
    }
}

#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn raditz_comes_with_the_imposter_beaten_and_not_with_a_level() {
    let mut rigs = Rigs::new();
    // (Goku, Piccolo, imposter beaten, the Gohan card, already started): the original's answer, the feature's answer.
    let cases: [(u8, u8, bool, bool, bool, bool, bool); 12] = [
        (1, 1, false, true, false, false, false), // low levels, cave not done: neither
        (5, 1, false, true, false, true, false), // Goku 5: the original starts, the feature waits for the cave
        (1, 4, false, true, false, true, false), // Piccolo 4: the same
        (12, 12, false, true, false, true, false), // far above both levels: the original starts, the feature waits
        (1, 1, true, true, false, false, true),    // low levels, cave done: the feature starts
        (4, 3, true, true, false, false, true),    // just under both thresholds: the feature starts
        (12, 12, true, true, false, true, true),   // both start
        (1, 1, true, false, false, false, false), // no Gohan card: neither (the original's own test comes first)
        (12, 12, true, false, false, false, false),
        (12, 12, true, true, true, false, false), // already started (flag 0x0B): neither
        (1, 1, true, true, true, false, false),
        (5, 4, false, true, false, true, false),
    ];
    for (goku, piccolo, imposter, card, again, original, feature) in cases {
        let label = format!(
            "Goku {goku}, Piccolo {piccolo}, imposter {imposter}, card {card}, started {again}"
        );
        let (plain, pack) =
            rigs.run(&label, |rig| raditz(rig, goku, piccolo, imposter, card, again));
        assert_eq!(plain, original, "{label}: the original");
        assert_eq!(pack, feature, "{label}: the feature");
    }
    rigs.pack.assert_contracts();
    assert!(rigs.pack.calls_of("mod_fp_raditz_goku") > 0, "the hook at Goku's load ran");
    assert!(rigs.pack.calls_of("mod_fp_raditz_piccolo") > 0, "the hook at Piccolo's load ran");
    println!("Raditz: the imposter decides, not the level ({} cases, three worlds)", cases.len());
}

/// The stop of the random battles of maps 0 and 2 (`02:D428`, which jumps to `02:D477`): the encounter level after, with Goku and
/// Piccolo at the given levels and a zone that would fight (level 5, zone 3).
fn encounters(rig: &mut Rig, map: u8, goku: u8, piccolo: u8) -> (u8, u8) {
    rig.reset();
    rig.member(0, GOKU, goku);
    rig.member(1, PICCOLO, piccolo);
    rig.write(MAP_ID, &[map]);
    rig.write(ENCOUNTER_LEVEL, &[5]);
    rig.write(ENCOUNTER_ZONE, &[3]);
    rig.call(0x02_D428);
    (rig.byte(ENCOUNTER_LEVEL), rig.byte(ENCOUNTER_ZONE))
}

#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn random_battles_on_maps_0_and_2_do_not_stop_on_a_level() {
    let mut rigs = Rigs::new();
    for map in [0_u8, 2] {
        for (goku, piccolo, stops) in
            [(1, 1, false), (4, 3, false), (5, 1, true), (1, 4, true), (12, 12, true)]
        {
            let label = format!("map {map}, Goku {goku}, Piccolo {piccolo}");
            let (plain, pack) = rigs.run(&label, |rig| encounters(rig, map, goku, piccolo));
            assert_eq!(plain, if stops { (0, 0) } else { (5, 3) }, "{label}: the original");
            assert_eq!(pack, (5, 3), "{label}: the feature never stops them");
        }
    }
    assert!(rigs.pack.calls_of("mod_fp_encounters") > 0, "the hook at the stop ran");
    rigs.pack.assert_contracts();
    println!("random battles: the stop is the original's without the pack and never with it");
}

/// The level check of map 5 (`03:F300`): the deadline's flag byte (`$0173`) after, with one party member of the given character.
fn deadline(rig: &mut Rig, character: u8, level: u8) -> u8 {
    rig.reset();
    rig.member(0, character, level);
    rig.flags(&[1, 2]);
    rig.call(0x03_F300);
    rig.byte(STORY_FLAGS + 3)
}

#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn the_recruits_deadline_is_off_with_the_feature_alone() {
    let mut rigs = Rigs::new();
    for (character, level, set) in [
        (PICCOLO, 8, false),
        (PICCOLO, 9, true),
        (PICCOLO, 40, true),
        (GOHAN, 9, false),
        (GOHAN, 10, true),
        (GOHAN, 30, true),
    ] {
        let label = format!("character {character} at level {level}");
        let (plain, pack) = rigs.run(&label, |rig| deadline(rig, character, level));
        assert_eq!(
            plain & 0x20 != 0,
            set,
            "{label}: the original sets flag 0x1D at the deadline: ${plain:02X}"
        );
        assert_eq!(pack & 0x20, 0, "{label}: the feature leaves the recruits alone: ${pack:02X}");
    }
    assert!(rigs.pack.calls_of("mod_fp_deadline_piccolo") > 0);
    assert!(rigs.pack.calls_of("mod_fp_deadline_gohan") > 0);
    rigs.pack.assert_contracts();
    println!(
        "deadline: flag 0x1D never set by a level with the feature, with the cheats' story fix absent"
    );
}

/// The Super Saiyan builder's guard (`03:8683`) in the last fight's group, with Goku at `level` and a friend who is not in the way
/// of the party-size test: Goku's unit and level after.
fn super_saiyan(rig: &mut Rig, level: u8, group: u8) -> (u8, u8) {
    rig.reset();
    rig.member(0, GOKU, level);
    rig.member(1, GOHAN, 30);
    rig.flags(&[0, 1]);
    rig.write(ENCOUNTER_GROUP, &[group]);
    rig.write(MAP_PARAM, &[0]);
    rig.write(PARTY_COUNT, &[3]);
    rig.call(0x03_8683);
    (rig.byte(PARTY + 1), rig.byte(PARTY + 2))
}

#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn goku_turns_super_saiyan_by_the_story_whatever_his_level() {
    let mut rigs = Rigs::new();
    for (level, built_by_original) in [(1, false), (20, false), (46, false), (47, true)] {
        let label = format!("Goku at level {level}");
        let (plain, pack) = rigs.run(&label, |rig| super_saiyan(rig, level, LAST_GROUP));
        assert_eq!(plain.0 == SUPER_SAIYAN, built_by_original, "{label}: the original {plain:?}");
        assert_eq!(pack.0, SUPER_SAIYAN, "{label}: the feature builds the form {pack:?}");
        assert_eq!(pack.1, 48, "{label}: the form is level 48 as the original builds it");
    }
    // The story's other conditions stay: another group is no transformation, for either.
    let (plain, pack) = rigs.run("another group", |rig| super_saiyan(rig, 20, 0x55));
    assert_ne!(plain.0, SUPER_SAIYAN);
    assert_ne!(pack.0, SUPER_SAIYAN, "the group of the last fight is still asked for");
    assert!(rigs.pack.calls_of("mod_fp_ssj_build") > 0, "the hook at the builder's guard ran");
    rigs.pack.assert_contracts();
    println!(
        "Super Saiyan builder: level 1 to 47 all build the form with the feature, only 47 in the original"
    );
}

/// The battle state that starts Goku's transformation (`05:872A`, substate 2: it ends in `05:8772..87B3`), in the last fight's group
/// with a friend fallen (`$14D2`), Goku acting for the party (side A) at `level`: whether the transformation phase (`$1A`) starts.
fn transformation_phase(rig: &mut Rig, level: u8, group: u8, fallen: bool) -> bool {
    rig.reset();
    rig.write(0x7E_00AC, &[2]);
    rig.write(0x7E_00AB, &[0]);
    rig.write(ENCOUNTER_GROUP, &[group]);
    rig.write(0x7E_14C2, &[0]);
    let mut record = [0_u8; RECORD as usize];
    record[0] = 0xC0;
    record[2] = level;
    rig.write(0x7E_1400, &record);
    rig.write(0x7E_14D2, &[u8::from(fallen)]);
    rig.call(0x05_872A);
    rig.byte(0x7E_00AB) == 0x1A
}

#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn the_battle_asks_no_level_of_goku_before_the_transformation() {
    let mut rigs = Rigs::new();
    for (level, fallen, group, original) in [
        (1, true, 0x56, false),
        (46, true, 0x56, false),
        (47, true, 0x56, true),
        (1, false, 0x56, false), // nobody has fallen: neither
        (47, false, 0x56, false),
        (47, true, 0x55, false), // not the last fight's group: neither
        (47, true, 0x6E, true),  // the other group of the last fight
        (20, true, 0x6E, false),
    ] {
        let label = format!("Goku {level}, friend fallen {fallen}, group {group:#04X}");
        let (plain, pack) = rigs.run(&label, |rig| transformation_phase(rig, level, group, fallen));
        assert_eq!(plain, original, "{label}: the original");
        assert_eq!(pack, fallen && (group == 0x56 || group == 0x6E), "{label}: the feature");
    }
    assert!(rigs.pack.calls_of("mod_fp_ssj_trigger") > 0, "the hook at the battle's compare ran");
    rigs.pack.assert_contracts();
    println!(
        "battle transformation: level 1 to 47 start the phase with the feature, only 47 in the original"
    );
}

/// The map 16 set-up (`03:ED9C`): the flags `0x10` to `0x12` (`$0172` bits 0 to 2) after, with Goku's level in the roster image.
fn ginyu(rig: &mut Rig, level: u8) -> u8 {
    rig.reset();
    rig.write(ROSTER + 2, &[level]);
    rig.call(0x03_ED9C);
    rig.byte(STORY_FLAGS + 2) & 7
}

#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn goku_arrives_at_the_ginyu_squad_as_the_pack_says() {
    let rom = rom();
    let mut plain = Rig::new(&rom, World::Plain);
    let mut no_pack = Rig::new(&rom, World::NoPack);
    let levels = [1_u8, 20, 33, 34, 35, 36, 40, 47];
    let original: Vec<u8> = levels.iter().map(|&level| ginyu(&mut plain, level)).collect();
    assert_eq!(
        original,
        [1, 1, 1, 2, 2, 4, 4, 4],
        "the original reads Goku's level: flags 0x10, 0x11, 0x12"
    );
    let without: Vec<u8> = levels.iter().map(|&level| ginyu(&mut no_pack, level)).collect();
    assert_eq!(without, original, "the redirectors with no pack are the original's");
    for (arrival, wanted) in [(0_u8, None), (1, Some(1_u8)), (2, Some(2)), (3, Some(4))] {
        let mut pack = Rig::new(&rom, World::Pack(arrival));
        let flags: Vec<u8> = levels.iter().map(|&level| ginyu(&mut pack, level)).collect();
        match wanted {
            Some(bits) => {
                assert_eq!(
                    flags,
                    vec![bits; levels.len()],
                    "arrival {arrival}: the same at every level"
                );
            }
            None => assert_eq!(flags, original, "arrival 0 is the original's rule"),
        }
        assert!(pack.calls_of("mod_fp_ginyu_arrival") > 0, "the hook ran for arrival {arrival}");
        pack.assert_contracts();
        println!("ginyu_arrival {arrival}: flags at levels {levels:?} = {flags:?}");
    }
}

/// Effect `$35` (`03:C522`, Piccolo returns): Piccolo's BP in the first party record after, with his record in the roster at
/// `level` and a BP of 123 456.
fn piccolo_returns(rig: &mut Rig, level: u8) -> u32 {
    rig.reset();
    let mut record = [0_u8; RECORD as usize];
    record[0] = 0xC0;
    record[1] = 0x01;
    record[2] = level;
    record[7..10].copy_from_slice(&123_456_u32.to_le_bytes()[..3]);
    record[0x0A..0x0C].copy_from_slice(&300_u16.to_le_bytes());
    record[0x0C..0x0E].copy_from_slice(&50_u16.to_le_bytes());
    record[0x1C] = PICCOLO;
    rig.write(ROSTER + u32::from(PICCOLO) * RECORD, &record);
    rig.call(0x03_C522);
    assert_eq!(rig.byte(PARTY + 0x1C), PICCOLO, "Piccolo leads the party after the effect");
    assert_eq!(rig.byte(PARTY + 2), level, "his level is the roster's");
    let bp = rig.bytes(PARTY + 7, 3);
    u32::from(bp[0]) | u32::from(bp[1]) << 8 | u32::from(bp[2]) << 16
}

#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn piccolos_return_keeps_his_bp_at_a_level_the_table_does_not_hold() {
    let rom = rom();
    // The table of effect $35: three bytes of BP for each level from 33, at 03:C5BA (the file offset of that bank 3 address).
    let table = |level: u8| {
        let at = 3 * 0x8000 + (0xC5BA - 0x8000) + 3 * usize::from(level - 33);
        u32::from(rom[at]) | u32::from(rom[at + 1]) << 8 | u32::from(rom[at + 2]) << 16
    };
    let mut rigs = Rigs::new();
    for level in 33..=40_u8 {
        let (plain, pack) = rigs.run(&format!("level {level}"), |rig| piccolo_returns(rig, level));
        assert_eq!(plain, table(level), "level {level}: the original reads its table");
        assert_eq!(
            pack,
            table(level),
            "level {level}: the feature leaves a level of the table alone"
        );
    }
    for level in [1_u8, 13, 20, 32, 41, 44, 47, 48, 60] {
        let (plain, pack) = rigs.run(&format!("level {level}"), |rig| piccolo_returns(rig, level));
        assert_ne!(
            plain, 123_456,
            "level {level}: the original assigns a BP it read past the table ({plain})"
        );
        assert_eq!(pack, 123_456, "level {level}: the feature keeps his BP ({pack})");
    }
    assert!(rigs.pack.calls_of("mod_fp_piccolo_bp_low") > 0);
    assert!(rigs.pack.calls_of("mod_fp_piccolo_bp_high") > 0);
    rigs.pack.assert_contracts();
    println!(
        "effect 35: the table's levels 33 to 40 as the original, any other level keeps the BP"
    );
}

/// The floors and gifts that raise a level with `03:C64A` (`$164C` the character, `$164D` the levels): Goku's level after the
/// floor of map 7's set-up applied to Goku at `level` (a floor of 8: `$164D` = 8 - level).
fn floor_of_8(rig: &mut Rig, level: u8) -> u8 {
    rig.reset();
    rig.member(0, GOKU, level);
    rig.write(0x7E_164C, &[GOKU]);
    rig.write(0x7E_164D, &[8_u8.wrapping_sub(level)]);
    rig.call(0x03_C64A);
    rig.byte(PARTY + 2)
}

#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn a_floor_applied_above_its_level_adds_one_level_and_the_feature_leaves_it() {
    let mut rigs = Rigs::new();
    for level in [1_u8, 4, 7, 8, 9, 12, 30] {
        let (plain, pack) = rigs.run(&format!("level {level}"), |rig| floor_of_8(rig, level));
        assert_eq!(plain, pack, "level {level}: the feature does not touch the floors");
        println!("floor of 8 on level {level}: level {plain} after");
    }
}
