//! Replays: the file format and its checks, recording, playback, and the frame-indexed input that
//! keeps a hooked playback on the recorded input of every frame.

mod common;

use common::{
    JOYPAD_LOG_COUNT, JOYPAD_LOG_HIGH, JOYPAD_LOG_LONG, JOYPAD_LOG_LOW, JOYPAD_LOG_SHORT,
    joypad_log_rom, red_backdrop_rom, sram_counter_rom,
};
use ssd_oracle::{
    CORE_VERSION, Ctx, DEFAULT_WRAM_SEED, HookAction, HookEntry, HookMode, HookStats, HookTable,
    Joypad, MODE_ENHANCED, Machine, MemorySaveStore, NullAudio, ORACLE_REVISION, OracleError,
    Recorder, Replay, ReturnKind, RoutineFn, joypad_bits, joypad_from_bits,
};

/// Master cycles of the instructions of the fixture routines (`SlowROM`).
const INC_DP: u64 = 38;
const RTS: u64 = 42;
const LDX_IMM16: u64 = 24;
const DEX: u64 = 14;
const BNE_TAKEN: u64 = 22;
const BNE_NOT_TAKEN: u64 = 16;
/// The long routine: `INC $11; LDX #$0100`, 256 `DEX; BNE`, `RTS`.
const LONG_COST: u64 = INC_DP + LDX_IMM16 + 256 * DEX + 255 * BNE_TAKEN + BNE_NOT_TAKEN + RTS;
/// The short routine: `INC $10; RTS`.
const SHORT_COST: u64 = INC_DP + RTS;

/// Frames of the playback tests: more than a hundred NMIs logged.
const FRAMES: u64 = 120;

fn machine_with(rom: Vec<u8>) -> Machine {
    Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap()
}

/// A different joypad on every frame, every button used.
fn varied_input(frame: u64) -> Joypad {
    joypad_from_bits((frame as u16).wrapping_mul(0x9E37) & 0x0FFF)
}

fn replay_error<T>(result: Result<T, OracleError>) -> String {
    match result {
        Err(OracleError::Replay(message)) => message,
        Err(other) => panic!("expected a replay error, got {other:?}"),
        Ok(_) => panic!("expected a replay error, got Ok"),
    }
}

/// A replay with SRAM and a few varied inputs.
fn sample() -> Replay {
    Replay {
        rom_crc32: 0x1234_5678,
        wram_seed: 0xDEAD_BEEF_0123_4567,
        initial_sram: Some((0..=255).collect()),
        mode: 0,
        events: Vec::new(),
        packs: None,
        inputs: (0..40).map(varied_input).collect(),
    }
}

/// `bytes` with its CRC fixed, so a check after the CRC's sees the change.
fn with_fixed_crc(mut bytes: Vec<u8>) -> Vec<u8> {
    let crc = ssd_rom::crc32(&bytes[16..]);
    bytes[12..16].copy_from_slice(&crc.to_le_bytes());
    bytes
}

// The file format

#[test]
fn a_replay_round_trips() {
    for replay in [sample(), Replay { initial_sram: None, inputs: Vec::new(), ..sample() }] {
        let bytes = replay.to_bytes(CORE_VERSION);
        assert_eq!(&bytes[..8], b"SSDRPLAY");
        assert_eq!(bytes[8..12], 1_u32.to_le_bytes(), "format 1");
        let read = Replay::from_bytes(&bytes, replay.rom_crc32, CORE_VERSION).unwrap();
        assert_eq!(read, replay);
    }
}

#[test]
fn the_layout_is_the_documented_one() {
    let replay = Replay {
        rom_crc32: 0x0A0B_0C0D,
        wram_seed: 0x1122_3344_5566_7788,
        initial_sram: Some(vec![0xAA, 0xBB]),
        mode: 0,
        events: Vec::new(),
        packs: None,
        inputs: vec![
            Joypad { b: true, ..Joypad::default() },
            Joypad { r: true, ..Joypad::default() },
        ],
    };
    let bytes = replay.to_bytes("core");
    let mut expected = Vec::new();
    expected.extend_from_slice(&0x0A0B_0C0D_u32.to_le_bytes());
    expected.extend_from_slice(&0x1122_3344_5566_7788_u64.to_le_bytes());
    expected.extend_from_slice(&4_u32.to_le_bytes());
    expected.extend_from_slice(b"core");
    expected.extend_from_slice(&2_u32.to_le_bytes());
    expected.extend_from_slice(&[0xAA, 0xBB]);
    expected.extend_from_slice(&2_u32.to_le_bytes());
    expected.extend_from_slice(&0x0001_u16.to_le_bytes()); // B is bit 0
    expected.extend_from_slice(&0x0800_u16.to_le_bytes()); // R is bit 11
    assert_eq!(&bytes[16..], &expected[..]);
    assert_eq!(bytes[12..16], ssd_rom::crc32(&expected).to_le_bytes(), "the CRC covers the rest");
}

#[test]
fn joypad_bits_round_trip() {
    for bits in 0..=0x0FFF_u16 {
        assert_eq!(joypad_bits(joypad_from_bits(bits)), bits);
    }
    let all = Joypad {
        up: true,
        down: true,
        left: true,
        right: true,
        a: true,
        b: true,
        x: true,
        y: true,
        l: true,
        r: true,
        start: true,
        select: true,
    };
    assert_eq!(joypad_bits(all), 0x0FFF);
    assert_eq!(joypad_bits(Joypad { start: true, ..Joypad::default() }), 1 << 3);
    assert_eq!(joypad_bits(Joypad { a: true, ..Joypad::default() }), 1 << 8);
}

#[test]
fn a_corrupted_byte_anywhere_after_the_checksum_is_caught() {
    let replay = sample();
    let bytes = replay.to_bytes(CORE_VERSION);
    for index in 12..bytes.len() {
        let mut corrupted = bytes.clone();
        corrupted[index] ^= 0x10;
        let message = replay_error(Replay::from_bytes(&corrupted, replay.rom_crc32, CORE_VERSION));
        assert!(message.contains("corrupted (checksum mismatch)"), "byte {index}: {message}");
    }
}

#[test]
fn another_magic_or_format_is_named() {
    let replay = sample();
    let mut bytes = replay.to_bytes(CORE_VERSION);
    bytes[8] = 4;
    let message = replay_error(Replay::from_bytes(&bytes, replay.rom_crc32, CORE_VERSION));
    assert!(message.contains("format 4, expected 1, 2 or 3"), "{message}");
    bytes[0] = b'X';
    let message = replay_error(Replay::from_bytes(&bytes, replay.rom_crc32, CORE_VERSION));
    assert!(message.contains("not a replay"), "{message}");
    let message = replay_error(Replay::from_bytes(b"SSD", replay.rom_crc32, CORE_VERSION));
    assert!(message.contains("not a replay"), "{message}");
}

#[test]
fn a_replay_of_another_rom_is_rejected_naming_both() {
    let replay = sample();
    let bytes = replay.to_bytes(CORE_VERSION);
    let message = replay_error(Replay::from_bytes(&bytes, 0xCAFE_F00D, CORE_VERSION));
    assert!(message.contains("12345678") && message.contains("CAFEF00D"), "{message}");
    assert!(message.contains("ROM"), "{message}");
}

#[test]
fn a_replay_of_another_core_version_is_rejected_naming_both() {
    let replay = sample();
    let bytes = replay.to_bytes("jgenesis 000000000000+00000000");
    let message = replay_error(Replay::from_bytes(&bytes, replay.rom_crc32, CORE_VERSION));
    assert!(message.contains("emulator core"), "{message}");
    assert!(message.contains("jgenesis 000000000000+00000000"), "{message}");
    assert!(message.contains(CORE_VERSION), "{message}");
}

#[test]
fn a_rebased_replay_is_the_same_replay_of_the_new_core() {
    let replay = sample();
    let old = "jgenesis 000000000000+00000000 oracle 1";
    let bytes = replay.to_bytes(old);
    let (was, rebased) = Replay::rebase(&bytes, CORE_VERSION).unwrap();
    assert_eq!(was, old);
    assert_eq!(rebased, replay.to_bytes(CORE_VERSION));
    assert_eq!(Replay::from_bytes(&rebased, replay.rom_crc32, CORE_VERSION).unwrap(), replay);
}

#[test]
fn a_replay_that_is_not_intact_is_not_rebased() {
    let bytes = sample().to_bytes("jgenesis 000000000000+00000000 oracle 1");
    let message = replay_error(Replay::rebase(b"not a replay", CORE_VERSION));
    assert!(message.contains("SSDRPLAY"), "{message}");
    // A flipped input bit: the checksum catches it
    let mut flipped = bytes.clone();
    *flipped.last_mut().unwrap() ^= 1;
    let message = replay_error(Replay::rebase(&flipped, CORE_VERSION));
    assert!(message.contains("corrupted"), "{message}");
    // Trailing bytes, even with a good checksum
    let mut trailing = bytes;
    trailing.push(0);
    let message = replay_error(Replay::rebase(&with_fixed_crc(trailing), CORE_VERSION));
    assert!(message.contains("trailing") || message.contains("rewrite"), "{message}");
}

#[test]
fn a_replay_of_another_oracle_revision_is_rejected() {
    let replay = sample();
    let core = CORE_VERSION.strip_suffix(ORACLE_REVISION).unwrap();
    let older = format!("{core}0");
    let bytes = replay.to_bytes(&older);
    let message = replay_error(Replay::from_bytes(&bytes, replay.rom_crc32, CORE_VERSION));
    assert!(message.contains(&older) && message.contains(CORE_VERSION), "{message}");
}

#[test]
fn write_to_writes_the_file_through_a_temporary_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new").join("run.ssdreplay");
    let replay = sample();
    replay.write_to(&path, CORE_VERSION).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), replay.to_bytes(CORE_VERSION));
    assert!(!path.with_extension("ssdreplay.tmp").exists());

    // Over an existing file; a failed write (here: a directory in the way) leaves no temporary.
    let shorter = Replay { inputs: Vec::new(), ..replay };
    shorter.write_to(&path, CORE_VERSION).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), shorter.to_bytes(CORE_VERSION));
    let blocked = dir.path().join("blocked.ssdreplay");
    std::fs::create_dir(&blocked).unwrap();
    assert!(shorter.write_to(&blocked, CORE_VERSION).is_err());
    assert!(!blocked.with_extension("ssdreplay.tmp").exists());
}

#[test]
fn truncated_trailing_and_unknown_bits_are_named() {
    let replay = sample();
    let bytes = replay.to_bytes(CORE_VERSION);

    let short = with_fixed_crc(bytes[..bytes.len() - 1].to_vec());
    let message = replay_error(Replay::from_bytes(&short, replay.rom_crc32, CORE_VERSION));
    assert!(message.contains("truncated"), "{message}");

    let mut long = bytes.clone();
    long.push(0);
    let message =
        replay_error(Replay::from_bytes(&with_fixed_crc(long), replay.rom_crc32, CORE_VERSION));
    assert!(message.contains("trailing bytes"), "{message}");

    // The last input with bit 12 set.
    let mut unknown = bytes.clone();
    let last = unknown.len() - 1;
    unknown[last] |= 0x10;
    let message =
        replay_error(Replay::from_bytes(&with_fixed_crc(unknown), replay.rom_crc32, CORE_VERSION));
    assert!(message.contains("frame 39") && message.contains("unused bits"), "{message}");

    // A frame count far beyond the data: no allocation, just truncated.
    let mut huge = bytes.clone();
    let frames_at = bytes.len() - 2 * replay.inputs.len() - 4;
    huge[frames_at..frames_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    let message =
        replay_error(Replay::from_bytes(&with_fixed_crc(huge), replay.rom_crc32, CORE_VERSION));
    assert!(message.contains("truncated"), "{message}");
}

// Power-on

#[test]
fn power_on_restores_the_initial_sram_and_the_wram_fill() {
    let rom = sram_counter_rom();
    let reference = machine_with(rom.clone());
    let sram: Vec<u8> = (0..2048).map(|i| (i * 7) as u8).collect();
    let replay = Replay {
        rom_crc32: reference.rom_crc32(),
        wram_seed: 0x0123_4567_89AB_CDEF,
        initial_sram: Some(sram.clone()),
        mode: 0,
        events: Vec::new(),
        packs: None,
        inputs: Vec::new(),
    };
    let m = replay.power_on(rom.clone()).unwrap();
    assert_eq!(m.sram(), Some(&sram[..]));
    assert_eq!(m.wram_seed(), 0x0123_4567_89AB_CDEF);
    let seeded = Machine::with_wram_seed(
        rom.clone(),
        Box::new(MemorySaveStore::default()),
        0x0123_4567_89AB_CDEF,
    )
    .unwrap();
    assert_eq!(m.wram(), seeded.wram(), "the replay's WRAM fill");
    assert_ne!(m.wram(), reference.wram(), "not the default fill");
    assert_eq!(reference.wram_seed(), DEFAULT_WRAM_SEED);
}

#[test]
fn power_on_rejects_another_rom() {
    let replay = Replay::of(&machine_with(sram_counter_rom()));
    let message = replay_error(replay.power_on(red_backdrop_rom()));
    assert!(message.contains("ROM"), "{message}");
}

#[test]
fn an_initial_sram_the_cartridge_does_not_take_is_rejected() {
    let rom = sram_counter_rom();
    let replay = Replay::of(&machine_with(rom.clone()));
    assert_eq!(replay.initial_sram.as_ref().map(Vec::len), Some(2048));

    let other_size = Replay { initial_sram: Some(vec![0; 1024]), ..replay.clone() };
    let message = replay_error(other_size.power_on(rom.clone()));
    assert!(message.contains("initial SRAM (1024 bytes)") && message.contains("2048"), "{message}");

    let none = Replay { initial_sram: None, ..replay };
    let message = replay_error(none.power_on(rom));
    assert!(message.contains("no initial SRAM") && message.contains("2048"), "{message}");

    let rom = red_backdrop_rom();
    let replay =
        Replay { initial_sram: Some(vec![0; 2048]), ..Replay::of(&machine_with(rom.clone())) };
    let message = replay_error(replay.power_on(rom));
    assert!(
        message.contains("2048 bytes of initial SRAM") && message.contains("no SRAM"),
        "{message}"
    );
}

// Recording and playback

/// The routine of `key` checked on every call, contract 8-bit A and 16-bit X/Y.
fn verify(name: &str, key: u32, func: RoutineFn, cycles: u64) -> HookEntry {
    HookEntry {
        name: name.to_owned(),
        key,
        mode: HookMode::Verify,
        action: HookAction::Routine { func, ret: ReturnKind::Rts },
        entry_m8: Some(true),
        entry_x8: Some(false),
        flags_mask: 0xFF,
        verify_sample: 1,
        bound: None,
        budget_frames: None,
        cycles: Some(cycles),
    }
}

/// `INC $11`, then X counts down to 0.
fn long_routine(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(0x11).wrapping_add(1);
    ctx.write_dp8(0x11, value);
    ctx.set_x(0);
    ctx.set_nz16(0);
    ctx.set_cost(LONG_COST);
}

/// `INC $10`.
fn short_routine(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(0x10).wrapping_add(1);
    ctx.write_dp8(0x10, value);
    ctx.set_nz8(value);
    ctx.set_cost(SHORT_COST);
}

/// [`joypad_log_rom`] with both routines in `Verify`.
fn verified_log_machine(rom: Vec<u8>) -> Machine {
    let mut m = machine_with(rom);
    let table: HookTable = [
        verify("long", JOYPAD_LOG_LONG, long_routine, LONG_COST),
        verify("short", JOYPAD_LOG_SHORT, short_routine, SHORT_COST),
    ]
    .into_iter()
    .collect();
    m.set_hook_table(Some(table));
    m
}

fn stats(m: &Machine, name: &str) -> HookStats {
    m.hook_stats().into_iter().find(|(n, _)| n == name).unwrap().1
}

/// The joypad words the NMI handler logged, oldest first.
fn joypad_log(m: &Machine) -> Vec<(u8, u8)> {
    let count = usize::from(m.wram()[JOYPAD_LOG_COUNT]);
    (0..count).map(|i| (m.wram()[JOYPAD_LOG_LOW + i], m.wram()[JOYPAD_LOG_HIGH + i])).collect()
}

/// Records [`FRAMES`] frames of [`varied_input`] on the unhooked `rom`.
fn record(rom: Vec<u8>) -> (Replay, Machine) {
    let mut m = machine_with(rom);
    let mut recorder = Recorder::start(&m).unwrap();
    while m.frame_count() < FRAMES {
        let input = varied_input(m.frame_count());
        recorder.run_frame(&mut m, input, &mut NullAudio).unwrap();
    }
    (recorder.into_replay(), m)
}

#[test]
fn a_recording_plays_back_to_the_same_state() {
    let rom = joypad_log_rom();
    let (replay, recorded) = record(rom.clone());
    assert_eq!(replay.frames(), FRAMES);
    assert_eq!(replay.input(5), varied_input(5));
    assert_eq!(replay.input(FRAMES), Joypad::default(), "released after the end");
    assert!(joypad_log(&recorded).len() > 100, "the handler logged the joypad");

    let bytes = replay.to_bytes(CORE_VERSION);
    let read = Replay::from_bytes(&bytes, recorded.rom_crc32(), CORE_VERSION).unwrap();
    let mut played = read.power_on(rom).unwrap();
    read.play(&mut played, &mut NullAudio).unwrap();
    assert_eq!(played.frame_count(), FRAMES);
    assert!(played.save_state() == recorded.save_state(), "the playback is the recorded run");

    // A machine at the end plays nothing more; one part-way plays the rest.
    let state = played.save_state();
    read.play(&mut played, &mut NullAudio).unwrap();
    assert!(played.save_state() == state, "nothing left to play");
    let mut halfway = read.power_on(joypad_log_rom()).unwrap();
    while halfway.frame_count() < FRAMES / 2 {
        halfway.run_frame_with(&mut |frame| read.input(frame), &mut NullAudio).unwrap();
    }
    read.play(&mut halfway, &mut NullAudio).unwrap();
    assert!(halfway.save_state() == recorded.save_state(), "the rest of the replay");
}

#[test]
fn the_recorder_refuses_hooks_a_running_machine_and_a_rewound_one() {
    let message = replay_error(Recorder::start(&verified_log_machine(joypad_log_rom())));
    assert!(message.contains("without hooks"), "{message}");

    let mut m = machine_with(joypad_log_rom());
    m.tick(Joypad::default(), &mut NullAudio).unwrap();
    let message = replay_error(Recorder::start(&m));
    assert!(message.contains("power-on"), "{message}");

    let mut m = machine_with(joypad_log_rom());
    let mut recorder = Recorder::start(&m).unwrap();
    let start = m.save_state();
    for _ in 0..3 {
        recorder.run_frame(&mut m, Joypad::default(), &mut NullAudio).unwrap();
    }
    m.load_state(&start).unwrap();
    let message = replay_error(recorder.run_frame(&mut m, Joypad::default(), &mut NullAudio));
    assert!(message.contains("frame 0") && message.contains("frame 3"), "{message}");
    assert_eq!(recorder.replay().frames(), 3, "nothing recorded for the refused frame");
}

/// The unhooked run of `replay` on `rom` stopped at master cycle `target`, ticked by frame
/// through its inputs.
fn unhooked_at(rom: Vec<u8>, replay: &Replay, target: u64) -> Machine {
    let mut m = replay.power_on(rom).unwrap();
    while m.total_master_cycles() < target {
        m.tick(replay.input(m.frame_count()), &mut NullAudio).unwrap();
    }
    assert_eq!(m.total_master_cycles(), target, "the same tick boundaries");
    m
}

/// Review Focus 6. The NMI lands inside the long routine, a `Verify` call whose original runs
/// on through the frame's render and the auto-joypad read 4 300 master cycles later. Looked up
/// by frame, the input of those ticks is the next frame's, as in the unhooked run frame by frame;
/// held for the whole `run_frame` call, it would be the frame's before. A run ends after the
/// dispatch in progress, so the unhooked twin is compared at the same master cycle.
#[test]
fn a_hooked_playback_feeds_every_frame_its_recorded_input() {
    let rom = joypad_log_rom();
    let (replay, recorded) = record(rom.clone());
    assert!(joypad_log(&recorded).len() > 100);

    let mut played = verified_log_machine(rom.clone());
    replay.play(&mut played, &mut NullAudio).unwrap();
    let long = stats(&played, "long");
    assert!(long.interrupted > 100, "the NMI lands inside the long routine: {long:?}");
    assert!(long.verified_ok > 0 && long.divergences == 0, "{long:?}");
    let short = stats(&played, "short");
    assert!(short.nested > 100 && short.divergences == 0, "{short:?}");
    let twin = unhooked_at(rom.clone(), &replay, played.total_master_cycles());
    assert_eq!(joypad_log(&played), joypad_log(&twin), "the joypad latched after each NMI");
    assert!(played.save_state() == twin.save_state(), "the whole machine");

    // The same through `run_until_sync` (no sync point: it runs out its budget).
    let mut synced = verified_log_machine(rom.clone());
    let event = synced.run_until_sync(&mut |frame| replay.input(frame), &mut NullAudio, FRAMES);
    assert_eq!(event.unwrap(), None);
    let twin = unhooked_at(rom.clone(), &replay, synced.total_master_cycles());
    assert_eq!(joypad_log(&synced), joypad_log(&twin));
    assert!(synced.save_state() == twin.save_state(), "the whole machine");

    // Held for the whole call, the input of a frame reaches the read of the next one.
    let mut held = verified_log_machine(rom.clone());
    while held.frame_count() < FRAMES {
        held.run_frame(replay.input(held.frame_count()), &mut NullAudio).unwrap();
    }
    let twin = unhooked_at(rom, &replay, held.total_master_cycles());
    assert_ne!(joypad_log(&held), joypad_log(&twin), "the dispatches span the auto-joypad read");
}

/// Without hooks, `run_frame_with` is `run_frame` with each frame's input, tick for tick.
#[test]
fn unhooked_run_frame_with_is_run_frame_frame_by_frame() {
    let rom = joypad_log_rom();
    let mut by_frame = machine_with(rom.clone());
    let mut looked_up = machine_with(rom);
    for frame in 0..30 {
        by_frame.run_frame(varied_input(frame), &mut NullAudio).unwrap();
        looked_up.run_frame_with(&mut varied_input, &mut NullAudio).unwrap();
        assert_eq!(by_frame.total_master_cycles(), looked_up.total_master_cycles());
        assert!(by_frame.save_state() == looked_up.save_state(), "frame {frame}");
    }
}

/// A replay without packs serialises to the bytes it had before pack sets existed: format 1 for a
/// classic replay without host events, format 2 with a mode and events (the expected bytes are
/// pinned, with a fixed core version string).
#[test]
fn a_replay_without_packs_serialises_to_the_same_bytes() {
    let classic = Replay {
        rom_crc32: 0x1234_5678,
        wram_seed: 0xDEAD_BEEF_0123_4567,
        initial_sram: Some(vec![1, 2, 3]),
        inputs: vec![joypad_from_bits(0x0001), joypad_from_bits(0x0A50), joypad_from_bits(0x0FFF)],
        mode: 0,
        events: Vec::new(),
        packs: None,
    };
    assert_eq!(classic.to_bytes("pinned-core"), PINNED_FORMAT_1);
    let enhanced = Replay {
        mode: 1,
        events: vec![
            Vec::new(),
            vec![
                ssd_oracle::HostEvent::Wram { address: 0x7F_FF00, bytes: vec![0xAA, 0xBB] },
                ssd_oracle::HostEvent::App { tag: 2, payload: vec![9] },
            ],
            Vec::new(),
        ],
        ..classic
    };
    assert_eq!(enhanced.to_bytes("pinned-core"), PINNED_FORMAT_2);
}

/// The format 1 bytes of [`a_replay_without_packs_serialises_to_the_same_bytes`].
const PINNED_FORMAT_1: &[u8] = &[
    0x53, 0x53, 0x44, 0x52, 0x50, 0x4C, 0x41, 0x59, 0x01, 0x00, 0x00, 0x00, 0x01, 0xC0, 0xA9, 0x4A,
    0x78, 0x56, 0x34, 0x12, 0x67, 0x45, 0x23, 0x01, 0xEF, 0xBE, 0xAD, 0xDE, 0x0B, 0x00, 0x00, 0x00,
    0x70, 0x69, 0x6E, 0x6E, 0x65, 0x64, 0x2D, 0x63, 0x6F, 0x72, 0x65, 0x03, 0x00, 0x00, 0x00, 0x01,
    0x02, 0x03, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x50, 0x0A, 0xFF, 0x0F,
];
/// The format 2 bytes of [`a_replay_without_packs_serialises_to_the_same_bytes`].
const PINNED_FORMAT_2: &[u8] = &[
    0x53, 0x53, 0x44, 0x52, 0x50, 0x4C, 0x41, 0x59, 0x02, 0x00, 0x00, 0x00, 0x7B, 0x5F, 0xDA, 0x52,
    0x78, 0x56, 0x34, 0x12, 0x67, 0x45, 0x23, 0x01, 0xEF, 0xBE, 0xAD, 0xDE, 0x0B, 0x00, 0x00, 0x00,
    0x70, 0x69, 0x6E, 0x6E, 0x65, 0x64, 0x2D, 0x63, 0x6F, 0x72, 0x65, 0x03, 0x00, 0x00, 0x00, 0x01,
    0x02, 0x03, 0x03, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x50, 0x0A, 0x02,
    0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x7F, 0x00, 0x02, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0x01, 0x02,
    0x01, 0x00, 0x00, 0x00, 0x09, 0xFF, 0x0F, 0x00, 0x00, 0x00, 0x00,
];

// Pack sets (format 3)

/// A 1 MiB ROM for the pack tests: the synthetic 32 KiB cartridge, then `0xFF` to a cartridge's size.
fn one_mebibyte(small: Vec<u8>) -> Vec<u8> {
    let mut rom = small;
    rom.resize(0x10_0000, 0xFF);
    rom
}

/// A pack set's image on `rom`: the ROM, then 1 MiB of extension with one byte of data in it.
fn image_on(rom: &[u8], data: u8) -> Vec<u8> {
    let mut image = rom.to_vec();
    image.resize(0x20_0000, 0);
    image[0x10_0000] = data;
    image
}

/// A replay with a pack set, five frames of input and SRAM.
fn packed_sample() -> Replay {
    Replay {
        packs: Some(ssd_oracle::PackTag {
            set_id: 0x1122_3344_5566_7788,
            image_crc32: 0xABCD_0123,
        }),
        mode: MODE_ENHANCED,
        events: (0..40)
            .map(|frame| {
                if frame == 1 {
                    vec![ssd_oracle::HostEvent::App { tag: 1, payload: vec![4] }]
                } else {
                    Vec::new()
                }
            })
            .collect(),
        ..sample()
    }
}

#[test]
fn a_replay_with_packs_is_written_as_format_3_and_reads_back() {
    let replay = packed_sample();
    let bytes = replay.to_bytes(CORE_VERSION);
    assert_eq!(bytes[8..12], 3_u32.to_le_bytes(), "format 3");
    // The set id and the image CRC32 follow the mode byte, after the core version and the SRAM.
    let at = 16 + 4 + 8 + 4 + CORE_VERSION.len() + 4 + 256 + 4 + 1;
    assert_eq!(bytes[at..at + 8], 0x1122_3344_5566_7788_u64.to_le_bytes(), "the set id");
    assert_eq!(bytes[at + 8..at + 12], 0xABCD_0123_u32.to_le_bytes(), "the image CRC32");
    assert_eq!(Replay::from_bytes(&bytes, replay.rom_crc32, CORE_VERSION).unwrap(), replay);
}

#[test]
fn a_replay_with_packs_keeps_its_section_when_rebased() {
    let replay = packed_sample();
    let bytes = replay.to_bytes(CORE_VERSION);
    let (old, rebased) = Replay::rebase(&bytes, "another-core").unwrap();
    assert_eq!(old, CORE_VERSION);
    let read = Replay::from_bytes(&rebased, replay.rom_crc32, "another-core").unwrap();
    assert_eq!(read, replay, "the same replay, with its pack set");
}

#[test]
fn power_on_refuses_a_replay_with_packs_naming_the_set() {
    let replay = packed_sample();
    let message = replay_error(replay.power_on(one_mebibyte(sram_counter_rom())));
    assert!(message.contains("1122334455667788"), "{message}");
    assert!(message.contains("ABCD0123") || message.contains("abcd0123"), "{message}");
}

#[test]
fn power_on_image_needs_packs_the_rom_and_the_image_crc32() {
    let rom = one_mebibyte(sram_counter_rom());
    let image = image_on(&rom, 0x5A);
    let image_crc = ssd_rom::crc32(&image);
    let rom_crc = ssd_rom::crc32(&rom);
    // The synthetic cartridge has 2 KiB of SRAM: the replays take a SRAM of that size.
    let sram = Some(vec![0; 2048]);
    let plain = Replay { rom_crc32: rom_crc, packs: None, initial_sram: sram, ..packed_sample() };
    let message = replay_error(plain.power_on_image(image.clone()));
    assert!(message.contains("recorded without packs"), "{message}");

    let packed =
        Replay { packs: Some(ssd_oracle::PackTag { set_id: 7, image_crc32: image_crc }), ..plain };
    let machine = packed.power_on_image(image.clone()).unwrap();
    assert_eq!((machine.rom_crc32(), machine.image_crc32()), (rom_crc, image_crc));
    assert_eq!(machine.frame_count(), 0);

    let short = replay_error(packed.power_on_image(rom[..0x8000].to_vec()));
    assert!(short.contains("shorter than the 1 MiB ROM"), "{short}");
    let mut other_rom = image.clone();
    other_rom[0x1000] ^= 1;
    let wrong_rom = replay_error(packed.power_on_image(other_rom));
    assert!(wrong_rom.contains(&format!("{rom_crc:08X}")), "{wrong_rom}");
    let mut other_data = image.clone();
    other_data[0x10_0000] ^= 1;
    let wrong_set = replay_error(packed.power_on_image(other_data));
    assert!(wrong_set.contains("pack set is image"), "{wrong_set}");
}

#[test]
fn a_machine_on_an_image_keys_its_save_states_on_the_image() {
    let rom = one_mebibyte(sram_counter_rom());
    let image = image_on(&rom, 0x5A);
    let other = image_on(&rom, 0x3C);
    let (rom_crc, image_crc) = (ssd_rom::crc32(&rom), ssd_rom::crc32(&image));
    let on = |bytes: Vec<u8>| {
        Machine::with_image(bytes, rom_crc, Box::new(MemorySaveStore::default()), DEFAULT_WRAM_SEED)
            .unwrap()
    };
    let packed = on(image.clone());
    assert_eq!((packed.rom_crc32(), packed.image_crc32()), (rom_crc, image_crc));
    let state = packed.save_state();
    assert_eq!(state[16..20], image_crc.to_le_bytes(), "the state's key is the image's CRC32");
    on(image.clone()).load_state(&state).expect("the same image loads it");
    assert!(on(other).load_state(&state).is_err(), "another image does not");
    let mut alone = machine_with(rom.clone());
    assert!(alone.load_state(&state).is_err(), "the ROM alone does not");
    assert_eq!(
        machine_with(rom).save_state()[16..20],
        rom_crc.to_le_bytes(),
        "a plain machine is keyed on the ROM"
    );
}
