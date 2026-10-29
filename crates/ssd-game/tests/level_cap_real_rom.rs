//! The level cap of `free_progression` (`level_cap = 999`) against the real game: the level-up check (`03:841E`) and the raise
//! (`03:C64A` to `03:851B`) are run in the game's own code, staged by a `JSL` in the upper work RAM, on the machine of the example
//! pack `examples/free-progression` and on a machine with the hooks but no pack. Up to the game's own cap the two must give the
//! same record; past it the record must be what the extension gives, and the enemies of a battle follow the party's wide level.
//!
//! Needs the game's ROM:
//! `SSD_ROM_PATH=... cargo test -p ssd-game --release --test level_cap_real_rom -- --ignored --nocapture`.

use ssd_game::mods::state::{
    BLOCK, LEVELS_MARKER_AT, MARKER, MARKER_AT, PARTY_HIGH, exp_high_at, wide_level_at,
};
use ssd_menus::levels::Anchor;
use ssd_oracle::{
    DEFAULT_WRAM_SEED, HookTable, Joypad, Machine, MemorySaveStore, NullAudio, StatusRegister,
    Stopped, Stops,
};
use std::path::Path;

const CALL: u32 = 0x7F_FE00;
const BACK: u32 = CALL + 4;
const BOOT_FRAMES: u64 = 400;
const PARTY: u32 = 0x7E_0B00;
const GAINED: u32 = 0x7E_001F;
const RAISE_CHARACTER: u32 = 0x7E_164C;
const RAISE_LEVELS: u32 = 0x7E_164D;
const GROUP_INDEX: u32 = 0x7E_0EA3;
const ENEMIES: u32 = 0x7E_0E00;

fn rom() -> Vec<u8> {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let bytes = std::fs::read(path).unwrap();
    ssd_rom::normalize(&bytes, None).unwrap().bytes
}

/// One row of a character's table: `(bp, exp, hp, ki, byte 0e)`.
fn game_row(rom: &[u8], id: usize, level: usize) -> (u64, u64, u64, u64, u64) {
    let rom_at = |bank: usize, address: usize| (bank << 15) + (address - 0x8000);
    let pointer = rom_at(2, 0xC13B) + 2 * id;
    let table = rom_at(2, usize::from(u16::from_le_bytes([rom[pointer], rom[pointer + 1]])));
    let row = &rom[table + 11 * (level - 1)..table + 11 * level];
    let u24 =
        |at: usize| u64::from(row[at]) | u64::from(row[at + 1]) << 8 | u64::from(row[at + 2]) << 16;
    (
        u24(0),
        u24(3),
        u64::from(row[6]) | u64::from(row[7]) << 8,
        u64::from(row[8]) | u64::from(row[9]) << 8,
        u64::from(row[10]),
    )
}

/// The curve past the game's cap of character `id`, from its rows in the ROM.
fn anchor(rom: &[u8], id: usize) -> Anchor {
    let cap_at = (3 << 15) + (0x850E - 0x8000) + id;
    let cap = usize::from(rom[cap_at]);
    let (mut bp, mut ki) = (0, 0);
    for level in 1..=cap {
        let row = game_row(rom, id, level);
        bp += row.0;
        ki += row.3;
    }
    let last = game_row(rom, id, cap);
    Anchor { level: cap as u16, bp, ki, max_hp: last.2, exp: last.1, byte_0e: last.4 }
}

struct Rig {
    machine: Machine,
    state: Vec<u8>,
}

impl Rig {
    fn new(rom: &[u8], pack: bool) -> Self {
        let saves = Box::new(MemorySaveStore::default());
        let mut machine = if pack {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/free-progression");
            let packs = [ssd_mod::Pack::open(&dir).unwrap()];
            let options = ssd_mod::CheckOptions { allow_rom_bytes: false };
            ssd_mod::check_set(rom, &packs, options).expect("the pack is accepted");
            let image = ssd_mod::build(rom, &packs).unwrap().bytes;
            Machine::with_image(image, ssd_rom::crc32(rom), saves, DEFAULT_WRAM_SEED).unwrap()
        } else {
            Machine::new(rom.to_vec(), saves).unwrap()
        };
        let mut table = HookTable::new();
        ssd_game::mods::install(&mut table);
        machine.set_hook_table(Some(table));
        while machine.frame_count() < BOOT_FRAMES {
            machine.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        }
        let stops = Stops { instructions: Some(1), ..Stops::default() };
        let stopped = machine.run(&stops, &mut |_| Joypad::default(), &mut NullAudio, 4).unwrap();
        assert!(matches!(stopped, Stopped::Instructions), "{stopped:?}");
        let state = machine.save_state();
        Self { machine, state }
    }

    fn reset(&mut self) {
        self.machine.load_state(&self.state).unwrap();
        self.write(PARTY, &[0; 0x120]);
        self.write(MARKER_AT, &[0; 4]);
        self.write(LEVELS_MARKER_AT, &[0; 4]);
    }

    fn write(&mut self, address: u32, bytes: &[u8]) {
        self.machine.write_wram(address, bytes).unwrap();
    }

    fn bytes(&self, address: u32, len: usize) -> Vec<u8> {
        let at = (address - 0x7E_0000) as usize;
        self.machine.wram()[at..at + len].to_vec()
    }

    fn word(&self, address: u32) -> u16 {
        let b = self.bytes(address, 2);
        u16::from_le_bytes([b[0], b[1]])
    }

    fn triple(&self, address: u32) -> u64 {
        let b = self.bytes(address, 3);
        u64::from(b[0]) | u64::from(b[1]) << 8 | u64::from(b[2]) << 16
    }

    /// A party record in the party: the level, EXP, BP, maximum HP and KI given.
    #[allow(clippy::too_many_arguments)]
    fn member(&mut self, slot: u32, id: u8, level: u8, exp: u64, bp: u64, hp: u16, ki: u16) {
        let mut record = [0_u8; 0x20];
        record[0] = 0xC0;
        record[1] = id;
        record[2] = level;
        record[0x1C] = id;
        record[3..5].copy_from_slice(&hp.to_le_bytes());
        record[0x0A..0x0C].copy_from_slice(&hp.to_le_bytes());
        record[5..7].copy_from_slice(&ki.to_le_bytes());
        record[0x0C..0x0E].copy_from_slice(&ki.to_le_bytes());
        record[7..10].copy_from_slice(&(bp as u32).to_le_bytes()[..3]);
        record[0x0F..0x12].copy_from_slice(&(exp as u32).to_le_bytes()[..3]);
        self.write(PARTY + slot * 0x20, &record);
        if bp >> 24 != 0 {
            self.write(MARKER_AT, &MARKER);
            self.write(BLOCK + PARTY_HIGH as u32 + 2 * slot, &((bp >> 24) as u16).to_le_bytes());
        }
    }

    /// A second record in the party (a friend for the Super Saiyan's party-size test), without touching the first.
    fn member_keep(&mut self, slot: u32, id: u8, level: u8) {
        let mut record = [0_u8; 0x20];
        record[0] = 0xC0;
        record[1] = id;
        record[2] = level;
        record[0x1C] = id;
        self.write(PARTY + slot * 0x20, &record);
    }

    fn call(&mut self, entry: u32, y: u16) {
        let jsl = [0x22, entry as u8, (entry >> 8) as u8, (entry >> 16) as u8];
        self.write(CALL, &jsl);
        let mut registers = self.machine.cpu_registers();
        registers.pbr = (CALL >> 16) as u8;
        registers.pc = (CALL & 0xFFFF) as u16;
        registers.dbr = 0;
        registers.d = 0;
        registers.y = y;
        registers.p = StatusRegister::from(0x24);
        self.machine.set_cpu_registers(registers);
        for stale in self.machine.breakpoints().clone() {
            self.machine.clear_breakpoint(stale);
        }
        self.machine.set_breakpoint(BACK);
        let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
        for _ in 0..6 {
            let stopped =
                self.machine.run(&stops, &mut |_| Joypad::default(), &mut NullAudio, 1).unwrap();
            if matches!(stopped, Stopped::Breakpoint { .. }) {
                return;
            }
        }
        panic!("${entry:06X} did not return");
    }

    /// The wide level, the EXP and the BP of party slot 0 as the mod state keeps them.
    fn wide(&self, id: u8) -> (u16, u64, u64) {
        let record = PARTY;
        let level = self.word(wide_level_at(id));
        let exp = self.triple(record + 0x0F) | u64::from(self.bytes(exp_high_at(id), 1)[0]) << 24;
        let live = ssd_game::mods::state::block_live(self.machine.wram());
        let high = if live { u64::from(self.word(BLOCK + PARTY_HIGH as u32)) } else { 0 };
        (level, exp, self.triple(record + 7) | high << 24)
    }
}

#[test]
#[ignore = "needs the game's ROM (SSD_ROM_PATH)"]
fn the_level_cap_is_the_games_own_up_to_its_cap_and_the_extension_past_it() {
    let rom = rom();
    let mut plain = Rig::new(&rom, false);
    let mut pack = Rig::new(&rom, true);

    // 1. Gohan, cap 48: from level 30 to 48 the two machines write the same record, byte for byte.
    let (id, from) = (1_u8, 30_u8);
    let rows: Vec<_> = (1..=48).map(|l| game_row(&rom, usize::from(id), l)).collect();
    let bp: u64 = rows[..usize::from(from)].iter().map(|r| r.0).sum();
    let ki: u64 = rows[..usize::from(from)].iter().map(|r| r.3).sum();
    let mut records = Vec::new();
    for rig in [&mut plain, &mut pack] {
        rig.reset();
        rig.member(
            0,
            id,
            from,
            rows[usize::from(from) - 1].1,
            bp,
            rows[usize::from(from) - 1].2 as u16,
            ki as u16,
        );
        // The EXP of the 48th level, and more: the original stops at its cap 48.
        rig.write(PARTY + 0x0F, &(rows[47].1 as u32 + 5000).to_le_bytes()[..3]);
        rig.write(GAINED, &[0]);
        rig.call(0x03_841E, 0);
        records.push((rig.bytes(PARTY, 0x20), rig.bytes(GAINED, 1)));
    }
    assert_eq!(records[0].0[2], 48, "the original reaches its cap");
    assert_eq!(records[1], records[0], "the pack's level-up check is the original's up to the cap");

    // 2. The same raise by the story's floor (`03:C64A`, 8 levels on a level 4 Goku), in both machines.
    let mut floors = Vec::new();
    for rig in [&mut plain, &mut pack] {
        rig.reset();
        rig.member(0, 0, 4, 0, 0, 100, 10);
        rig.write(RAISE_CHARACTER, &[0]);
        rig.write(RAISE_LEVELS, &[8 - 4]);
        rig.call(0x03_C64A, 0);
        floors.push(rig.bytes(PARTY, 0x20));
    }
    assert_eq!(floors[0][2], 9, "a floor of 8 gives level 9");
    assert_eq!(floors[1], floors[0], "the pack's raise is the original's below the cap");

    // 3. Goku past 47: raised to level 100 (the byte saturates at 99), then by EXP to a higher level.
    let goku = anchor(&rom, 0);
    assert_eq!(goku.level, 47);
    let rows_goku: Vec<_> = (1..=47).map(|l| game_row(&rom, 0, l)).collect();
    pack.reset();
    pack.member(0, 0, 47, rows_goku[46].1, goku.bp, rows_goku[46].2 as u16, goku.ki as u16);
    pack.write(RAISE_CHARACTER, &[0]);
    pack.write(RAISE_LEVELS, &[52]); // 47 + 52 = 99, +1: level 100.
    pack.call(0x03_C64A, 0);
    let (level, exp, bp) = pack.wide(0);
    assert_eq!(level, 100);
    assert_eq!(pack.bytes(PARTY + 2, 1)[0], 99, "the record's byte saturates at 99");
    let at = goku.at(100);
    assert_eq!((exp, bp), (at.exp, at.bp), "EXP and BP of level 100 are the extension's");
    assert_eq!(u64::from(pack.word(PARTY + 0x0A)), u64::from(at.max_hp));
    assert_eq!(u64::from(pack.word(PARTY + 0x0C)), at.ki);
    println!("level 100: BP {} HP {} KI {} EXP {}", at.bp, at.max_hp, at.ki, at.exp);

    // The level-up check after a fight: the EXP of level 150 and more.
    let target = goku.at(150);
    pack.write(PARTY + 0x0F, &(target.exp as u32 + 10).to_le_bytes()[..3]);
    pack.write(exp_high_at(0), &[((target.exp + 10) >> 24) as u8]);
    pack.call(0x03_841E, 0);
    let (level, _, bp) = pack.wide(0);
    assert_eq!(level, 150, "levelled up by EXP");
    assert_eq!(bp, target.bp);
    assert_eq!(pack.bytes(GAINED, 1)[0], 0x80, "a level was gained");
    println!("level 150: BP {} HP {} KI {}", target.bp, target.max_hp, target.ki);

    // 4. The enemies follow the party's wide level: a boss battle is made for 150 + 3. Both machines start from the same state,
    // so the original's small draw of each BP is the same one.
    let mut original = Vec::new();
    for (rig, raised) in [(&mut plain, false), (&mut pack, true)] {
        rig.reset();
        rig.member(0, 0, 47, rows_goku[46].1, goku.bp, 700, 100);
        if raised {
            rig.write(RAISE_CHARACTER, &[0]);
            rig.write(RAISE_LEVELS, &[102]); // 47 + 102 = 149, +1: level 150.
            rig.call(0x03_C64A, 0);
            assert_eq!(rig.wide(0).0, 150);
        }
        rig.write(GROUP_INDEX, &[0x14]);
        rig.call(0x01_F692, 0);
        rig.call(0x01_F719, 0);
        if !raised {
            for slot in 0..5_u32 {
                let base = ENEMIES + 0x20 * slot;
                original.push((
                    rig.bytes(base, 1)[0],
                    rig.bytes(base + 2, 1)[0],
                    rig.word(base + 0x0A),
                    rig.triple(base + 7),
                ));
            }
        }
    }
    let mut checked = 0;
    for (slot, &(flags, own, hp, bp)) in original.iter().enumerate() {
        if flags == 0 || own == 0 {
            continue;
        }
        let base = ENEMIES + 0x20 * slot as u32;
        let (num, den) = (150_u64 + 3 + 5, u64::from(own) + 5);
        let scale = |value: u64, power: u32| {
            (u128::from(value) * u128::from(num).pow(power) + u128::from(den).pow(power) / 2)
                / u128::from(den).pow(power)
        };
        assert_eq!(pack.bytes(base + 2, 1)[0], 99, "the enemy's byte saturates like the party's");
        assert_eq!(u128::from(pack.word(base + 0x0A)), scale(u64::from(hp), 1).clamp(1, 65535));
        let live = ssd_game::mods::state::block_live(pack.machine.wram());
        let live_high = if live { u64::from(pack.word(BLOCK + 0x12 + 2 * slot as u32)) } else { 0 };
        assert_eq!(
            u128::from(pack.triple(base + 7) | live_high << 24),
            scale(bp, 3).max(1),
            "slot {slot}: BP follows (153 + 5) / (L + 5) cubed"
        );
        checked += 1;
    }
    assert!(checked > 0, "an enemy was scaled");

    // 5. The Super Saiyan form (`03:8683` builds it in the last fight's group) follows Goku's level: at 100 its BP is 4 999 999 times
    // (105 / 52)^3, its HP and KI times 105 / 52, and the byte of the level is the party's own (99), not 48.
    pack.reset();
    pack.member(0, 0, 47, rows_goku[46].1, goku.bp, 700, 100);
    pack.write(RAISE_CHARACTER, &[0]);
    pack.write(RAISE_LEVELS, &[52]);
    pack.call(0x03_C64A, 0);
    pack.member_keep(1, 1, 30);
    pack.write(0x7E_0170, &[0x03]);
    pack.write(GROUP_INDEX, &[0x56]);
    pack.write(0x7E_01BB, &[0]);
    pack.write(0x7E_0D9A, &[3]);
    pack.call(0x03_8683, 0);
    assert_eq!(pack.bytes(PARTY + 1, 1)[0], 0x3B, "the form is built");
    let scale = |value: u64, power: u32| {
        (u128::from(value) * 105_u128.pow(power) + 52_u128.pow(power) / 2) / 52_u128.pow(power)
    };
    let (_, _, bp) = pack.wide(0);
    assert_eq!(u128::from(bp), scale(4_999_999, 3), "the form's BP follows the level");
    assert_eq!(u128::from(pack.word(PARTY + 0x0A)), scale(800, 1));
    assert_eq!(u128::from(pack.word(PARTY + 0x0C)), scale(189, 1));
    assert_eq!(pack.bytes(PARTY + 2, 1)[0], 99, "the level byte is the party's, not 48");
    println!("Super Saiyan at level 100: BP {bp}");
}

// ---- The fights that wait for a floor of HP, at a high level ----

const SIDE_B: u32 = 0x7E_1440;
const ACTIVE_SIDE: u32 = 0x7E_14C2;
const ENEMY_OFFSET: u32 = 0x7E_14C5;
const HIT_FLAG: u32 = 0x7E_14CC;
const HIT_CLASS: u32 = 0x7E_14CD;
const DAMAGE: u32 = 0x7E_14CF;
const BATTLE_EXP: u32 = 0x7E_14F3;
const FLAG_01D6: u32 = 0x7E_01D6;
const SWAP_STATE: u32 = 0x7E_15F1;
const REDIRECT: u32 = 0x7E_1700;
/// Where the hit step's `RTS` lands in these runs (an address of bank 5 that nothing else runs).
const LANDING: u32 = 0x05_8000;

impl Rig {
    /// Runs the code at the 24-bit `entry` (the registers as a battle's code has them: M8, X16) until the CPU is about to fetch at
    /// one of `stops`, and returns that address. With `rts` the routine ends in an `RTS`, which returns to [`LANDING`] (a stop too).
    fn run_from(&mut self, entry: u32, rts: bool, stops: &[u32]) -> u32 {
        let mut registers = self.machine.cpu_registers();
        registers.pbr = (entry >> 16) as u8;
        registers.pc = (entry & 0xFFFF) as u16;
        registers.dbr = 0;
        registers.d = 0;
        registers.p = StatusRegister::from(0x24);
        if rts {
            registers.s = registers.s.wrapping_sub(2);
            let at = 0x7E_0000 + u32::from(registers.s) + 1;
            let landing = ((LANDING & 0xFFFF) as u16).wrapping_sub(1);
            self.write(at, &landing.to_le_bytes());
        }
        self.machine.set_cpu_registers(registers);
        for stale in self.machine.breakpoints().clone() {
            self.machine.clear_breakpoint(stale);
        }
        let mut all = stops.to_vec();
        if rts {
            all.push(LANDING);
        }
        for &stop in &all {
            self.machine.set_breakpoint(stop);
        }
        let run = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
        for _ in 0..12 {
            let stopped =
                self.machine.run(&run, &mut |_| Joypad::default(), &mut NullAudio, 1).unwrap();
            if matches!(stopped, Stopped::Breakpoint { .. }) {
                let at = self.machine.cpu_registers();
                return u32::from(at.pbr) << 16 | u32::from(at.pc);
            }
        }
        panic!("${entry:06X} did not reach one of {stops:06X?}");
    }

    /// Goku at the level `level` (47, the game's own cap, without it), alone in the party, and the battle of `group` built. The hits go to the enemy of `unit` (the
    /// first one in the fight when it is `None`); returns its unit.
    fn battle(&mut self, rom: &[u8], level: Option<u8>, group: u8, unit: Option<u8>) -> u8 {
        let goku = anchor(rom, 0);
        let rows: Vec<_> = (1..=47).map(|l| game_row(rom, 0, l)).collect();
        self.reset();
        self.member(0, 0, 47, rows[46].1, goku.bp, 700, 100);
        if let Some(level) = level {
            self.write(RAISE_CHARACTER, &[0]);
            self.write(RAISE_LEVELS, &[level - 48]);
            self.call(0x03_C64A, 0);
            assert_eq!(self.wide(0).0, u16::from(level));
        }
        self.write(GROUP_INDEX, &[group]);
        self.call(0x01_F692, 0);
        self.call(0x01_F719, 0);
        let slot = (0..5_u32)
            .find(|&slot| {
                let record = self.bytes(ENEMIES + 0x20 * slot, 2);
                record[0] != 0 && unit.is_none_or(|unit| record[1] == unit)
            })
            .unwrap_or_else(|| panic!("an enemy of {unit:02X?} is in group {group:#04x}"));
        let record = self.bytes(ENEMIES + 0x20 * slot, 0x20);
        self.write(SIDE_B, &record);
        self.write(0x7E_17C1 + slot, &[record[1]]); // the unit of each slot, which the EXP award reads
        self.write(ENEMY_OFFSET, &[(0x20 * slot) as u8, 0]);
        self.write(ACTIVE_SIDE, &[0]);
        self.write(FLAG_01D6, &[0x80]);
        self.write(REDIRECT, &[0]);
        self.write(SWAP_STATE, &[0]);
        self.write(BATTLE_EXP, &[0; 3]);
        record[1]
    }

    /// The party's hit on the enemy of side B: `damage` HP, resolved by the game's own hit step (`05:A435`). Returns the side B
    /// record afterwards, and the battle's EXP.
    fn hit(&mut self, damage: u32) -> (Vec<u8>, u64) {
        self.write(HIT_FLAG, &[1]);
        self.write(HIT_CLASS, &[2]);
        self.write(DAMAGE, &damage.to_le_bytes()[..3]);
        let at = self.run_from(0x05_A435, true, &[]);
        assert_eq!(at, LANDING);
        (self.bytes(SIDE_B, 0x20), self.triple(BATTLE_EXP))
    }

    /// The HP of the enemy of side B.
    fn side_hp(&self) -> u64 {
        u64::from(self.word(SIDE_B + 3))
    }

    fn set_side_hp(&mut self, hp: u64) {
        self.write(SIDE_B + 3, &(hp as u16).to_le_bytes());
    }
}

/// The unit row's level of the original table.
fn unit_level(rom: &[u8], unit: u8) -> u64 {
    let at = (2 << 15) + (0xB214 - 0x8000) + 14 * usize::from(unit) + 1;
    u64::from(rom[at])
}

#[test]
#[ignore = "needs the game's ROM (SSD_ROM_PATH)"]
fn the_hp_floor_fights_keep_the_story_when_a_hit_could_skip_the_floor() {
    let rom = rom();
    let mut plain = Rig::new(&rom, false);
    let mut pack = Rig::new(&rom, true);
    // The scaled number of `value` for the enemy of `unit` in a story fight of a party of 150 (the example pack: +3 levels).
    let scaled = |value: u64, unit: u8| {
        let (num, den) = (150_u64 + 3 + 5, unit_level(&rom, unit) + 5);
        ((value * num + den / 2) / den).max(1)
    };

    // Zarbon's first form and Frieza's first three forms: (name, group, unit, the original's floor).
    let fights = [
        ("Zarbon", 0x38_u8, 0x1E_u8, 30_u64),
        ("Frieza 1", 0x53, 0x35, 300),
        ("Frieza 2", 0x54, 0x36, 20),
        ("Frieza 3", 0x55, 0x37, 50),
    ];
    for (name, group, unit, floor) in fights {
        let now = scaled(floor, unit);
        assert!(now > floor, "{name}: the floor is scaled");

        // The original, without the pack: a hit that is too large kills (the floor is judged for a live enemy only); a hit that
        // takes it below the floor withdraws it at the original's floor.
        plain.battle(&rom, None, group, Some(unit));
        let (record, _) = plain.hit(0xFFFF);
        assert_eq!(
            plain.side_hp(),
            0,
            "{name}: the original enemy dies of a hit that is too large ({record:02x?})"
        );
        plain.battle(&rom, None, group, Some(unit));
        plain.set_side_hp(floor + 4);
        let (record, exp) = plain.hit(10);
        assert_eq!(
            (record[0], plain.side_hp(), exp > 0),
            (0, floor, true),
            "{name}: the original withdraws at its floor"
        );

        // The pack at party level 150.
        pack.battle(&rom, Some(150), group, Some(unit));
        let max_hp = pack.side_hp();
        assert!(
            max_hp > 20 * floor.min(50),
            "{name}: a scaled enemy has thousands of HP ({max_hp})"
        );
        let (record, exp) = pack.hit(0xFFFF);
        println!(
            "{name}: floor {floor} -> {now}, HP {max_hp}; a hit of 65535: flags {:#04x} HP {} EXP {exp}",
            record[0],
            pack.side_hp()
        );
        assert_eq!(
            record[0], 0,
            "{name}: the enemy has withdrawn (flags cleared, not knocked out)"
        );
        assert_eq!(pack.side_hp(), now, "{name}: it is left at the scaled floor");
        assert!(exp > 0, "{name}: its reward is paid");

        // A hit to exactly zero is the same.
        pack.battle(&rom, Some(150), group, Some(unit));
        let hp = pack.side_hp();
        let (record, exp) = pack.hit(hp as u32);
        assert_eq!(
            (record[0], pack.side_hp(), exp > 0),
            (0, now, true),
            "{name}: a hit to exactly 0"
        );

        // A hit that leaves the enemy above the floor leaves it fighting; one that takes it below withdraws it at the floor.
        pack.battle(&rom, Some(150), group, Some(unit));
        pack.set_side_hp(now + 40);
        let (record, exp) = pack.hit(39);
        assert_eq!(
            (record[0] != 0, pack.side_hp(), exp),
            (true, now + 1, 0),
            "{name}: above the floor it fights on"
        );
        pack.battle(&rom, Some(150), group, Some(unit));
        pack.set_side_hp(now + 40);
        let (record, exp) = pack.hit(41);
        assert_eq!(
            (record[0], pack.side_hp(), exp > 0),
            (0, now, true),
            "{name}: just below, it withdraws at the floor"
        );
    }

    // The second form's fight is judged at its end by the HP of its first enemy: the story goes on up to the floor.
    for (world, rig, level, floor) in
        [("original", &mut plain, None, 20_u64), ("pack", &mut pack, Some(150), scaled(20, 0x36))]
    {
        for (hp, story) in [(floor, true), (floor + 1, false)] {
            rig.battle(&rom, level, 0x54, Some(0x36));
            rig.write(ENEMIES + 3, &(hp as u16).to_le_bytes());
            let at = rig.run_from(0x05_97F3, false, &[0x05_9831, 0x05_980A]);
            assert_eq!(
                at == 0x05_9831,
                story,
                "{world}: an HP of {hp} at the end of the second form's fight"
            );
        }
    }

    // The Ginyu duel: a hit that would kill him leaves him at the scaled 1 HP until the swap, and kills him after it.
    plain.battle(&rom, None, 0x51, None);
    let (record, _) = plain.hit(0xFFFF);
    assert_eq!(
        (record[0] != 0, plain.side_hp()),
        (true, 1),
        "the original keeps Ginyu at 1 HP ({record:02x?})"
    );
    let unit = pack.battle(&rom, Some(150), 0x51, None);
    let one = scaled(1, unit);
    let (record, exp) = pack.hit(0xFFFF);
    println!(
        "Ginyu (unit {unit:#04x}): 1 HP -> {one}; a hit of 65535 leaves flags {:#04x} HP {}",
        record[0],
        pack.side_hp()
    );
    assert_eq!(
        (record[0] != 0, pack.side_hp(), exp),
        (true, one, 0),
        "Ginyu is kept at the scaled 1 HP"
    );
    pack.battle(&rom, Some(150), 0x51, None);
    pack.write(SWAP_STATE, &[2]);
    let (record, _) = pack.hit(0xFFFF);
    assert_eq!(
        (record[0] & 0x40, pack.side_hp()),
        (0, 0),
        "after the swap he can fall ({record:02x?})"
    );
}
