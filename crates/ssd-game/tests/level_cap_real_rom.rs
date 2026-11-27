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

// ---- The trainings: the gravity trials of the spaceship and the trials' EXP ----

/// `$0136`: the ten counters of the trials (one per gravity), and `03:CD2E`, the effect that pays them to the first record.
const TRIAL_COUNTERS: u32 = 0x7E_0136;
const TRIAL_EXP_EFFECT: u32 = 0x03_CD2E;
/// `05:8E3F`: the win of a gravity trial (+2 levels).
const TRIAL_WIN: u32 = 0x05_8E3F;

/// Goku at level 48 (past his own cap), the EXP and the stats of that level.
fn goku_at_48(rig: &mut Rig, rom: &[u8]) {
    let goku = anchor(rom, 0);
    let rows: Vec<_> = (1..=47).map(|l| game_row(rom, 0, l)).collect();
    rig.reset();
    rig.member(0, 0, 47, rows[46].1, goku.bp, rows[46].2 as u16, goku.ki as u16);
    rig.write(RAISE_CHARACTER, &[0]);
    rig.write(RAISE_LEVELS, &[0]);
    rig.call(0x03_C64A, 0);
}

#[test]
#[ignore = "needs the game's ROM (SSD_ROM_PATH)"]
fn a_won_gravity_trial_gives_two_levels_past_the_games_cap_only_with_the_pack() {
    let rom = rom();
    let goku = anchor(&rom, 0);
    let mut plain = Rig::new(&rom, false);
    let mut pack = Rig::new(&rom, true);

    // The original: Goku at 48 is past his cap, the trial's win leaves him at 48 (and the EXP as the table's following bytes).
    plain.reset();
    let rows: Vec<_> = (1..=47).map(|l| game_row(&rom, 0, l)).collect();
    plain.member(0, 0, 48, rows[46].1, goku.bp, 759, 141);
    plain.call(TRIAL_WIN, 0);
    assert_eq!(plain.bytes(PARTY + 2, 1)[0], 48, "without the pack the level stays");

    // With the pack: 48 to 50 on the extended rows (BP, EXP, HP and KI of level 50).
    goku_at_48(&mut pack, &rom);
    assert_eq!(pack.bytes(PARTY + 2, 1)[0], 48);
    pack.call(TRIAL_WIN, 0);
    let at = goku.at(50);
    assert_eq!(pack.bytes(PARTY + 2, 1)[0], 50, "the trial gives two levels");
    assert_eq!(pack.triple(PARTY + 0x0F), at.exp, "the EXP of level 50");
    assert_eq!(pack.triple(PARTY + 7), at.bp, "the BP of level 50");
    assert_eq!(u64::from(pack.word(PARTY + 0x0A)), u64::from(at.max_hp));

    // Past 99: from 98 to 100 (the byte saturates at 99, the wide level is 100), and at the cap 998 to 999 only.
    for (from, to) in [(98_u8, 100_u16), (97, 99)] {
        goku_at_48(&mut pack, &rom);
        pack.write(RAISE_CHARACTER, &[0]);
        pack.write(RAISE_LEVELS, &[from - 49]);
        pack.call(0x03_C64A, 0);
        pack.call(TRIAL_WIN, 0);
        let want = goku.at(to);
        assert_eq!(pack.bytes(PARTY + 2, 1)[0], to.min(99) as u8);
        if to > 99 {
            let (level, exp, bp) = pack.wide(0);
            assert_eq!(level, to);
            assert_eq!((exp, bp), (want.exp, want.bp), "level {to}");
        } else {
            assert_eq!((pack.triple(PARTY + 0x0F), pack.triple(PARTY + 7)), (want.exp, want.bp));
        }
    }
}

#[test]
#[ignore = "needs the game's ROM (SSD_ROM_PATH)"]
fn a_trial_exp_that_carries_past_24_bits_goes_to_the_high_byte() {
    let rom = rom();
    let mut pack = Rig::new(&rom, true);
    goku_at_48(&mut pack, &rom);
    // The EXP just under 2^24 and a trial that pays 64 000 (40 clears at the last gravity).
    pack.write(PARTY + 0x0F, &[0x00, 0xFF, 0xFF]);
    pack.write(TRIAL_COUNTERS, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 40]);
    pack.call(TRIAL_EXP_EFFECT, 0);
    let high = pack.bytes(exp_high_at(0), 1)[0];
    assert_eq!(high, 1, "the carry is in the high byte");
    assert_eq!(pack.triple(PARTY + 0x0F), 0xFF_FF00 + 64_000 - 0x100_0000);
    let (level, exp, _) = pack.wide(0);
    assert_eq!(exp, 0x100_0000 + (0xFF_FF00 + 64_000 - 0x100_0000), "the whole EXP");
    assert!(level > 48 || pack.bytes(PARTY + 2, 1)[0] > 48, "the level-up check saw the whole EXP");
}

// ---- The Porunga card ----

/// `05:C8BB`: the card's effect from the target scan (`$0D92`: the party offsets divided by two, `$FF` for none), through its cap and
/// the refusal at `05:C8F3`, to the message at `05:93A9`.
const CARD: u32 = 0x05_C8BB;
const CARD_REFUSED: u32 = 0x05_C8F3;
const CARD_MESSAGE: u32 = 0x05_C8E9;
const CARD_USED: u32 = 0x05_93A9;
const CARD_TARGETS: u32 = 0x7E_0D92;

impl Rig {
    /// Uses the card on party slot `slot`: the number the message is given (the level), or `None` when the card is refused.
    fn use_card(&mut self, slot: u32) -> Option<u32> {
        self.write(CARD_TARGETS, &[(0x10 * slot) as u8, 0xFF, 0xFF]);
        match self.run_from(CARD, false, &[CARD_REFUSED, CARD_MESSAGE]) {
            CARD_MESSAGE => {
                let shown = self.bytes(0x7E_0000, 3);
                let shown =
                    u32::from(shown[0]) | u32::from(shown[1]) << 8 | u32::from(shown[2]) << 16;
                assert_eq!(self.run_from(CARD_MESSAGE, false, &[CARD_USED]), CARD_USED);
                Some(shown)
            }
            CARD_REFUSED => None,
            other => panic!("the card stopped at ${other:06X}"),
        }
    }
}

#[test]
#[ignore = "needs the game's ROM (SSD_ROM_PATH)"]
fn the_porunga_card_goes_past_its_cap_of_47_only_with_the_pack() {
    let rom = rom();
    let goku = anchor(&rom, 0);
    let rows: Vec<_> = (1..=47).map(|l| game_row(&rom, 0, l)).collect();
    let mut plain = Rig::new(&rom, false);
    let mut pack = Rig::new(&rom, true);

    // The original: Goku at 47 is refused.
    plain.reset();
    plain.member(0, 0, 47, rows[46].1, goku.bp, rows[46].2 as u16, goku.ki as u16);
    assert_eq!(plain.use_card(0), None, "the original refuses Goku at 47");
    assert_eq!(plain.bytes(PARTY + 2, 1)[0], 47);
    plain.member(0, 0, 30, rows[29].1, 0, rows[29].2 as u16, 100);
    assert_eq!(plain.use_card(0), Some(31), "a level below the cap is given");
    assert_eq!(plain.bytes(PARTY + 2, 1)[0], 31);
    assert_eq!(plain.triple(PARTY + 7), 0, "the original card gives the level and its EXP only");
    let gohan = anchor(&rom, 1);
    let gohan_rows: Vec<_> = (1..=48).map(|l| game_row(&rom, 1, l)).collect();

    // The pack leaves a level below the cap as it is (the original gives the level and its EXP, no stats).
    pack.reset();
    pack.member(0, 0, 30, rows[29].1, 0, rows[29].2 as u16, 100);
    assert_eq!(pack.use_card(0), Some(31));
    assert_eq!(pack.bytes(PARTY + 2, 1)[0], 31);
    assert_eq!(pack.triple(PARTY + 7), 0, "no BP below the cap, as in the original");

    // With the pack: 47 to 48, 48 to 49 and 98 to 99, on the extended rows.
    for (from, to) in [(47_u8, 48_u16), (48, 49), (98, 99)] {
        if from == 47 {
            pack.reset();
            pack.member(0, 0, 47, rows[46].1, goku.bp, rows[46].2 as u16, goku.ki as u16);
        } else {
            goku_at_48(&mut pack, &rom);
            if from != 48 {
                pack.write(RAISE_LEVELS, &[from - 49]);
                pack.call(0x03_C64A, 0);
            }
        }
        assert_eq!(pack.bytes(PARTY + 2, 1)[0], from, "the member is at {from}");
        assert_eq!(pack.use_card(0), Some(u32::from(to)), "the card is accepted at {from}");
        let want = goku.at(to);
        assert_eq!(pack.bytes(PARTY + 2, 1)[0], to as u8, "level {to}");
        let (exp, bp) = (pack.triple(PARTY + 0x0F), pack.triple(PARTY + 7));
        assert_eq!((exp, bp), (want.exp, want.bp), "EXP and BP of level {to}");
        assert_eq!(u64::from(pack.word(PARTY + 0x0A)), u64::from(want.max_hp), "HP of {to}");
        assert_eq!(u64::from(pack.word(PARTY + 0x0C)), want.ki, "KI of {to}");
        assert_eq!(pack.word(PARTY + 3), pack.word(PARTY + 0x0A), "HP full");
        assert_eq!(pack.bytes(GAINED, 1)[0], 0x80, "a level was gained");
    }

    // Gohan (cap 48 in the game) goes to 49 with the pack; from 99 the byte saturates and the wide level is 100.
    pack.reset();
    pack.member(0, 1, 48, gohan_rows[47].1, gohan.bp, gohan_rows[47].2 as u16, 100);
    assert_eq!(pack.use_card(0), Some(49));
    assert_eq!(pack.bytes(PARTY + 2, 1)[0], 49);
    goku_at_48(&mut pack, &rom);
    pack.write(RAISE_LEVELS, &[99 - 49]);
    pack.call(0x03_C64A, 0);
    assert_eq!(pack.bytes(PARTY + 2, 1)[0], 99);
    assert_eq!(pack.use_card(0), Some(100), "the message shows level 100");
    let (wide, exp, bp) = pack.wide(0);
    let want = goku.at(100);
    assert_eq!((wide, exp, bp), (100, want.exp, want.bp), "level 100");
    assert_eq!(pack.bytes(PARTY + 2, 1)[0], 99, "the byte saturates");

    // At the level cap the card is refused, as at the cap of the original.
    pack.write(wide_level_at(0), &999_u16.to_le_bytes());
    assert_eq!(pack.use_card(0), None, "refused at the cap");
    assert_eq!(pack.word(wide_level_at(0)), 999);
}

// ---- The forms and the story's gifts at a high level ----

/// What a character has at `level` in the game's own growth: the BP and KI as the sums of the rows (the extension past the cap), the
/// maximum HP and the EXP the level begins at.
fn curve(rom: &[u8], id: usize, level: u16) -> (u64, u64, u64, u64) {
    let cap = u16::from(rom[(3 << 15) + (0x850E - 0x8000) + id]);
    if level <= cap {
        let (mut bp, mut ki) = (0, 0);
        for l in 1..=level {
            let row = game_row(rom, id, usize::from(l));
            bp += row.0;
            ki += row.3;
        }
        let row = game_row(rom, id, usize::from(level));
        return (bp, ki, row.2, row.1);
    }
    let point = anchor(rom, id).at(level);
    (point.bp, point.ki, u64::from(point.max_hp), point.exp)
}

impl Rig {
    /// Party slot `slot` is character `id` at `level` on its curve (past 99 the byte saturates and the wide level is kept).
    fn at_level(&mut self, rom: &[u8], slot: u32, id: u8, level: u16) {
        let (bp, ki, hp, exp) = curve(rom, usize::from(id), level);
        self.member(
            slot,
            id,
            level.min(99) as u8,
            exp & 0xFF_FFFF,
            bp & 0xFF_FFFF,
            hp as u16,
            ki as u16,
        );
        if bp >> 24 != 0 {
            self.write(MARKER_AT, &MARKER);
            self.write(BLOCK + PARTY_HIGH as u32 + 2 * slot, &((bp >> 24) as u16).to_le_bytes());
        }
        if level > 99 {
            self.write(LEVELS_MARKER_AT, b"SSDl");
            self.write(wide_level_at(id), &level.to_le_bytes());
            self.write(exp_high_at(id), &[(exp >> 24) as u8]);
        }
    }

    /// The level of party slot `slot`, as the game reads it (the wide level while the byte agrees with it).
    fn level_of(&self, slot: u32, id: u8) -> u16 {
        let byte = u16::from(self.bytes(PARTY + 0x20 * slot + 2, 1)[0]);
        let wide = if self.bytes(LEVELS_MARKER_AT, 4) == b"SSDl" {
            self.word(wide_level_at(id))
        } else {
            0
        };
        if wide > 99 && wide.min(99) == byte { wide } else { byte }
    }

    /// The 40-bit BP of party slot `slot`.
    fn bp_of(&self, slot: u32) -> u64 {
        let live = ssd_game::mods::state::block_live(self.machine.wram());
        let high =
            if live { u64::from(self.word(BLOCK + PARTY_HIGH as u32 + 2 * slot)) } else { 0 };
        self.triple(PARTY + 0x20 * slot + 7) | high << 24
    }

    /// Runs the code at `entry` as a `JSL` at `ret - 3` would (the 24-bit address of its last byte, `ret`, is on the stack), until it
    /// returns to the instruction after it, with `a` in the accumulator. A short stub in work RAM pushes the address and jumps, so that
    /// the routine is entered as the game enters it.
    fn call_from(&mut self, entry: u32, ret: u32, a: u16) {
        let stub = [
            0xA9,
            (ret >> 16) as u8,
            0x48,
            0xF4,
            ret as u8,
            (ret >> 8) as u8,
            0xA9,
            a as u8,
            0x5C,
            entry as u8,
            (entry >> 8) as u8,
            (entry >> 16) as u8,
        ];
        self.write(CALL, &stub);
        let mut registers = self.machine.cpu_registers();
        registers.pbr = (CALL >> 16) as u8;
        registers.pc = (CALL & 0xFFFF) as u16;
        registers.dbr = 0;
        registers.d = 0;
        registers.p = StatusRegister::from(0x24);
        self.machine.set_cpu_registers(registers);
        for stale in self.machine.breakpoints().clone() {
            self.machine.clear_breakpoint(stale);
        }
        self.machine.set_breakpoint(ret + 1);
        let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
        for _ in 0..12 {
            let stopped =
                self.machine.run(&stops, &mut |_| Joypad::default(), &mut NullAudio, 1).unwrap();
            if matches!(stopped, Stopped::Breakpoint { .. }) {
                return;
            }
        }
        panic!("${entry:06X} did not return to ${ret:06X}");
    }

    /// The builder of a form for character `id`, as `03:D1B4` and `03:8683` call it.
    fn build_form(&mut self, id: u8) {
        self.call_from(0x03_85F0, 0x7F_FE03, u16::from(id));
    }

    /// The level routine of the story run as the scene engine runs the effect `$38` in `scene` (`$164D` levels for character `id`).
    fn gift(&mut self, scene: u8, ret: u32, id: u8, levels: u8) {
        self.write(0x7E_01D8, &[scene]);
        self.write(RAISE_CHARACTER, &[id]);
        self.write(RAISE_LEVELS, &[levels]);
        self.call_from(0x03_C64A, ret, 0);
    }
}

/// `(L + 5) / 52` to the power `power` of `value`, rounded (halves up): the form's rule.
fn form_scaled(value: u64, level: u16, power: u32) -> u64 {
    let (u, u0) = (u128::from(level.max(47)) + 5, 52_u128);
    ((u128::from(value) * u.pow(power) + u0.pow(power) / 2) / u0.pow(power)) as u64
}

#[test]
#[ignore = "needs the game's ROM (SSD_ROM_PATH)"]
fn a_form_keeps_the_level_of_the_character_and_scales_with_it() {
    let rom = rom();
    let mut plain = Rig::new(&rom, false);
    let mut pack = Rig::new(&rom, true);

    // Piccolo's fusion with Dende (effect `$66` builds the form of character 2) at level 40: the original's form, level 48, BP 1 900 000.
    for rig in [&mut plain, &mut pack] {
        rig.reset();
        rig.at_level(&rom, 0, 2, 40);
        rig.build_form(2);
        assert_eq!(rig.bytes(PARTY + 2, 1)[0], 48, "the builder sets 48");
        assert_eq!(rig.triple(PARTY + 7), 1_900_000, "the row's BP");
        assert_eq!(rig.bytes(PARTY + 1, 1)[0], 0x3E, "unit $3E");
    }
    assert_eq!(
        plain.bytes(PARTY + 0x0A, 2),
        pack.bytes(PARTY + 0x0A, 2),
        "the same maximum HP below 48"
    );
    assert!(
        pack.word(PARTY + 0x0C) >= plain.word(PARTY + 0x0C),
        "the KI is not lowered by the form"
    );

    // At 80: the original falls back to 48; the pack keeps 80 and the form follows the level, none of it below what he had.
    plain.reset();
    plain.at_level(&rom, 0, 2, 80);
    plain.build_form(2);
    assert_eq!(plain.bytes(PARTY + 2, 1)[0], 48, "without the pack he falls back to 48");
    pack.reset();
    pack.at_level(&rom, 0, 2, 80);
    let (own_bp, own_ki, own_hp, _) = curve(&rom, 2, 80);
    pack.build_form(2);
    assert_eq!(pack.level_of(0, 2), 80, "the level is kept");
    assert_eq!(pack.bytes(PARTY + 1, 1)[0], 0x3E);
    let bp = pack.bp_of(0);
    assert_eq!(bp, form_scaled(1_900_000, 80, 3).max(own_bp), "BP of the form at 80");
    assert!(bp > own_bp);
    assert_eq!(u64::from(pack.word(PARTY + 0x0A)), form_scaled(710, 80, 1).max(own_hp));
    assert_eq!(
        u64::from(pack.word(PARTY + 0x0C)),
        form_scaled(124, 80, 1).max(own_ki),
        "KI is not lowered"
    );
    assert_eq!(pack.word(PARTY + 3), pack.word(PARTY + 0x0A), "HP full");
    println!("Piccolo's form at 80: BP {bp} (his own {own_bp})");

    // Above 99 the byte saturates, the wide level is kept and the BP's high part is the form's.
    pack.reset();
    pack.at_level(&rom, 0, 2, 150);
    pack.build_form(2);
    assert_eq!(pack.level_of(0, 2), 150);
    assert_eq!(pack.bytes(PARTY + 2, 1)[0], 99);
    assert_eq!(pack.bp_of(0), form_scaled(1_900_000, 150, 3).max(curve(&rom, 2, 150).0));

    // Goku's Super Saiyan at 70 (between 48 and 99): the form follows his level (before, it was the fixed 4 999 999 at 48).
    for (rig, scaled) in [(&mut plain, false), (&mut pack, true)] {
        rig.reset();
        rig.at_level(&rom, 0, 0, 70);
        rig.build_form(0);
        let level = rig.level_of(0, 0);
        if scaled {
            assert_eq!(level, 70);
            assert_eq!(rig.bp_of(0), form_scaled(4_999_999, 70, 3));
            assert_eq!(u64::from(rig.word(PARTY + 0x0A)), form_scaled(800, 70, 1));
            assert_eq!(u64::from(rig.word(PARTY + 0x0C)), form_scaled(189, 70, 1));
        } else {
            assert_eq!((level, rig.bp_of(0)), (48, 4_999_999), "the original's form");
        }
    }

    // Vegeta and the other characters have no form here: the builder is the original's.
    plain.reset();
    pack.reset();
    for rig in [&mut plain, &mut pack] {
        rig.at_level(&rom, 0, 7, 60);
        rig.build_form(7);
    }
    assert_eq!(plain.bytes(PARTY, 0x20), pack.bytes(PARTY, 0x20));
}

#[test]
#[ignore = "needs the game's ROM (SSD_ROM_PATH)"]
fn the_gifts_of_the_story_are_worth_the_same_at_every_level() {
    const EFFECT: u32 = 0x03_B1C9;
    const GURU: u32 = 0x03_CFE4;
    const FLOOR: u32 = 0x03_EAFB;
    let rom = rom();
    let mut pack = Rig::new(&rom, true);

    // Nail's fusion (scene $37, +5: six levels in the original) at Piccolo's 35: six levels, the original's.
    pack.reset();
    pack.at_level(&rom, 0, 2, 35);
    pack.gift(0x37, EFFECT, 2, 5);
    assert_eq!(pack.level_of(0, 2), 41);
    let (bp, ..) = curve(&rom, 2, 41);
    assert_eq!(pack.bp_of(0), bp, "the game's own rows");

    // At 100 the gift is 16 levels (6 * 105 / 40 = 15.75), on the extended rows.
    pack.reset();
    pack.at_level(&rom, 0, 2, 100);
    pack.gift(0x37, EFFECT, 2, 5);
    assert_eq!(pack.level_of(0, 2), 116, "six levels at 35 are sixteen at 100");
    assert_eq!(pack.bp_of(0), curve(&rom, 2, 116).0);
    // The BP rose by the same ratio as the original's gift at 35: (46 / 40)^3 = 1.52.
    let ratio = pack.bp_of(0) as f64 / curve(&rom, 2, 100).0 as f64;
    let story = (46.0_f64 / 40.0).powi(3);
    assert!((ratio - story).abs() / story < 0.02, "{ratio} against {story}");

    // The gift of the scene $3C (Dende, +4 then +1) and the other scenes' effect $38 (Gohan's +3 in scene 4, the Snake Way's +20
    // in scene $4C) and any floor are the original's rule.
    pack.reset();
    pack.at_level(&rom, 0, 2, 100);
    pack.gift(0x04, EFFECT, 2, 3);
    assert_eq!(pack.level_of(0, 2), 104, "scene 4 is the original's +3");
    pack.reset();
    pack.at_level(&rom, 0, 2, 100);
    pack.gift(0x37, FLOOR, 2, 5);
    assert_eq!(pack.level_of(0, 2), 106, "a floor's call is the original's");
    pack.reset();
    pack.at_level(&rom, 0, 2, 100);
    pack.gift(0x3C, EFFECT, 2, 4);
    assert_eq!(pack.level_of(0, 2), 111, "Dende's +4 is 5 levels at 41: 5 * 105 / 46 = 11.4");

    // Guru's unlocking (effect $5B, +2: three levels) at Gohan's 100 is 8 levels; at 30 it is three.
    pack.reset();
    pack.at_level(&rom, 0, 1, 100);
    pack.gift(0x2D, GURU, 1, 2);
    assert_eq!(pack.level_of(0, 1), 108);
    pack.reset();
    pack.at_level(&rom, 0, 1, 30);
    pack.gift(0x2D, GURU, 1, 2);
    assert_eq!(pack.level_of(0, 1), 33);

    // At the level cap nothing more is given, and a character with no record is left as the original leaves him.
    pack.reset();
    pack.at_level(&rom, 0, 2, 999);
    pack.gift(0x37, EFFECT, 2, 5);
    assert_eq!(pack.level_of(0, 2), 999);
    pack.gift(0x37, EFFECT, 5, 5);
}

#[test]
#[ignore = "needs the game's ROM (SSD_ROM_PATH)"]
fn goku_after_the_pod_and_in_the_last_fight_joins_the_party_at_its_level() {
    const POD: u32 = 0x03_FA85;
    let rom = rom();
    let mut plain = Rig::new(&rom, false);
    let mut pack = Rig::new(&rom, true);

    // The party: Gohan 80, Krillin 70, Piccolo 60; Goku 40. The mean of the three highest is 70, Goku arrives at 77.
    let party = |rig: &mut Rig, goku: u16| {
        rig.reset();
        rig.at_level(&rom, 0, 0, goku);
        rig.at_level(&rom, 1, 1, 80);
        rig.at_level(&rom, 2, 3, 70);
        rig.at_level(&rom, 3, 2, 60);
        rig.write(RAISE_CHARACTER, &[0]);
    };
    party(&mut plain, 40);
    plain.write(0x7E_01D8, &[0x3F]);
    plain.write(RAISE_LEVELS, &[49 - 40]);
    plain.call_from(0x03_C64A, POD, 0);
    assert_eq!(plain.level_of(0, 0), 47, "the original raises him to 49, which stops at his cap");

    party(&mut pack, 40);
    pack.write(0x7E_01D8, &[0x3F]);
    pack.write(RAISE_LEVELS, &[49 - 40]);
    pack.call_from(0x03_C64A, POD, 0);
    assert_eq!(pack.level_of(0, 0), 77, "1.1 * 70");
    assert_eq!(pack.bp_of(0), curve(&rom, 0, 77).0, "his rows");
    assert_eq!(u64::from(pack.word(PARTY + 0x0A)), curve(&rom, 0, 77).2);
    assert_eq!(pack.word(PARTY + 3), pack.word(PARTY + 0x0A), "HP full");

    // Never lowered: a Goku of 90 stays at 90.
    party(&mut pack, 90);
    pack.write(0x7E_01D8, &[0x3F]);
    pack.write(RAISE_LEVELS, &[49_u8.wrapping_sub(90)]);
    pack.call_from(0x03_C64A, POD, 0);
    assert_eq!(pack.level_of(0, 0), 90);

    // Goku is not in the party at the pod: nothing happens (the original finds no record either).
    party(&mut pack, 40);
    pack.write(PARTY, &[0; 0x20]);
    pack.write(0x7E_01D8, &[0x3F]);
    pack.write(RAISE_LEVELS, &[9]);
    pack.call_from(0x03_C64A, POD, 0);
    assert_eq!(pack.bytes(PARTY, 0x20), [0; 0x20]);

    // The last fight's load (effect $3C, to `03:C845`): Goku was built from the roster at his old level and joins at 77.
    party(&mut pack, 40);
    pack.call(0x03_C845, 0);
    assert_eq!(pack.level_of(0, 0), 77);
    party(&mut plain, 40);
    plain.call(0x03_C845, 0);
    assert_eq!(plain.level_of(0, 0), 40, "without the pack the load leaves him as he was");
}

#[test]
#[ignore = "needs the game's ROM (SSD_ROM_PATH)"]
fn the_last_enemy_is_made_for_the_party_that_goku_joins() {
    let rom = rom();
    let mut pack = Rig::new(&rom, true);
    // The enemy of the last fight is built before Goku joins, from the roster: party Gohan, Krillin and Piccolo at 60, roster
    // Goku at 30. The offsets of the pack's scaling give the story's enemies P + 3: P is the mean of 66 (Goku), 60 and 60.
    let level_of_enemy = |pack: &mut Rig, goku_in_roster: bool| -> u8 {
        pack.reset();
        pack.at_level(&rom, 0, 1, 60);
        pack.at_level(&rom, 1, 3, 60);
        pack.at_level(&rom, 2, 2, 60);
        if goku_in_roster {
            let (bp, ki, hp, exp) = curve(&rom, 0, 30);
            let mut record = [0_u8; 0x20];
            record[0] = 0xC0;
            record[1] = 0;
            record[2] = 30;
            record[0x1C] = 0;
            record[0x0A..0x0C].copy_from_slice(&(hp as u16).to_le_bytes());
            record[0x0C..0x0E].copy_from_slice(&(ki as u16).to_le_bytes());
            record[7..10].copy_from_slice(&(bp as u32).to_le_bytes()[..3]);
            record[0x0F..0x12].copy_from_slice(&(exp as u32).to_le_bytes()[..3]);
            pack.write(0x7E_2800, &record);
        } else {
            pack.write(0x7E_2800, &[0; 0x20]);
        }
        pack.write(GROUP_INDEX, &[0x56]);
        pack.call(0x01_F692, 0);
        pack.call(0x01_F719, 0);
        let slot = (0..5_u32)
            .find(|&slot| pack.bytes(ENEMIES + 0x20 * slot, 1)[0] != 0)
            .expect("an enemy of the last fight");
        pack.bytes(ENEMIES + 0x20 * slot + 2, 1)[0]
    };
    let with_goku = level_of_enemy(&mut pack, true);
    let without = level_of_enemy(&mut pack, false);
    println!("last fight's enemy: {with_goku} with Goku in the roster, {without} without");
    assert_eq!(without, 63, "the party's 60 and 3 above");
    assert_eq!(with_goku, 65, "Goku arrives at 66: the mean of 66, 60 and 60 is 62");
}

// ---- Porunga's level wish and Guru's boost from the Item screen, with the level cap ----

impl Rig {
    /// A machine on the image of both example packs, the level cap's and the party-wide boosts'.
    fn both(rom: &[u8]) -> Self {
        let saves = Box::new(MemorySaveStore::default());
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let packs = [
            ssd_mod::Pack::open(&root.join("free-progression")).unwrap(),
            ssd_mod::Pack::open(&root.join("party-boosts")).unwrap(),
        ];
        let options = ssd_mod::CheckOptions { allow_rom_bytes: false };
        ssd_mod::check_set(rom, &packs, options).expect("the packs are accepted");
        let image = ssd_mod::build(rom, &packs).unwrap().bytes;
        let mut machine =
            Machine::with_image(image, ssd_rom::crc32(rom), saves, DEFAULT_WRAM_SEED).unwrap();
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

    /// The commit of Porunga's level wish on the member at party offset `offset` (step 7 of the Item effect, after the A button).
    fn wish(&mut self, offset: u16) {
        self.write(0x7E_0748, &offset.to_le_bytes());
        self.write(0x7E_073C, &[7]);
        self.write(0x7E_073B, &[0xFF]);
        self.write(0x7E_0C36, &[0]);
        self.call_from(0x01_E3FC, 0x7F_FE03, 0);
    }

    /// The first step of the BP objects (`item` 5 or 6) on the member at party offset `offset`.
    fn boost(&mut self, item: u8, offset: u16) {
        self.write(0x7E_0748, &offset.to_le_bytes());
        self.write(0x7E_0D7C, &[item]);
        self.write(0x7E_0D62, &[0]);
        self.write(0x7E_073C, &[0]);
        self.call_from(0x01_DF13, 0x7F_FE03, 0);
    }
}

#[test]
#[ignore = "needs the game's ROM (SSD_ROM_PATH)"]
fn porunga_wish_goes_past_the_games_cap_with_the_level_cap() {
    let rom = rom();
    let mut plain = Rig::new(&rom, false);
    let mut pack = Rig::new(&rom, true);

    // The original refuses Goku at 47 (his cap): the level and the step stay.
    plain.reset();
    plain.at_level(&rom, 0, 0, 47);
    plain.wish(0);
    assert_eq!(plain.level_of(0, 0), 47, "the original refuses");
    assert_eq!(plain.bytes(0x7E_073C, 1)[0], 7, "the step stays");

    // With the cap the wish gives level 48 on the extended rows: level, EXP, BP; the targets of the growth animation.
    pack.reset();
    pack.at_level(&rom, 0, 0, 47);
    pack.write(0x7E_4C00, &[0; 3]);
    pack.wish(0);
    let (bp48, ki48, hp48, exp48) = curve(&rom, 0, 48);
    assert_eq!(pack.level_of(0, 0), 48);
    assert_eq!(pack.triple(PARTY + 0x0F), exp48, "the EXP the level begins at");
    assert_eq!(pack.bp_of(0), bp48, "the level's BP");
    assert_eq!(u64::from(pack.word(0x7E_0740)), hp48, "the maximum HP the animation grows to");
    assert_eq!(u64::from(pack.word(0x7E_0742)), ki48, "the maximum KI the animation grows to");
    assert_eq!(pack.bytes(0x7E_073C, 1)[0], 8, "the step goes on to the animation");
    assert_eq!(pack.bytes(0x7E_073B, 1)[0], 0xBF, "the pending bit is cleared");
    let (hp47, ki47) = (curve(&rom, 0, 47).2, curve(&rom, 0, 47).1);
    assert_eq!(
        (u64::from(pack.word(PARTY + 0x0A)), u64::from(pack.word(PARTY + 0x0C))),
        (hp47, ki47),
        "the maxima are left to the animation"
    );
    // The BP snapshot (which a battle's end restores the record from) has the level's BP too.
    assert_eq!(
        pack.triple(0x7E_4C00),
        bp48 - curve(&rom, 0, 47).0,
        "the snapshot got the level's BP"
    );

    // Past 99, and from level 98 to 99, the wide level; and a member of the original's range goes by the original's own code.
    pack.reset();
    pack.at_level(&rom, 0, 0, 99);
    pack.wish(0);
    assert_eq!(pack.level_of(0, 0), 100);
    assert_eq!(pack.bp_of(0), curve(&rom, 0, 100).0);
    pack.reset();
    pack.at_level(&rom, 0, 0, 30);
    plain.reset();
    plain.at_level(&rom, 0, 0, 30);
    pack.wish(0);
    plain.wish(0);
    assert_eq!(
        pack.bytes(PARTY, 0x20),
        plain.bytes(PARTY, 0x20),
        "below the cap the original runs"
    );
    assert_eq!(pack.level_of(0, 0), 31);

    // The original's other tests stay: Goku with the Kaioken on, a member who has his flags' bit 5, and the level cap.
    pack.reset();
    pack.at_level(&rom, 0, 0, 47);
    pack.write(0x7E_0748, &0_u16.to_le_bytes());
    pack.write(0x7E_0C36, &[0x80]);
    pack.write(0x7E_073C, &[7]);
    pack.call_from(0x01_E3FC, 0x7F_FE03, 0);
    assert_eq!(pack.level_of(0, 0), 47, "Goku with the Kaioken on is refused");
    pack.at_level(&rom, 0, 0, 47);
    pack.write(PARTY, &[0xE0]);
    pack.wish(0);
    assert_eq!(pack.level_of(0, 0), 47, "a member with flags bit 5 is refused");
    pack.reset();
    pack.at_level(&rom, 0, 0, 999);
    pack.wish(0);
    assert_eq!(pack.level_of(0, 0), 999, "refused at the level cap");
    assert_eq!(pack.bytes(0x7E_073C, 1)[0], 7);
}

#[test]
#[ignore = "needs the game's ROM (SSD_ROM_PATH)"]
fn the_party_wide_wish_and_guru_know_the_level_cap() {
    let rom = rom();
    let mut both = Rig::both(&rom);

    // The wish for a party: Goku (47, the chosen one), Gohan (48) and Krillin (60) each go up one level.
    both.reset();
    both.at_level(&rom, 0, 0, 47);
    both.at_level(&rom, 1, 1, 48);
    both.at_level(&rom, 2, 3, 60);
    both.write(0x7E_01BB, &[3]);
    both.wish(0);
    assert_eq!(
        [both.level_of(0, 0), both.level_of(1, 1), both.level_of(2, 3)],
        [48, 49, 61],
        "the whole party"
    );
    for (slot, id, level) in [(0_u32, 0_u8, 48_u16), (1, 1, 49), (2, 3, 61)] {
        let (bp, ki, hp, exp) = curve(&rom, usize::from(id), level);
        assert_eq!(both.bp_of(slot), bp, "BP of {id}");
        assert_eq!(both.triple(PARTY + 0x20 * slot + 0x0F), exp & 0xFF_FFFF, "EXP of {id}");
        if slot != 0 {
            assert_eq!(u64::from(both.word(PARTY + 0x20 * slot + 0x0A)), hp, "max HP of {id}");
            assert_eq!(u64::from(both.word(PARTY + 0x20 * slot + 0x0C)), ki, "max KI of {id}");
        }
    }
    assert_eq!(both.bytes(0x7E_073C, 1)[0], 8);

    // The wish for a party whose chosen member is below his cap: the original's code for him, the extension for the others.
    both.reset();
    both.at_level(&rom, 0, 0, 30);
    both.at_level(&rom, 1, 1, 48);
    both.write(0x7E_01BB, &[2]);
    both.wish(0);
    assert_eq!([both.level_of(0, 0), both.level_of(1, 1)], [31, 49]);

    // Guru's boost (+50 percent) on a member whose BP is wide: the whole BP, in 40 bits (the original's step cuts it at 9 999 999).
    both.reset();
    both.at_level(&rom, 0, 0, 300);
    both.at_level(&rom, 1, 1, 200);
    both.at_level(&rom, 2, 2, 100);
    both.write(0x7E_01BB, &[3]);
    let before: Vec<u64> = (0..3).map(|slot| both.bp_of(slot)).collect();
    assert!(before[0] > 50_000_000, "Goku's BP at 300 is wide: {}", before[0]);
    both.boost(6, 0);
    assert_eq!(
        both.bp_of(0),
        before[0] + before[0] / 2,
        "the chosen member's BP: +50 percent whole"
    );
    assert_eq!(
        both.bytes(0x7E_0744, 3)[..],
        both.bp_of(0).to_le_bytes()[..3],
        "the animation has nothing left to do"
    );
    for slot in 1..3 {
        assert_eq!(
            both.bp_of(slot),
            before[slot as usize] + before[slot as usize] / 2,
            "member {slot}"
        );
    }
    // King Kai's object adds a quarter.
    both.reset();
    both.at_level(&rom, 0, 0, 300);
    both.boost(5, 0);
    assert_eq!(both.bp_of(0), before[0] + before[0] / 4);
    // Below it, the original's own rule (the animation ends at BP + BP / 2; the cut at 9 999 999 is the original's).
    both.reset();
    both.at_level(&rom, 0, 0, 100);
    both.boost(6, 0);
    let own = curve(&rom, 0, 100).0;
    assert_eq!(
        both.bytes(0x7E_0744, 3)[..],
        (own + own / 2).to_le_bytes()[..3],
        "the original's target"
    );
    assert_eq!(both.bp_of(0), own, "the record is moved by the count-up of the animation");
}
