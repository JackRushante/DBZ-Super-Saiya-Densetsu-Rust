//! The Game Over screen against the real game: the original loops there forever and no button leaves it (a console's
//! Reset was the way out); with the story fix of Remastered (`cheats::GAME_OVER_LOOP`) a button returns to the title
//! through the game's own code after the screen, and the title starts a game again.
//!
//! The lost battle is the reference battle with the enemies made far stronger and the party at 1 HP (host writes).
//!
//! Needs the game's ROM:
//! `SSD_ROM_PATH=... cargo test -p ssd-game --release --test game_over_real_rom -- --ignored --nocapture`.

use ssd_game::cheats::{self, GAME_OVER_LOOP};
use ssd_game::scripts::battle_input;
use ssd_oracle::{HookTable, Joypad, Machine, MemorySaveStore, NullAudio};

/// The main loop's state (`game_mode`, `$24`): `$0D` is the Game Over screen, `$87` the title as the boot enters it.
const GAME_MODE: usize = 0x24;
const GAME_OVER: u8 = 0x0D;
const TITLE: u8 = 0x87;
/// The mode the title gives a new game: the first story transition (`docs/re/data/title-chapters.md`).
const NEW_GAME: u8 = 0x07;
/// The frame the battle of the reference script is under way, and the records the host writes change there: the HP of
/// the four party records (`$0B00`, `+03`) and the BP of the five enemy slots (`$0E00`, `+07`).
const LOSE_AT: u64 = 4_800;
const PARTY_RECORDS: u32 = 0x7E_0B00;
const ENEMY_SLOTS: u32 = 0x7E_0E00;

fn game(story_fix: bool) -> Machine {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let bytes = std::fs::read(path).unwrap();
    let rom = ssd_rom::normalize(&bytes, None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32, "not the canonical ROM");
    let mut machine = Machine::new(rom.bytes, Box::new(MemorySaveStore::default())).unwrap();
    if story_fix {
        let mut table = HookTable::new();
        cheats::install(&mut table);
        machine.set_hook_table(Some(table));
    }
    machine
}

fn step(machine: &mut Machine, pad: Joypad) {
    machine.run_frame(pad, &mut NullAudio).unwrap();
}

fn mode(machine: &Machine) -> u8 {
    machine.wram()[GAME_MODE]
}

/// Plays the reference battle and loses it, then waits on the Game Over screen until its last loop runs (with the fix
/// installed, the hook's calls count it; without, a fixed 900 frames past the start of the screen).
fn to_game_over(story_fix: bool) -> Machine {
    let mut machine = game(story_fix);
    while machine.frame_count() < LOSE_AT {
        let pad = battle_input(machine.frame_count());
        step(&mut machine, pad);
    }
    for record in 0..4 {
        machine.write_wram(PARTY_RECORDS + record * 0x20 + 3, &[1, 0]).unwrap();
    }
    for slot in 0..5 {
        machine.write_wram(ENEMY_SLOTS + slot * 0x20 + 7, &[0x40, 0x42, 0x0F]).unwrap();
    }
    while mode(&machine) != GAME_OVER {
        assert!(machine.frame_count() < 8_000, "the battle is lost by frame 8000");
        let pad = battle_input(machine.frame_count());
        step(&mut machine, pad);
    }
    let shown_at = machine.frame_count();
    println!("Game Over at frame {shown_at}");
    loop {
        let waited = machine.frame_count() - shown_at;
        let in_loop =
            if story_fix { machine.hook_stat(GAME_OVER_LOOP).calls > 0 } else { waited >= 900 };
        if in_loop {
            break;
        }
        assert!(waited < 1_200, "the screen reaches its last loop within 1200 frames");
        step(&mut machine, Joypad::default());
    }
    println!("last loop at frame {}", machine.frame_count());
    machine
}

/// A press of every button in turn, each held for 10 frames and let go for 10.
fn every_button(machine: &mut Machine) {
    let presses: [fn(&mut Joypad); 12] = [
        |p| p.a = true,
        |p| p.b = true,
        |p| p.x = true,
        |p| p.y = true,
        |p| p.l = true,
        |p| p.r = true,
        |p| p.start = true,
        |p| p.select = true,
        |p| p.up = true,
        |p| p.down = true,
        |p| p.left = true,
        |p| p.right = true,
    ];
    for press in presses {
        let mut pad = Joypad::default();
        press(&mut pad);
        for _ in 0..10 {
            step(machine, pad);
        }
        for _ in 0..10 {
            step(machine, Joypad::default());
        }
    }
}

/// The original: no button leaves the Game Over screen.
#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn the_original_game_over_screen_never_ends() {
    let mut machine = to_game_over(false);
    every_button(&mut machine);
    for _ in 0..300 {
        step(&mut machine, Joypad::default());
    }
    assert_eq!(mode(&machine), GAME_OVER, "still on the Game Over screen");
}

/// The story fix: the screen stays without a press and with the d-pad or the shoulders; A goes to the title, and the
/// title starts a new game.
#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn a_button_on_the_game_over_screen_goes_back_to_the_title() {
    let mut machine = to_game_over(true);
    for _ in 0..300 {
        step(&mut machine, Joypad::default());
    }
    for press in [Joypad { up: true, ..Joypad::default() }, Joypad { l: true, ..Joypad::default() }]
    {
        for _ in 0..10 {
            step(&mut machine, press);
        }
    }
    assert_eq!(mode(&machine), GAME_OVER, "waits for a button, as the original");

    // Each of the buttons that leave does it, on its own
    let exits: [Joypad; 6] = [
        Joypad { a: true, ..Joypad::default() },
        Joypad { b: true, ..Joypad::default() },
        Joypad { x: true, ..Joypad::default() },
        Joypad { y: true, ..Joypad::default() },
        Joypad { start: true, ..Joypad::default() },
        Joypad { select: true, ..Joypad::default() },
    ];
    for (n, exit) in exits.into_iter().enumerate() {
        let mut machine = machine.snapshot();
        step(&mut machine, exit);
        let mut seen_title = false;
        for _ in 0..600 {
            step(&mut machine, Joypad::default());
            seen_title |= mode(&machine) == TITLE;
        }
        assert!(seen_title, "button {n}: the boot enters the title");
        assert_ne!(mode(&machine), GAME_OVER, "button {n}: the Game Over screen is left");
    }

    // The title after it works: Start (a new game: the cartridge has no save) starts the story
    step(&mut machine, exits[0]);
    let left = machine.frame_count();
    let mut started = false;
    while machine.frame_count() < left + 3_000 && !started {
        let since = machine.frame_count() - left;
        let pad = Joypad { start: since % 64 < 4 && since > 600, ..Joypad::default() };
        step(&mut machine, pad);
        started = mode(&machine) == NEW_GAME;
    }
    println!("the title led on at frame {} (mode {:#04x})", machine.frame_count(), mode(&machine));
    assert!(started, "Start on the title starts the game");
}
