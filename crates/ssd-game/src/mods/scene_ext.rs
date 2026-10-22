//! The extension commands of the scene language (M10, Task 3b): `flagx.set` (`$33`), `flagx.clear` (`$34`) and
//! `flagx.if` (`$35`), and the 2048 extended flags they use.
//!
//! The original engine has 51 commands. Its dispatcher indexes the table of them with the opcode, and the table has no
//! entry for `$33` and up, so a script that runs one would read past the table. The wrapper here takes the dispatch
//! before the table is indexed.
//!
//! The dispatcher (`docs/re/data/scenes.md`, "The commands"): `03:AC2B` runs each frame while a scene is on. When the
//! fetch bit (bit 6 of `state_1648`, `$1648`) is clear and the scene is on, the code at `03:AC41` runs the command in
//! `scene_op` (`$01D9`), which the fetch at `03:AC0B` put there, and `03:AC49` jumps through the table. A command
//! ends by jumping to `03:ACD7`, the common tail: it sets the fetch bit, clears `scene_op_state` (`$01DA`), and returns.
//! The operands are fetched one byte at a time by `03:AC1B`: the byte at `[$A5],Y`, with `Y` the offset `scene_offset`
//! (`$01DC`), which then moves on by one.
//!
//! The hook is at `03:AC41`. The accumulator is 8-bit there: the `AND #$40` at `03:AC38` is encoded with an 8-bit
//! immediate, the width the code assumes at that point, and the `BEQ` at `03:AC3A` that leads to `03:AC41` changes no
//! flag. The traces of the reference replays do not run that code (none of the eight trace runs reaches `03:AC41`),
//! so the widths are read from the code, not observed. The index registers are not read by the wrapper, and it reads
//! its bytes from memory, so they do not matter to it. Direct page is 0 (`$7E:00A5` is the script pointer): the traces
//! have `D = 0000` at every instruction they run.
//!
//! A command below `$33` declines: the wrapper resumes at its own entry, and the original runs, at no cost. A command of
//! `$33` to `$35` reads its operands as the engine reads them, moves `scene_offset` past the command, acts, and resumes at
//! `03:ACD7`, so the next command is fetched exactly as after one of the original's. `flagx.if` moves the offset to its
//! label as `ifflag` does (the label minus the low word of the pointer) when the flag is set, and past the command when it
//! is not. A code above `$35` is not an extension: it moves past its opcode only, as a one-byte command.
//!
//! The extended flags are 256 bytes at `$7F:FD00` (flag `n` is bit `n % 8` of byte `n / 8`). They are valid only while the
//! marker `SSDf` is at `$7F:FCFC`: without it every flag reads clear, and a set first clears the 256 bytes and writes the
//! marker. A clear without the marker writes nothing. A flag past 2047 is neither set nor cleared, and reads clear. The
//! bytes are work RAM above the save image and below the cheat block (`$7F:FF00`); the tests of this module scan the
//! reference replays for writes to `$7F:FCFC` to `$7F:FDFF`. They are not saved yet. The four bytes before the marker
//! (`$7F:FCF8` to `$7F:FCFB`) belong to the trigger dispatch (`mods::triggers`): the marker's first write clears them with
//! the bank, so a state left there before the marker was written is never read as valid.

use ssd_oracle::Ctx;

/// `03:AC41`: the dispatch of the command in progress, before the table is indexed (see the module documentation).
pub(super) const DISPATCH_KEY: u32 = 0x03_AC41;
/// `03:ACD7`: the common tail of the commands. It sets the fetch bit and returns.
const TAIL: u32 = 0x03_ACD7;
/// `scene_op` (`$01D9`): the command in progress.
const SCENE_OP: u32 = 0x7E_01D9;
/// `scene_offset` (`$01DC`), a word: the position of the next byte of the script.
const SCENE_OFFSET: u32 = 0x7E_01DC;
/// The script pointer: a word at `$A5` (the address in the bank) and the bank at `$A7`.
const POINTER: u32 = 0x7E_00A5;
/// The first extension opcode, `flagx.set`.
const FIRST: u8 = 0x33;
/// The extended flags: 256 bytes at `$7F:FD00`, valid while the marker is at `$7F:FCFC`.
const FLAGS: u32 = 0x7F_FD00;
const FLAG_MARKER: u32 = 0x7F_FCFC;
/// The four spare bytes before the marker (`$7F:FCF8-FCFB`), cleared with it (see [`mark`]).
const SPARE: u32 = 0x7F_FCF8;
const SPARE_LEN: u32 = 4;
/// The marker of the extended flags.
const MARKER: [u8; 4] = *b"SSDf";
/// The number of extended flags.
const FLAG_COUNT: u16 = 2048;

/// The bus a command runs on: the game's memory as the CPU sees it. A test runs the commands on a map instead.
pub(super) trait Bus {
    fn read(&self, address: u32) -> u8;
    fn write(&mut self, address: u32, value: u8);
}

/// The bus of a wrapper: the reads and writes of the routine's context.
pub(super) struct Cpu<'c, 'a>(pub(super) &'c mut Ctx<'a>);

impl Bus for Cpu<'_, '_> {
    fn read(&self, address: u32) -> u8 {
        self.0.peek8(address)
    }

    fn write(&mut self, address: u32, value: u8) {
        self.0.write8(address, value);
    }
}

/// The 24-bit script pointer: the word at `$A5` and the bank at `$A7`.
fn pointer(bus: &impl Bus) -> u32 {
    u32::from(bus.read(POINTER))
        | u32::from(bus.read(POINTER + 1)) << 8
        | u32::from(bus.read(POINTER + 2)) << 16
}

/// `scene_offset`: the position of the next byte of the script.
fn offset(bus: &impl Bus) -> u16 {
    u16::from_le_bytes([bus.read(SCENE_OFFSET), bus.read(SCENE_OFFSET + 1)])
}

fn set_offset(bus: &mut impl Bus, value: u16) {
    let [low, high] = value.to_le_bytes();
    bus.write(SCENE_OFFSET, low);
    bus.write(SCENE_OFFSET + 1, high);
}

/// The word at script position `at`, read as the engine reads an operand: the byte at `pointer + at` (24-bit, so a
/// carry goes into the bank), then the byte after it.
fn operand(bus: &impl Bus, pointer: u32, at: u16) -> u16 {
    let byte = |k: u32| bus.read((pointer + u32::from(at) + k) & 0xFF_FFFF);
    u16::from(byte(0)) | u16::from(byte(1)) << 8
}

/// Whether the marker of the extended flags is in place.
pub(super) fn marked(bus: &impl Bus) -> bool {
    MARKER.iter().enumerate().all(|(k, &byte)| bus.read(FLAG_MARKER + k as u32) == byte)
}

/// The byte and the bit of extended flag `flag`, `None` past the bank.
fn bit(flag: u16) -> Option<(u32, u8)> {
    (flag < FLAG_COUNT).then(|| (FLAGS + u32::from(flag / 8), 1 << (flag % 8)))
}

/// Whether extended flag `flag` is set (clear without the marker).
pub(super) fn is_set(bus: &impl Bus, flag: u16) -> bool {
    match bit(flag) {
        Some((address, mask)) if marked(bus) => bus.read(address) & mask != 0,
        _ => false,
    }
}

/// Writes the marker, after clearing the 256 bytes of the flags and the spare bytes before the marker (`$7F:FCF8-FCFB`,
/// where the trigger dispatch keeps its state, `mods::triggers`). Without the marker every flag reads clear, so the clearing
/// changes no flag.
pub(super) fn mark(bus: &mut impl Bus) {
    for k in 0..256 {
        bus.write(FLAGS + k, 0);
    }
    for k in 0..SPARE_LEN {
        bus.write(SPARE + k, 0);
    }
    for (k, &byte) in MARKER.iter().enumerate() {
        bus.write(FLAG_MARKER + k as u32, byte);
    }
}

/// Sets extended flag `flag`; without the marker, first marks the bank (see [`mark`]).
fn set(bus: &mut impl Bus, flag: u16) {
    let Some((address, mask)) = bit(flag) else { return };
    if !marked(bus) {
        mark(bus);
    }
    let value = bus.read(address) | mask;
    bus.write(address, value);
}

/// Clears extended flag `flag`, when the bank is marked (without the marker every flag is clear already).
fn clear(bus: &mut impl Bus, flag: u16) {
    let Some((address, mask)) = bit(flag) else { return };
    if marked(bus) {
        let value = bus.read(address) & !mask;
        bus.write(address, value);
    }
}

/// Runs the extension command `code` for the scene in progress: its operands are read at the script's offset, the offset
/// moves past the command, and the command acts (see the module documentation).
fn run(bus: &mut impl Bus, code: u8) {
    let pointer = pointer(bus);
    let at = offset(bus);
    match code {
        0x33 => {
            let flag = operand(bus, pointer, at);
            set(bus, flag);
            set_offset(bus, at + 2);
        }
        0x34 => {
            let flag = operand(bus, pointer, at);
            clear(bus, flag);
            set_offset(bus, at + 2);
        }
        0x35 => {
            let flag = operand(bus, pointer, at);
            if is_set(bus, flag) {
                // As `ifflag` does: the label is an address in the script's bank, the offset is from the pointer's low word.
                let label = operand(bus, pointer, at + 2);
                set_offset(bus, label.wrapping_sub((pointer & 0xFFFF) as u16));
            } else {
                set_offset(bus, at + 4);
            }
        }
        _ => {}
    }
}

/// `03:AC41`: the extension commands run here; every other command is the original's, which runs when this declines.
pub(super) fn scene_ext_wrapper(ctx: &mut Ctx<'_>) {
    let code = ctx.peek8(SCENE_OP);
    if code < FIRST {
        ctx.resume_at(DISPATCH_KEY);
        return;
    }
    run(&mut Cpu(ctx), code);
    ctx.resume_at(TAIL);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::ops::RangeInclusive;

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

    /// A memory with the script `bytes` at bank address `pointer` and the offset `offset`.
    fn script(bytes: &[u8], pointer: u32, offset: u16) -> Memory {
        let mut memory = Memory::default();
        for (k, &byte) in pointer.to_le_bytes()[..3].iter().enumerate() {
            memory.write(POINTER + k as u32, byte);
        }
        set_offset(&mut memory, offset);
        for (k, &byte) in bytes.iter().enumerate() {
            memory.write(pointer + k as u32, byte);
        }
        memory
    }

    #[test]
    fn an_operand_is_read_where_the_engine_reads_it_even_across_the_bank() {
        let memory = script(&[0x20, 0x41], 0x06_8000, 0);
        assert_eq!(operand(&memory, 0x06_8000, 0), 0x4120);
        let mut across = Memory::default();
        across.write(0x07_0000, 0xAB);
        across.write(0x07_0001, 0xCD);
        assert_eq!(operand(&across, 0x06_FFFF, 1), 0xCDAB, "the carry goes into the bank");
    }

    #[test]
    fn flagx_set_and_clear_use_the_marker_and_the_bit_layout() {
        let mut memory = Memory::default();
        set(&mut memory, 1000);
        assert!(marked(&memory), "the marker is written");
        assert_eq!(memory.read(FLAGS + 125), 0x01, "flag 1000 is bit 0 of byte 125");
        assert!(is_set(&memory, 1000));
        assert!(!is_set(&memory, 999));
        clear(&mut memory, 1000);
        assert!(!is_set(&memory, 1000));
        assert!(marked(&memory), "a clear keeps the marker");
    }

    #[test]
    fn without_the_marker_every_flag_reads_clear_and_a_set_clears_the_bank_first() {
        let mut memory = Memory::default();
        for k in 0..256 {
            memory.write(FLAGS + k, 0xFF);
        }
        assert!(!is_set(&memory, 5), "no marker: clear");
        clear(&mut memory, 3);
        assert!(!marked(&memory), "a clear without the marker writes nothing");
        set(&mut memory, 5);
        assert!(marked(&memory));
        assert_eq!(
            memory.read(FLAGS),
            0x20,
            "the old bytes are cleared: flag 5 is bit 5 of byte 0"
        );
        assert_eq!(memory.read(FLAGS + 1), 0, "and the rest of the bank too");
        assert!(is_set(&memory, 5));
        assert!(!is_set(&memory, 6));
    }

    #[test]
    fn a_flag_past_the_bank_is_neither_set_nor_cleared_and_reads_clear() {
        let mut memory = Memory::default();
        set(&mut memory, FLAG_COUNT);
        assert!(!marked(&memory), "nothing is written past the bank");
        assert!(!is_set(&memory, FLAG_COUNT));
        set(&mut memory, 7);
        clear(&mut memory, FLAG_COUNT);
        assert!(is_set(&memory, 7), "the other flags are unchanged");
    }

    #[test]
    fn set_and_clear_move_past_their_operands_and_if_jumps_or_moves_past_its_four() {
        // The script at $06:8100: the opcode is at offset 0 and the offset is past it, at the operands.
        let mut memory = script(&[0x33, 0x05, 0x00], 0x06_8100, 1);
        run(&mut memory, 0x33);
        assert_eq!(offset(&memory), 3, "set moves past its two operand bytes");
        assert!(is_set(&memory, 5));
        let mut memory = script(&[0x34, 0x05, 0x00], 0x06_8100, 1);
        set(&mut memory, 5);
        run(&mut memory, 0x34);
        assert_eq!(offset(&memory), 3, "clear moves past its two operand bytes");
        assert!(!is_set(&memory, 5));

        // `flagx.if 5, $8120`: with flag 5 set the offset is the label minus the pointer's low word, $20 here.
        let bytes = [0x35, 0x05, 0x00, 0x20, 0x81];
        let mut memory = script(&bytes, 0x06_8100, 1);
        set(&mut memory, 5);
        run(&mut memory, 0x35);
        assert_eq!(offset(&memory), 0x20, "the label, as `ifflag` gives it");
        let mut memory = script(&bytes, 0x06_8100, 1);
        run(&mut memory, 0x35);
        assert_eq!(offset(&memory), 5, "flag clear: past the four operand bytes");
    }

    #[test]
    fn the_addresses_are_the_documented_ones() {
        assert_eq!(DISPATCH_KEY, 0x03_AC41);
        assert_eq!(TAIL, 0x03_ACD7);
        assert_eq!(FLAG_MARKER, 0x7F_FCFC);
        assert_eq!(FLAGS, 0x7F_FD00);
    }

    /// The work-RAM index of `$7F:0000` (the index of `$7E:0000` is 0).
    const BANK_7F: usize = 0x1_0000;
    /// The extended flags' bytes, `$7F:FCFC` to `$7F:FDFF`, and the cheat block's upper range, `$7F:F001` to `$7F:FFFF`.
    const FLAG_RANGE: RangeInclusive<usize> = (BANK_7F + 0xFCFC)..=(BANK_7F + 0xFDFF);
    const UPPER_RANGE: RangeInclusive<usize> = (BANK_7F + 0xF001)..=(BANK_7F + 0xFFFF);
    /// The mod state block, `$7F:FAFC` to `$7F:FBFF` (its marker and its high parts: `mods::state`).
    const MOD_RANGE: RangeInclusive<usize> = (BANK_7F + 0xFAFC)..=(BANK_7F + 0xFBFF);

    /// What one replay wrote into a range of work RAM: the CPU's writes to it, the `WMDATA` writes to it (`$2180`,
    /// at the address `WMADD` holds), and the frames at whose end it differed from the frame before.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Writes {
        cpu: usize,
        wmdata: usize,
        frames: usize,
    }

    /// Plays `replay` on the ROM and counts, for each range, what it wrote there, and every `WMDATA` write.
    fn scan(
        rom: &[u8],
        replay: &ssd_oracle::Replay,
        ranges: &[RangeInclusive<usize>],
    ) -> (Vec<Writes>, usize) {
        let mut machine = replay.power_on(rom.to_vec()).expect("the replay powers on");
        machine.set_cpu_write_log(true);
        let stops = ssd_oracle::Stops { frame_end: true, ..ssd_oracle::Stops::default() };
        let mut before: Vec<Vec<u8>> =
            ranges.iter().map(|range| machine.wram()[range.clone()].to_vec()).collect();
        let mut written: Vec<Writes> = ranges.iter().map(|_| Writes::default()).collect();
        let (mut wmadd, mut wmdata_total) = (0_usize, 0_usize);
        while machine.frame_count() < replay.frames() {
            machine
                .run(&stops, &mut |f| replay.input(f), &mut ssd_oracle::NullAudio, u64::MAX)
                .expect("the replay runs");
            for (address, value) in machine.take_cpu_writes() {
                let (bank, offset) = ((address >> 16) & 0xFF, (address & 0xFFFF) as usize);
                let value = usize::from(value);
                match (bank, offset) {
                    (0x7E | 0x7F, _) => {
                        let at = (bank as usize - 0x7E) * BANK_7F + offset;
                        for (range, count) in ranges.iter().zip(&mut written) {
                            count.cpu += usize::from(range.contains(&at));
                        }
                    }
                    (_, 0x2181) => wmadd = (wmadd & !0xFF) | value,
                    (_, 0x2182) => wmadd = (wmadd & !0xFF00) | value << 8,
                    (_, 0x2183) => wmadd = (wmadd & !0x1_0000) | (value & 1) << 16,
                    (_, 0x2180) => {
                        wmdata_total += 1;
                        let at = wmadd & 0x1_FFFF;
                        for (range, count) in ranges.iter().zip(&mut written) {
                            count.wmdata += usize::from(range.contains(&at));
                        }
                        wmadd = (wmadd + 1) & 0x1_FFFF;
                    }
                    _ => {}
                }
            }
            for ((range, last), count) in ranges.iter().zip(&mut before).zip(&mut written) {
                let now = machine.wram()[range.clone()].to_vec();
                if now != *last {
                    count.frames += 1;
                    *last = now;
                }
            }
        }
        (written, wmdata_total)
    }

    /// The reference replays (`.work/replays`, `SSD_ROM_PATH` for the ROM) never write the extended flags'
    /// bytes, `$7F:FCFC` to `$7F:FDFF`, nor the upper work RAM above the save image, `$7F:F001` to `$7F:FFFF`, nor the
    /// mod state block, `$7F:FAFC` to `$7F:FBFF` (which lies inside the upper range, and is counted on its own too). A
    /// write is counted three ways: a CPU write to the range, a `WMDATA` write that lands in it, and a change of the
    /// range at a frame's end. The table is printed (`--nocapture`); the test fails on any count above zero.
    #[test]
    #[ignore = "needs SSD_ROM_PATH and the reference replays in .work/replays"]
    fn the_reference_replays_never_write_the_extended_flag_bank() {
        let raw = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let rom = ssd_rom::normalize(&raw, None).unwrap().bytes;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.work/replays");
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .expect("the reference replays")
            .map(|entry| entry.expect("a directory entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "ssdreplay"))
            .collect();
        paths.sort();
        assert!(!paths.is_empty(), "no replay in {}", dir.display());
        let mut dirty = Vec::new();
        for path in &paths {
            let bytes = std::fs::read(path).unwrap();
            let replay = ssd_oracle::Replay::from_bytes(
                &bytes,
                ssd_rom::crc32(&rom),
                ssd_oracle::CORE_VERSION,
            )
            .expect("a replay of this ROM and core");
            let (counts, wmdata) = scan(&rom, &replay, &[FLAG_RANGE, UPPER_RANGE, MOD_RANGE]);
            let name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            eprintln!(
                "{name:<28} frames {:>6}  flags: cpu {} wmdata {} frames {}  upper: cpu {} wmdata {} frames {}  mod: cpu {} wmdata {} frames {}  (wmdata writes {wmdata})",
                replay.frames(),
                counts[0].cpu,
                counts[0].wmdata,
                counts[0].frames,
                counts[1].cpu,
                counts[1].wmdata,
                counts[1].frames,
                counts[2].cpu,
                counts[2].wmdata,
                counts[2].frames,
            );
            if counts.iter().any(|c| *c != Writes::default()) {
                dirty.push(name);
            }
        }
        eprintln!("{} replays scanned", paths.len());
        assert!(dirty.is_empty(), "replays that write the range: {dirty:?}");
    }
}
