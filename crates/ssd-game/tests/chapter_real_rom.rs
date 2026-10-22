//! The ending wrappers of the packs (M10, Task 8) against the real game: each ending's mode init writes the completion marker,
//! and the frame reports it as the application event.
//!
//! Needs the game's ROM:
//! `SSD_ROM_PATH=... cargo test -p ssd-game --release --test chapter_real_rom -- --ignored --nocapture`.

use ssd_game::mods::chapter::{
    COMPLETED_TAG, ENDING_11, ENDING_12, MARKER, MARKER_AT, completion_events,
};
use ssd_oracle::{
    HookTable, HostEvent, Joypad, Machine, MemorySaveStore, NullAudio, StatusRegister, Stops,
};

/// Where the test stages its call: the upper work RAM, which the game's boot does not clear.
const CALL: u32 = 0x7F_FE00;

fn game() -> Machine {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let bytes = std::fs::read(path).unwrap();
    let rom = ssd_rom::normalize(&bytes, None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32, "not the canonical ROM");
    Machine::new(rom.bytes, Box::new(MemorySaveStore::default())).unwrap()
}

/// Boots the game (with the redirectors when `install`), then calls `entry` (a `JSL` staged at [`CALL`]) and runs one frame.
/// The wrapper runs at the entry, before the original init, so one frame shows its marker; the original's credits go on for
/// thousands of frames after it, which the test does not run.
fn call_ending(entry: u32, install: bool) -> Machine {
    let mut machine = game();
    let mut table = HookTable::new();
    if install {
        ssd_game::mods::install(&mut table);
    }
    machine.set_hook_table(Some(table));
    for _ in 0..600 {
        machine.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    }
    let jsl = [0x22, entry as u8, (entry >> 8) as u8, (entry >> 16) as u8];
    machine.write_wram(CALL, &jsl).unwrap();
    let mut registers = machine.cpu_registers();
    registers.pbr = (CALL >> 16) as u8;
    registers.pc = (CALL & 0xFFFF) as u16;
    registers.dbr = 0;
    registers.d = 0;
    registers.p = StatusRegister::from(0x24);
    machine.set_cpu_registers(registers);
    let stops = Stops { frame_end: true, ..Stops::default() };
    let input = &mut |_frame: u64| Joypad::default();
    machine.run(&stops, input, &mut NullAudio, 1).unwrap();
    machine
}

/// The marker's bytes of a machine's work RAM.
fn marker_of(machine: &Machine) -> &[u8] {
    let at = (MARKER_AT - 0x7E_0000) as usize;
    &machine.wram()[at..at + MARKER.len()]
}

/// Each ending's init writes the marker, and the frame reports the completion once; without the wrapper there is no marker.
#[test]
#[ignore = "needs the ROM (SSD_ROM_PATH)"]
fn each_ending_init_marks_the_game_completed_and_the_frame_reports_it() {
    for entry in [ENDING_11, ENDING_12] {
        let without = call_ending(entry, false);
        assert_ne!(
            marker_of(&without),
            &MARKER[..],
            "without the wrapper, no marker (${entry:06X})"
        );
        let machine = call_ending(entry, true);
        assert_eq!(marker_of(&machine), &MARKER[..], "the wrapper of ${entry:06X} sets the marker");
        let events = completion_events(machine.wram());
        assert!(
            events.contains(&HostEvent::App { tag: COMPLETED_TAG, payload: Vec::new() }),
            "${entry:06X}: {events:?}"
        );
    }
}
