//! `menu-test cheats`: the battle and map cheats of Enhanced mode, played headless on the cached ROM (M5, tasks 1
//! and 2).
//!
//! The scenarios (`menu-test cheats`, all, or the names given) play the reference replays of `--replays` (default
//! `.work/replays`) with the cheats on and check the numbers against Classic: the battle numbers (`battle`), the
//! Z cards of the whole party and of one fighter (`z-party`, `z-fighter`, `z-fighter-defence`), the Z cards switched
//! on mid-game (`z-mid-game`), the max zeni (`zeni`), the zeni gained by a story event and by a sale (`gain`, `sale`), the level deadline of the Saiyans' chapter that the Remastered story fix removes (`deadline`), random battles off and the encounter rate (`random-off`,
//! `encounter-rate`), and two recorded sessions that replay to the same hash (`recorded`). `--save <file.sav>`
//! adds the scripted loss of Nail against Frieza (`scripted-loss`): the extreme battle cheats against Classic,
//! frame by frame outside the cheat block.
//!
//! `--replay <battle.ssdreplay> [--frames N]` runs the battle numbers alone, as task 1 did.

use crate::menu_test::{self, Harness, Report};
use crate::save;
use anyhow::Context;
use ssd_game::cheats;
use ssd_game::data as d;
use ssd_game::menu::driver::{Curtain, Driver};
use ssd_game::scripts;
use ssd_menus::Key;
use ssd_menus::Screen;
use ssd_menus::bag::{Rules, Sidecar};
use ssd_menus::cheats::{
    BLOCK_LEN, CheatSidecar, Cheats, Multiplier, Row, SETTINGS_LEN, Setting, Z_VALUE,
    ZCARDS_FIGHTER, ZMode, ZTarget,
};
use ssd_oracle::{
    CORE_VERSION, HookMode, HookTable, HostEvent, MODE_ENHANCED, Machine, MemorySaveStore,
    NullAudio, Replay, Symbols, apply_events,
};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::menu_test::Start;

/// The bus address of the first byte of work RAM, and the offsets the checks read.
const WRAM_BASE: u32 = 0x7E_0000;
const DAMAGE: usize = 0x14CF;
const EXP_AWARD: usize = 0x14F3;
const ENEMY_COUNT: usize = 0x0EA0;
/// The first enemy slot (`$0E00`, `$20` bytes a slot, five slots); its HP (`+03`), its maximum HP (`+0A`) and its BP (`+07`).
const SLOT_0: usize = 0x0E00;
const SLOT_STRIDE: usize = 0x20;
const SLOTS: usize = 5;
/// The party records (`$0B00`, nine of `$20` bytes): the HP at `+03`.
const PARTY_0: usize = 0x0B00;
const PARTY_RECORDS: usize = 9;
/// The mode byte and the battle modes, the enemy group of the battle, and the map.
const MODE: usize = 0x24;
const BATTLE_MODES: [u8; 6] = [0x02, 0x04, 0x82, 0x84, 0x8C, 0x0C];
const GROUP: usize = 0x0EA3;
const MAP: usize = 0x01A0;
/// The hand (`$0140`, five cards of six bytes: the flag, the position word, the attack and defence values, the
/// symbol; `docs/re/data/cards.md`), the encounter timer (`$AA`), the item count and the zeni.
const HAND: usize = 0x0140;
const HAND_LEN: usize = 30;
const TIMER: usize = 0x00AA;
/// The value `02:D1F6` reloads the encounter timer with (`$3B`).
const ENCOUNTER_RELOAD: u8 = 59;
const ITEM_COUNT: usize = 0x01B2;
const ZENI: usize = 0x01B4;
/// The frame the block is written at: the game clears work RAM as it boots, so a write before that is lost.
const WRITE_AT: u64 = 300;
/// The gate byte in the block, as an offset in work RAM.
const GATE: usize = (cheats::BLOCK - WRAM_BASE) as usize + ssd_menus::cheats::SUSPENDED;
/// The character ids (`characters`): Goku and Piccolo.
const GOKU: u8 = 0;
const PICCOLO: u8 = 2;
/// The groups of the story's Frieza fights (map 22's first form, map 24's forced fights): the only battles a
/// cheat with random battles off lets start.
const FRIEZA_GROUPS: [u8; 2] = [0x53, 0x59];
/// The Nail fight's timeline (`tools/saves/secrets.sh`, `nail-porunga`).
const NAIL_TIMELINE: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/saves/secrets/mash-8400.txt");
const SYMBOLS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../re/symbols.toml");
/// The reference replays, in the workspace's `.work/replays` (the default `--replays` directory).
const REPLAYS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../.work/replays");

/// What the work RAM holds after a frame, as the scenarios read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Snap {
    zeni: u16,
    hand: [u8; HAND_LEN],
    timer: u8,
    map: u8,
    item_count: u8,
    mode: u8,
    /// The character id (`+1C`) of each party record.
    ids: [u8; PARTY_RECORDS],
}

/// One unit's HP lost in a battle: a party record (`party`) or an enemy slot, and the amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Blow {
    frame: u64,
    party: bool,
    index: u8,
    amount: u32,
}

/// The run's observations, frame by frame.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Trace {
    /// The first hit of the run: the damage and the enemy's maximum HP and BP at it, and the HP the
    /// hit leaves (the first time the HP is below its maximum).
    hit: Option<Hit>,
    /// The largest EXP award of the run (the battle's EXP, cleared when the battle ends).
    exp_award: u32,
    /// The largest enemy count of the run.
    enemies: u8,
    /// A hash of the work RAM outside the cheat block, after each frame.
    hashes: Vec<u64>,
    /// The scripted-loss gate after each frame (the block byte; zero in a run without the block).
    gate: Vec<u8>,
    /// The battles of the run, in order (a battle is a frame in which the mode byte is a battle mode).
    battles: Vec<Battle>,
    /// The HP lost in the battles, in order (enemies and party records).
    blows: Vec<Blow>,
    /// The frames the encounter timer was reloaded in (a roll of the encounter logic).
    rolls: Vec<u64>,
    /// What the work RAM holds after each frame.
    snaps: Vec<Snap>,
    /// The host events each frame was given, when the run is recorded (a format 2 replay of the session).
    events: Vec<Vec<HostEvent>>,
    /// The wrappers' calls, the calls that ran in Rust, and those a timing event sent to the original.
    wrapper_calls: u64,
    wrapper_runs: u64,
    wrapper_fallbacks: u64,
    in_battle: bool,
    /// The HP of each enemy slot and party record after the previous frame (for the blows).
    enemy_hp: [Option<u16>; SLOTS],
    party_hp: [u16; PARTY_RECORDS],
    /// The map and the mode of the last frame.
    final_map: u8,
    final_mode: u8,
    /// A hash of all of work RAM at the end of the run (the cheat block included).
    final_hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Battle {
    group: u8,
    start: u64,
    end: Option<u64>,
    /// The map and the mode the frame after the battle shows.
    map_after: u8,
    mode_after: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hit {
    damage: u32,
    max_hp: u16,
    bp: u32,
    hp_after: Option<u16>,
}

impl Trace {
    /// What the run came to: its first hit, the EXP, the enemies, the battles, and the map and mode at its end.
    fn outcome(&self) -> (Option<Hit>, u32, u8, &[Battle], u8, u8) {
        (self.hit, self.exp_award, self.enemies, &self.battles, self.final_map, self.final_mode)
    }

    fn observe(&mut self, frame: u64, wram: &[u8]) {
        let mode = wram[MODE];
        let battle = BATTLE_MODES.contains(&mode);
        let was_battle = self.in_battle;
        if battle && !self.in_battle {
            self.battles.push(Battle {
                group: wram[GROUP],
                start: frame,
                end: None,
                map_after: 0,
                mode_after: 0,
            });
        } else if !battle
            && self.in_battle
            && let Some(last) = self.battles.last_mut()
        {
            last.end = Some(frame);
            last.map_after = wram[MAP];
            last.mode_after = mode;
        }
        self.in_battle = battle;
        self.final_map = wram[MAP];
        self.final_mode = mode;
        let damage = read24(wram, DAMAGE);
        let max_hp = read16(wram, SLOT_0 + 0x0A);
        match self.hit.as_mut() {
            None if damage != 0 => {
                self.hit =
                    Some(Hit { damage, max_hp, bp: read24(wram, SLOT_0 + 0x07), hp_after: None });
            }
            Some(hit) if hit.hp_after.is_none() && read16(wram, SLOT_0 + 0x03) < hit.max_hp => {
                // The hit is done: the damage the message prints is the one the routine left.
                hit.damage = damage;
                hit.hp_after = Some(read16(wram, SLOT_0 + 0x03));
            }
            _ => {}
        }
        self.exp_award = self.exp_award.max(read24(wram, EXP_AWARD));
        self.enemies = self.enemies.max(wram[ENEMY_COUNT]);
        self.hashes.push(hash_outside_block(wram));
        self.gate.push(wram[GATE]);
        self.blows(frame, wram, battle && was_battle);
        let snap = Snap {
            zeni: read16(wram, ZENI),
            hand: wram[HAND..HAND + HAND_LEN].try_into().unwrap_or([0; HAND_LEN]),
            timer: wram[TIMER],
            map: wram[MAP],
            item_count: wram[ITEM_COUNT],
            mode,
            ids: std::array::from_fn(|k| wram[PARTY_0 + k * 0x20 + 0x1C]),
        };
        // A roll reloads the timer from zero with 59 or less (the cap of a faster rate included). The chain's end sets
        // it to 91 and is not a roll.
        if self.snaps.last().is_some_and(|prev| prev.timer == 0)
            && (1..=ENCOUNTER_RELOAD).contains(&snap.timer)
        {
            self.rolls.push(frame);
        }
        self.snaps.push(snap);
    }

    /// The HP lost since the previous frame, in a battle that was on before this frame too (a battle's first frame
    /// sets the HP apart, as the new fighters come in with theirs).
    fn blows(&mut self, frame: u64, wram: &[u8], counting: bool) {
        for slot in 0..SLOTS {
            let at = SLOT_0 + slot * SLOT_STRIDE;
            // A defeated enemy's flag is cleared: it has no HP left, so its last blow is the HP it had.
            let now = if wram[at] & 0x40 != 0 { read16(wram, at + 0x03) } else { 0 };
            if counting
                && let Some(before) = self.enemy_hp[slot]
                && now < before
            {
                self.blows.push(Blow {
                    frame,
                    party: false,
                    index: slot as u8,
                    amount: u32::from(before - now),
                });
            }
            self.enemy_hp[slot] = Some(now);
        }
        for record in 0..PARTY_RECORDS {
            let now = read16(wram, PARTY_0 + record * 0x20 + 0x03);
            if counting && now < self.party_hp[record] {
                self.blows.push(Blow {
                    frame,
                    party: true,
                    index: record as u8,
                    amount: u32::from(self.party_hp[record] - now),
                });
            }
            self.party_hp[record] = now;
        }
    }
}

fn read16(wram: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([wram[at], wram[at + 1]])
}

fn read24(wram: &[u8], at: usize) -> u32 {
    u32::from(wram[at]) | u32::from(wram[at + 1]) << 8 | u32::from(wram[at + 2]) << 16
}

/// FNV-1a of the work RAM, the cheat block left out.
fn hash_outside_block(wram: &[u8]) -> u64 {
    let start = (cheats::BLOCK - WRAM_BASE) as usize;
    let hash = fnv(&wram[..DEAD_STACK.start], 0xcbf2_9ce4_8422_2325_u64);
    let hash = fnv(&wram[DEAD_STACK.end..start], hash);
    fnv(&wram[start + BLOCK_LEN..], hash)
}

/// The game's stack is here (`$7E:0F80..$0FFF`): the calls of a hooked routine leave their return bytes below the
/// stack pointer, dead once the call returns, and the lockstep of the oracle excludes the same bytes (its replays
/// show `$0FC1..$0FFF`). No named variable lies in this range (`re/symbols.toml`), so the hash leaves it out.
const DEAD_STACK: std::ops::Range<usize> = 0x0F80..0x1000;

/// FNV-1a of `bytes`, continuing from `hash`.
fn fnv(bytes: &[u8], hash: u64) -> u64 {
    bytes.iter().fold(hash, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3))
}

/// The suite, for the start the command line gave and the scenarios named (all when empty).
///
/// # Errors
///
/// A run that cannot be played (an unreadable file, a ROM or core mismatch, a bad symbols file).
pub fn suite(rom: &[u8], start: &Start, only: &[String], out: &mut String) -> anyhow::Result<bool> {
    match start {
        Start::Replay { path, frames } => battle_numbers(rom, path, *frames, out),
        Start::Save { path, slot } => scripted_loss(rom, path, *slot, out),
        Start::Scenarios { dir, save, data_out } => {
            let dir = dir.clone().unwrap_or_else(|| PathBuf::from(REPLAYS));
            let replays = scenarios(rom, &dir, save.as_ref(), only, out)?;
            let screen =
                screen_scenarios(rom, &dir, save.as_ref(), data_out.as_deref(), only, out)?;
            Ok(replays && screen)
        }
    }
}

/// The cheat run's numbers against the run without cheats, for the battle of `path` (task 1).
fn battle_numbers(rom: &[u8], path: &Path, frames: u64, out: &mut String) -> anyhow::Result<bool> {
    let replay = read_replay(rom, path)?;
    let frames = if frames == 0 { replay.frames() } else { frames.min(replay.frames()) };
    let symbols = Symbols::load(SYMBOLS).map_err(anyhow::Error::msg)?;
    let base = play(rom, &replay, None, None, frames)?;
    let mut ok = true;
    let mut line = |out: &mut String, pass: bool, text: String| {
        ok &= pass;
        let _ = writeln!(out, "{} {text}", if pass { "pass" } else { "FAIL" });
    };
    let hit = base.hit.context("the battle has no hit")?;
    line(
        out,
        hit.damage == 19,
        format!("neutral: the first hit {} (19), the enemy HP {:?} (36)", hit.damage, hit.hp_after),
    );
    line(out, base.exp_award == 9, format!("neutral: the EXP award {} (9)", base.exp_award));
    line(
        out,
        hit.max_hp == 55 && hit.bp == 326,
        format!("neutral: the enemy HP {} (55), BP {} (326)", hit.max_hp, hit.bp),
    );
    line(out, base.enemies == 1, format!("neutral: the enemies {} (1)", base.enemies));

    let set = |setting: Setting, index: usize| {
        let mut cheats = Cheats::neutral();
        cheats.set_index(setting, index);
        cheats
    };
    let damage = play(rom, &replay, Some(&set(Setting::Damage, 2)), None, frames)?;
    let hit = damage.hit.context("the damage run has no hit")?;
    line(
        out,
        hit.damage == 38 && hit.hp_after == Some(17),
        format!(
            "damage x2: the first hit {} (38), the enemy HP {:?} (17)",
            hit.damage, hit.hp_after
        ),
    );
    let exp = play(rom, &replay, Some(&set(Setting::Exp, 2)), None, frames)?;
    line(out, exp.exp_award == 27, format!("EXP x3: the EXP award {} (27)", exp.exp_award));
    let power = play(rom, &replay, Some(&set(Setting::Power, 2)), None, frames)?;
    let hit = power.hit.context("the power run has no hit")?;
    line(
        out,
        hit.max_hp == 110 && hit.bp == 652,
        format!("power x2: the enemy HP {} (110), BP {} (652)", hit.max_hp, hit.bp),
    );
    let enemies = play(rom, &replay, Some(&set(Setting::Enemies, 2)), None, frames)?;
    line(out, enemies.enemies == 2, format!("enemies x2: the enemies {} (2)", enemies.enemies));

    let mut all = Cheats::neutral();
    all.set_index(Setting::Damage, 2);
    all.set_index(Setting::Exp, 2);
    all.set_index(Setting::Power, 2);
    let wrappers = play(rom, &replay, Some(&all), None, frames)?;
    let rust = play(rom, &replay, Some(&all), Some(&symbols), frames)?;
    // The wrappers must be reached with the recompiled routines too (generated code calls its callees directly).
    let same = wrappers.outcome() == rust.outcome() && rust.wrapper_calls > 0;
    let parts = format!(
        "hit {}, EXP {}, enemies {}, battles {}, map and mode {}",
        wrappers.hit == rust.hit,
        wrappers.exp_award == rust.exp_award,
        wrappers.enemies == rust.enemies,
        wrappers.battles == rust.battles,
        (wrappers.final_map, wrappers.final_mode) == (rust.final_map, rust.final_mode),
    );
    let _ = writeln!(out, "  the outcomes agree part by part: {parts}");
    let _ = writeln!(
        out,
        "  the wrappers ran {} calls: {} in Rust, {} left to the original (nothing to do, or a timing event); {} calls with the recompiled routines",
        wrappers.wrapper_calls,
        wrappers.wrapper_runs,
        wrappers.wrapper_fallbacks,
        rust.wrapper_calls,
    );
    line(
        out,
        same,
        format!(
            "--hooks original and --hooks rust end in the same outcome on the cheat run ({} frames: the hit, the EXP, the enemies, the battles, the map)",
            wrappers.hashes.len()
        ),
    );
    ok &= recorded_session(rom, &replay, out)?;
    Ok(ok)
}

/// Plays `replay` from power-on. With `cheats`, the wrappers are installed (on the recompiled routines in
/// Rust too, with `symbols`) and the block is written at the first frame.
fn play(
    rom: &[u8],
    replay: &Replay,
    cheats: Option<&Cheats>,
    symbols: Option<&Symbols>,
    frames: u64,
) -> anyhow::Result<Trace> {
    let plan = Plan { start: cheats.copied(), ..Plan::default() };
    play_plan(rom, replay, &plan, symbols, frames)
}

/// What a run does to the cheats: the settings it starts with (written at [`WRITE_AT`], with the hand's corners),
/// the changes made in the course of the run (each at a frame boundary), and host writes at a frame boundary (a
/// stand-in purchase).
#[derive(Debug, Default, Clone)]
struct Plan {
    start: Option<Cheats>,
    changes: Vec<(u64, Cheats)>,
    writes: Vec<(u64, Vec<HostEvent>)>,
}

/// Plays `replay` with `plan` from power-on, the cheats installed when the plan has a start.
fn play_plan(
    rom: &[u8],
    replay: &Replay,
    plan: &Plan,
    symbols: Option<&Symbols>,
    frames: u64,
) -> anyhow::Result<Trace> {
    let mut machine = replay.power_on(rom.to_vec())?;
    if plan.start.is_some() || !plan.changes.is_empty() {
        let mut table = match symbols {
            Some(symbols) => ssd_game::hook_table(symbols, Some(HookMode::Rust))?,
            None => HookTable::new(),
        };
        cheats::install(&mut table);
        machine.set_hook_table(Some(table));
    }
    run_frames(&mut machine, plan, |frame| replay.input(frame), frames)
}

/// Runs `frames` frames of `machine`, with the joypad of each from `input`. The plan's events are made before each
/// frame: the block (once, at [`WRITE_AT`]), the settings changes with the hand, the writes, and the frame events
/// (the gate and the max zeni) of the work RAM before the frame.
fn run_frames(
    machine: &mut Machine,
    plan: &Plan,
    input: impl Fn(u64) -> ssd_oracle::Joypad,
    frames: u64,
) -> anyhow::Result<Trace> {
    let mut trace = Trace::default();
    let cheating = plan.start.is_some() || !plan.changes.is_empty();
    // The frames are the machine's own count: one `run_frame` renders one frame, and the guard stops a run that renders
    // none from looping.
    let mut runs = 0_u64;
    while machine.frame_count() < frames {
        anyhow::ensure!(runs < frames, "the machine rendered no frame in {runs} runs");
        runs += 1;
        let frame = machine.frame_count();
        // A Classic run (no cheats) gets no host events: its block bytes are the power-on fill.
        let mut events = Vec::new();
        if let Some(start) = plan.start.as_ref().filter(|_| frame == WRITE_AT) {
            events.extend(cheats::block_events(start));
            events.extend(cheats::hand_events(start, machine.wram()));
        }
        for (_, changed) in plan.changes.iter().filter(|(at, _)| *at == frame) {
            events.extend(cheats::settings_events(changed, machine.wram()));
        }
        if cheating {
            events.extend(cheats::frame_events(machine.wram()));
        }
        for (_, writes) in plan.writes.iter().filter(|(at, _)| *at == frame) {
            events.extend(writes.iter().cloned());
        }
        apply_events(machine, &events)?;
        machine.run_frame(input(frame), &mut NullAudio)?;
        trace.events.push(events);
        trace.observe(frame, machine.wram());
    }
    for (name, stats) in machine.hook_stats() {
        if name.starts_with("cheat_") {
            trace.wrapper_calls += stats.calls;
            trace.wrapper_runs += stats.rust_runs;
            trace.wrapper_fallbacks += stats.timing_fallback;
        }
    }
    trace.final_hash = full_hash(machine.wram());
    Ok(trace)
}

/// Plays `replay` for 600 frames with `plan`, then sells `object` from `zeni` zeni through the shop's own sale (`03:A790`):
/// a `jsl` to it is written in the upper work RAM (`$7F:FE00`, which the reference replays never write) and run, with A
/// 8-bit and the index 16-bit, as the shop calls it, and the run stops where it returns. Returns the zeni after.
fn sale_event(
    rom: &[u8],
    replay: &Replay,
    plan: &Plan,
    object: u8,
    zeni: u16,
) -> anyhow::Result<u16> {
    const AT: u64 = 600;
    const CALL: u32 = 0x7F_FE00;
    const BACK: u32 = CALL + 4;
    let mut machine = replay.power_on(rom.to_vec())?;
    if plan.start.is_some() {
        let mut table = HookTable::new();
        cheats::install(&mut table);
        machine.set_hook_table(Some(table));
    }
    run_frames(&mut machine, plan, |frame| replay.input(frame), AT)?;
    let input = &mut |frame| replay.input(frame);
    let stops = ssd_oracle::Stops { instructions: Some(1), ..ssd_oracle::Stops::default() };
    let stopped = machine.run(&stops, input, &mut NullAudio, 4)?;
    anyhow::ensure!(
        matches!(stopped, ssd_oracle::Stopped::Instructions),
        "no instruction boundary in four frames: {stopped:?}"
    );
    let stage = [
        HostEvent::Wram { address: 0x7E_01B4, bytes: zeni.to_le_bytes().to_vec() },
        HostEvent::Wram { address: 0x7E_0D7C, bytes: vec![object] },
        HostEvent::Wram { address: 0x7E_0C3F, bytes: vec![0] },
        // jsl $03:A790
        HostEvent::Wram { address: CALL, bytes: vec![0x22, 0x90, 0xA7, 0x03] },
    ];
    apply_events(&mut machine, &stage)?;
    let mut registers = machine.cpu_registers();
    registers.pbr = (CALL >> 16) as u8;
    registers.pc = (CALL & 0xFFFF) as u16;
    registers.dbr = 0;
    registers.d = 0;
    registers.p = ssd_oracle::StatusRegister::from(0x24);
    machine.set_cpu_registers(registers);
    machine.set_breakpoint(BACK);
    let stops =
        ssd_oracle::Stops { frame_end: true, breakpoint: true, ..ssd_oracle::Stops::default() };
    let stopped = machine.run(&stops, input, &mut NullAudio, 1)?;
    machine.clear_breakpoint(BACK);
    anyhow::ensure!(
        matches!(stopped, ssd_oracle::Stopped::Breakpoint { .. }),
        "the sale did not return: {stopped:?}"
    );
    Ok(read16(machine.wram(), ZENI))
}

/// Plays `replay` for 600 frames with `plan`, then runs the level check of map 5 (`03:F300`) with Piccolo at `level` in the first
/// party record, through a `jsl` written at `$7F:FE00` as in [`sale_event`]. Returns the story flag byte `$0173` after (bit 5 is
/// flag `0x1D`, the deadline's).
fn deadline_event(rom: &[u8], replay: &Replay, plan: &Plan, level: u8) -> anyhow::Result<u8> {
    const AT: u64 = 600;
    const CALL: u32 = 0x7F_FE00;
    const BACK: u32 = CALL + 4;
    let mut machine = replay.power_on(rom.to_vec())?;
    if plan.start.is_some() {
        let mut table = HookTable::new();
        cheats::install(&mut table);
        machine.set_hook_table(Some(table));
    }
    run_frames(&mut machine, plan, |frame| replay.input(frame), AT)?;
    let input = &mut |frame| replay.input(frame);
    let stops = ssd_oracle::Stops { instructions: Some(1), ..ssd_oracle::Stops::default() };
    let stopped = machine.run(&stops, input, &mut NullAudio, 4)?;
    anyhow::ensure!(
        matches!(stopped, ssd_oracle::Stopped::Instructions),
        "no instruction boundary in four frames: {stopped:?}"
    );
    let stage = [
        // The first record in the party (`$C0`), Piccolo (id 2) at `level`; the flags clear.
        HostEvent::Wram { address: 0x7E_0B00, bytes: vec![0xC0] },
        HostEvent::Wram { address: 0x7E_0B02, bytes: vec![level] },
        HostEvent::Wram { address: 0x7E_0B1C, bytes: vec![2] },
        HostEvent::Wram { address: 0x7E_0172, bytes: vec![0, 0, 0] },
        // jsl $03:F300
        HostEvent::Wram { address: CALL, bytes: vec![0x22, 0x00, 0xF3, 0x03] },
    ];
    apply_events(&mut machine, &stage)?;
    let mut registers = machine.cpu_registers();
    registers.pbr = (CALL >> 16) as u8;
    registers.pc = (CALL & 0xFFFF) as u16;
    registers.dbr = 0;
    registers.d = 0;
    registers.p = ssd_oracle::StatusRegister::from(0x24);
    machine.set_cpu_registers(registers);
    machine.set_breakpoint(BACK);
    let stops =
        ssd_oracle::Stops { frame_end: true, breakpoint: true, ..ssd_oracle::Stops::default() };
    let stopped = machine.run(&stops, input, &mut NullAudio, 1)?;
    machine.clear_breakpoint(BACK);
    anyhow::ensure!(
        matches!(stopped, ssd_oracle::Stopped::Breakpoint { .. }),
        "the level check did not return: {stopped:?}"
    );
    Ok(machine.wram()[0x0173])
}

/// Plays `replay` for 600 frames with `plan`, then stages the story event that gives zeni (the amount `$0D53` is 40, the
/// zeni 100) by running the game's event dispatcher on it (`01:D433`, to `01:D7F9`), and runs one frame. Returns the zeni and the amount after.
fn money_event(
    rom: &[u8],
    replay: &Replay,
    plan: &Plan,
    symbols: Option<&Symbols>,
) -> anyhow::Result<(u16, u16)> {
    const AT: u64 = 600;
    let mut machine = replay.power_on(rom.to_vec())?;
    if plan.start.is_some() {
        let mut table = match symbols {
            Some(symbols) => ssd_game::hook_table(symbols, Some(HookMode::Rust))?,
            None => HookTable::new(),
        };
        cheats::install(&mut table);
        machine.set_hook_table(Some(table));
    }
    run_frames(&mut machine, plan, |frame| replay.input(frame), AT)?;
    run_money_event(&mut machine, &mut |frame| replay.input(frame), symbols.is_some())
}

/// Stages the story event that gives zeni on `machine` (the amount `$0D53` is 40, the zeni 100) and runs it to its end.
/// Returns the zeni and the amount after.
fn run_money_event(
    machine: &mut Machine,
    input: &mut dyn FnMut(u64) -> ssd_oracle::Joypad,
    at_hook: bool,
) -> anyhow::Result<(u16, u16)> {
    if at_hook {
        // With routines in Rust the CPU may be inside one at a frame's end: it is taken right after the dispatch of a hooked
        // routine instead, which is an opcode fetch whatever the routines it runs on.
        let stops = ssd_oracle::Stops { after_dispatch: true, ..ssd_oracle::Stops::default() };
        let stopped = machine.run(&stops, input, &mut NullAudio, 4)?;
        anyhow::ensure!(
            matches!(stopped, ssd_oracle::Stopped::Dispatched { .. }),
            "no hooked routine ran in four frames: {stopped:?}"
        );
    } else {
        // Between two instructions: the registers are not taken in the middle of one, of a DMA or of an interrupt's entry.
        let stops = ssd_oracle::Stops { instructions: Some(1), ..ssd_oracle::Stops::default() };
        let stopped = machine.run(&stops, input, &mut NullAudio, 4)?;
        anyhow::ensure!(
            matches!(stopped, ssd_oracle::Stopped::Instructions),
            "no instruction boundary in four frames: {stopped:?}"
        );
    }
    let stage = [
        HostEvent::Wram { address: 0x7E_01B4, bytes: 100_u16.to_le_bytes().to_vec() },
        HostEvent::Wram { address: 0x7E_0D53, bytes: 40_u16.to_le_bytes().to_vec() },
    ];
    apply_events(machine, &stage)?;
    // The event is reached as the game reaches it: the dispatcher at `01:D433` reads the event number (20) from `$072E`
    // and jumps through its table to `01:D7F9`.
    machine.bus_write(0x7E_072E, 20);
    let mut registers = machine.cpu_registers();
    registers.pbr = 0x01;
    registers.pc = 0xD433;
    registers.p = ssd_oracle::StatusRegister::default();
    machine.set_cpu_registers(registers);
    // The event ends with a jump to `01:D574`; the run stops there, before anything else of the game runs (in the original code).
    machine.set_breakpoint(0x01_D574);
    let stops =
        ssd_oracle::Stops { frame_end: true, breakpoint: true, ..ssd_oracle::Stops::default() };
    let stopped = machine.run(&stops, input, &mut NullAudio, 1)?;
    machine.clear_breakpoint(0x01_D574);
    // With the recompiled routines the run may go on to the end of the frame without an arrival at the breakpoint; the
    // credit is made by then all the same, and nothing else writes the zeni.
    anyhow::ensure!(
        matches!(stopped, ssd_oracle::Stopped::Breakpoint { .. } | ssd_oracle::Stopped::FrameEnd),
        "the money event did not run: {stopped:?}"
    );
    Ok((read16(machine.wram(), ZENI), read16(machine.wram(), 0x0D53)))
}

/// FNV-1a of all of work RAM: what a recorded session must end on when it is played back.
fn full_hash(wram: &[u8]) -> u64 {
    fnv(wram, 0xcbf2_9ce4_8422_2325_u64)
}

fn read_replay(rom: &[u8], path: &Path) -> anyhow::Result<Replay> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Replay::from_bytes(&bytes, ssd_rom::crc32(rom), CORE_VERSION)?)
}

/// The Nail fight from the crafted save: Classic against the extreme cheats, frame by frame.
fn scripted_loss(rom: &[u8], path: &Path, slot: usize, out: &mut String) -> anyhow::Result<bool> {
    let sram = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    // The extreme choices of the scripted-loss check: defence x10, damage x10, enemy power x5, EXP x10.
    let mut extreme = Cheats::neutral();
    extreme.set_index(Setting::Defence, 5);
    extreme.set_index(Setting::Damage, 5);
    extreme.set_index(Setting::Power, 4);
    extreme.set_index(Setting::Exp, 4);
    fight_gate(rom, &sram, slot, extreme, out)
}

/// The scripted loss of Nail against Frieza with the `extreme` cheats on, against Classic: the run is identical outside
/// the cheat block until the fight is over, and the gate rises at the fight and falls after it.
fn fight_gate(
    rom: &[u8],
    sram: &[u8],
    slot: usize,
    extreme: Cheats,
    out: &mut String,
) -> anyhow::Result<bool> {
    let timeline = scripts::parse_timeline(&format!(
        "{}{}",
        save::continue_prefix(slot),
        std::fs::read_to_string(NAIL_TIMELINE)
            .with_context(|| format!("reading {NAIL_TIMELINE}"))?
    ))
    .map_err(anyhow::Error::msg)?;
    let start = || -> anyhow::Result<Machine> {
        let mut saves = MemorySaveStore::default();
        saves.0.insert("sav".to_owned(), sram.to_vec());
        Ok(Machine::new(rom.to_vec(), Box::new(saves))?)
    };
    let mut classic = start()?;
    let classic_run = run_frames(
        &mut classic,
        &Plan::default(),
        |frame| timeline_input(&timeline, frame),
        timeline.len() as u64,
    )?;
    let mut cheat = start()?;
    // The wrappers alone on the original routines: the claim is the gate, not the Rust routines.
    let mut table = HookTable::new();
    cheats::install(&mut table);
    cheat.set_hook_table(Some(table));
    let plan = Plan { start: Some(extreme), ..Plan::default() };
    let cheat_run = run_frames(
        &mut cheat,
        &plan,
        |frame| timeline_input(&timeline, frame),
        timeline.len() as u64,
    )?;
    let first = classic_run.hashes.iter().zip(&cheat_run.hashes).position(|(a, b)| a != b);
    let identical = classic_run.hashes.len() == cheat_run.hashes.len();
    // The gate is read from the block, which is written at WRITE_AT: the power-on fill before it is not a gate.
    let from = WRITE_AT as usize;
    let rise = cheat_run.gate[from..].iter().position(|&gate| gate != 0).map(|k| from + k);
    let fall =
        rise.and_then(|at| cheat_run.gate[at..].iter().position(|&gate| gate == 0).map(|k| at + k));
    // Identical through the fight: the first difference, if any, comes after the gate has come back on.
    let through = first.is_none_or(|frame| fall.is_some_and(|fall| frame > fall));
    let pass = identical && through && rise.is_some() && fall.is_some();
    let _ = writeln!(
        out,
        "{} the scripted loss, {} frames, the extreme cheats against Classic: {}; the gate rises at {} and falls at {}",
        if pass { "pass" } else { "FAIL" },
        timeline.len(),
        first.map_or_else(
            || "identical outside the block".to_owned(),
            |frame| format!("first differs at frame {frame}")
        ),
        rise.map_or_else(|| "never".to_owned(), |frame| format!("frame {frame}")),
        fall.map_or_else(|| "never".to_owned(), |frame| format!("frame {frame}")),
    );
    Ok(pass)
}

/// The save the screen scenarios start from when `--save` is not given: a copy of a first-chapter save.
const DEFAULT_SAVE: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../.work/demo/a-goku-level-10.sav");
/// The Nail fight's save for the scripted loss through the screen when `--save` is not given (a copy in `.work/demo`).
const NAIL_SAVE: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../.work/demo/secret-nail-porunga.sav");
/// The cheat block as an offset of work RAM, the bytes of a card in the hand, and the attack corner of a card.
const BLOCK_AT: usize = (cheats::BLOCK - WRAM_BASE) as usize;
const HAND_CARD: usize = 6;
const CARD_ATTACK: usize = 3;
const PLAYED: u8 = 0x80;

/// The Cheats screen's scenarios (M5, task 3): the choices made through the screen, from a save's Continue. `--save`
/// names the save (the Nail save for `screen-scripted-loss`); the replays of `dir` are the battle's.
fn screen_scenarios(
    rom: &[u8],
    dir: &Path,
    save: Option<&(PathBuf, usize)>,
    data_out: Option<&Path>,
    only: &[String],
    out: &mut String,
) -> anyhow::Result<bool> {
    let wanted = |name: &str| only.is_empty() || only.iter().any(|o| o == name);
    let sram = std::fs::read(DEFAULT_SAVE).with_context(|| format!("reading {DEFAULT_SAVE}"))?;
    let slot = 0;
    let mut ok = true;
    if wanted("screen-keys") {
        let _ =
            writeln!(out, "  screen-keys: the Cheats screen through the keys, from the Menu list");
        ok &= screen_keys(rom, &sram, slot, out)?;
    }
    if wanted("screen-battle") {
        let _ = writeln!(out, "  screen-battle: the screen's Damage x2 in the battle replay");
        ok &= screen_battle(rom, dir, &sram, slot, out)?;
    }
    if wanted("screen-scripted-loss") {
        let _ = writeln!(
            out,
            "  screen-scripted-loss: the extreme choices made on the screen, the Nail fight"
        );
        let (nail, nail_slot) = save
            .map_or_else(|| (PathBuf::from(NAIL_SAVE), 0), |(path, slot)| (path.clone(), *slot));
        let nail_sram =
            std::fs::read(&nail).with_context(|| format!("reading {}", nail.display()))?;
        ok &= screen_scripted_loss(rom, &sram, slot, &nail_sram, nail_slot, out)?;
    }
    if wanted("screen-persist") {
        let _ = writeln!(out, "  screen-persist: the Save, Continue and the CHEATS badge");
        ok &= screen_persist(rom, &sram, slot, data_out, out)?;
    }
    if wanted("screen-difficulty") {
        let _ = writeln!(
            out,
            "  screen-difficulty: the Difficulty presets through the Options screen, the money event and the badge"
        );
        ok &= screen_difficulty(rom, &sram, slot, out)?;
    }
    if wanted("screen-gameplay") {
        let _ = writeln!(
            out,
            "  screen-gameplay: the Gameplay page: the text speed, the auto-run, the animation speed and the slot's values"
        );
        ok &= screen_gameplay(rom, &sram, slot, out)?;
    }
    if wanted("screen-resave") {
        let _ = writeln!(
            out,
            "  screen-resave: a second Save of the same data writes the cheats it is made with"
        );
        ok &= screen_resave(rom, &sram, slot, out)?;
    }
    Ok(ok)
}

/// A harness at the field of a save: its Continue, the save's cheats bound from `sidecar` (the first frame binds
/// them, as the window does), and the cheat wrappers installed as an Enhanced run has them.
fn field_harness(
    rom: &[u8],
    sram: &[u8],
    slot: usize,
    sidecar: CheatSidecar,
) -> anyhow::Result<Harness> {
    field_harness_with(rom, sram, slot, sidecar, None)
}

/// [`field_harness`] with the options of the save slots (`ssd.options`) the driver is given.
fn field_harness_with(
    rom: &[u8],
    sram: &[u8],
    slot: usize,
    sidecar: CheatSidecar,
    options: Option<ssd_menus::options::SlotOptions>,
) -> anyhow::Result<Harness> {
    let make = || -> anyhow::Result<Machine> {
        let mut saves = MemorySaveStore::default();
        saves.0.insert("sav".to_owned(), sram.to_vec());
        Ok(Machine::new(rom.to_vec(), Box::new(saves))?)
    };
    let mut at_title = make()?;
    let timeline =
        scripts::parse_timeline(&save::continue_prefix(slot)).map_err(anyhow::Error::msg)?;
    for pad in timeline {
        at_title.run_frame(pad, &mut NullAudio)?;
    }
    let state = at_title.save_state();
    let mut machine = make()?;
    machine.load_state(&state)?;
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    let labels = ssd_catalogue::menu_labels(&catalogue);
    let mut driver =
        Driver::with_bag(labels, Rules::default(), Sidecar::empty()).with_cheats(sidecar);
    if let Some(options) = options {
        driver = driver.with_options(options, ssd_menus::options::Values::new());
    }
    let mut table = HookTable::new();
    cheats::install(&mut table);
    machine.set_hook_table(Some(table));
    let mut h = Harness::new(machine, driver);
    h.frame(ssd_oracle::Joypad::default())?;
    Ok(h)
}

/// The block's settings bytes, as work RAM holds them.
fn block_settings(h: &Harness) -> [u8; SETTINGS_LEN] {
    let mut bytes = [0_u8; SETTINGS_LEN];
    bytes.copy_from_slice(&h.machine.wram()[BLOCK_AT..BLOCK_AT + SETTINGS_LEN]);
    bytes
}

/// Whether every card of the hand that is not played shows Z in its attack corner.
fn hand_attack_is_z(wram: &[u8]) -> bool {
    (0..HAND_LEN / HAND_CARD).all(|card| {
        let at = HAND + card * HAND_CARD;
        wram[at] & PLAYED != 0 || wram[at + CARD_ATTACK] == Z_VALUE
    })
}

/// Opens the Cheats screen from the field: the Menu list (the fourth row of the command window, then its fifth row).
/// Whether the screen is up afterwards.
fn open_cheats(h: &mut Harness) -> anyhow::Result<bool> {
    menu_test::menu_list(h)?;
    for _ in 0..5 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    Ok(h.driver.nav().screen() == Screen::Cheats && h.driver.curtain() == Curtain::Up)
}

/// Walks the Cheats screen's cursor to `row` with Down presses (the cursor wraps).
fn goto(h: &mut Harness, row: Row) -> anyhow::Result<()> {
    let target = Row::ALL.iter().position(|&r| r == row).unwrap_or(0);
    for _ in 0..Row::ALL.len() {
        if usize::from(h.driver.nav().cursor()) == target {
            return Ok(());
        }
        h.key(Key::Down)?;
    }
    anyhow::bail!("{row:?} is not reached on the Cheats screen")
}

/// The Cheats screen through the keys: every row's choices wrap at both ends and the block follows each choice; Z
/// cards for the whole party reach the hand, a Z for one fighter counts once in the On list, and Reset all clears
/// the block and keeps the hand's corners. B leaves the screen back to the Menu list, and closes the menu from it.
fn screen_keys(rom: &[u8], sram: &[u8], slot: usize, out: &mut String) -> anyhow::Result<bool> {
    let mut checks = Checks { out, ok: true };
    let mut h = field_harness(rom, sram, slot, CheatSidecar::empty())?;
    let opened = open_cheats(&mut h)?;
    checks.check(
        opened && h.byte(d::MENU_SCREEN) == 4,
        "the Cheats screen opens from the Menu list, and the original is sent nothing".to_owned(),
    );
    checks.check(
        block_settings(&h) == Cheats::neutral().settings_bytes(),
        "the block is neutral after Continue: a save with no entry has all off".to_owned(),
    );
    // The item spawner is an action row, as Reset all is: A or Right opens its page (`spawner` covers it).
    for row in Row::ALL.into_iter().filter(|row| row.choices() > 0 && *row != Row::ItemSpawner) {
        goto(&mut h, row)?;
        let count = row.choices();
        let start = row.index(h.driver.cheats());
        for _ in 0..=start {
            h.key(Key::Left)?;
        }
        let past_first = row.index(h.driver.cheats());
        h.key(Key::Right)?;
        let past_last = row.index(h.driver.cheats());
        for _ in 0..start {
            h.key(Key::Right)?;
        }
        let back = row.index(h.driver.cheats());
        checks.check(
            past_first == count - 1
                && past_last == 0
                && back == start
                && block_settings(&h) == h.driver.cheats().settings_bytes(),
            format!(
                "{}: Left past the first choice wraps to {}, Right past the last to 0, back to {start}, and the block follows",
                row.label(),
                count - 1
            ),
        );
    }
    goto(&mut h, Row::ZCards)?;
    h.key(Key::Right)?;
    checks.check(
        hand_attack_is_z(h.machine.wram()),
        "Z cards for the whole party: the cards in hand show Z in the attack corner".to_owned(),
    );
    goto(&mut h, Row::Damage)?;
    h.key(Key::Right)?;
    checks.check(h.driver.cheats().on_count() == 2, "On: 2 with damage and the Z cards".to_owned());
    goto(&mut h, Row::ZFor)?;
    h.key(Key::Right)?;
    checks.check(
        h.driver.cheats().on_count() == 2 && block_settings(&h)[ZCARDS_FIGHTER] == 1,
        "a Z for one fighter counts once (On: 2) and the block names the first character"
            .to_owned(),
    );
    goto(&mut h, Row::ResetAll)?;
    h.key(Key::Confirm)?;
    checks.check(
        h.driver.cheats().is_neutral() && block_settings(&h) == Cheats::neutral().settings_bytes(),
        "Reset all: every choice is neutral and the block is neutral".to_owned(),
    );
    checks.check(
        hand_attack_is_z(h.machine.wram()),
        "Reset all leaves the hand's Z corners as they are".to_owned(),
    );
    h.key(Key::Cancel)?;
    checks
        .check(h.driver.nav().screen() == Screen::Menu, "B goes back to the Menu list".to_owned());
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    checks.check(
        h.driver.curtain() == Curtain::Down && h.byte(d::FIELD_STATE) == 0,
        "B from the Menu list and the command window closes the menu".to_owned(),
    );
    Ok(checks.ok)
}

/// The first hit of the battle replay with Damage x2 chosen on the screen: 38 (19 without), the block the screen wrote
/// is the one the battle plays with.
fn screen_battle(
    rom: &[u8],
    dir: &Path,
    sram: &[u8],
    slot: usize,
    out: &mut String,
) -> anyhow::Result<bool> {
    let mut checks = Checks { out, ok: true };
    let mut h = field_harness(rom, sram, slot, CheatSidecar::empty())?;
    checks.check(open_cheats(&mut h)?, "the Cheats screen opens from the Menu list".to_owned());
    goto(&mut h, Row::Damage)?;
    h.key(Key::Right)?;
    let chosen = *h.driver.cheats();
    checks.check(
        chosen.multiplier(Setting::Damage) == Multiplier::new(2, 1),
        "the screen's Right makes Damage x2".to_owned(),
    );
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    checks.check(
        block_settings(&h) == chosen.settings_bytes(),
        "the screen wrote its choice into the block".to_owned(),
    );
    let replay = read_replay(rom, &dir.join("battle.ssdreplay"))?;
    let frames = replay.frames();
    let neutral = play(rom, &replay, None, None, frames)?;
    let damage = play(rom, &replay, Some(&chosen), None, frames)?;
    let base = neutral.hit.context("the battle has no hit")?;
    let hit = damage.hit.context("the damage run has no hit")?;
    checks.check(base.damage == 19, format!("without it the first hit is {} (19)", base.damage));
    checks.check(
        hit.damage == 38 && hit.hp_after == Some(17),
        format!(
            "damage x2 chosen on the screen: the first hit {} (38), the enemy HP {:?} (17)",
            hit.damage, hit.hp_after
        ),
    );
    Ok(checks.ok)
}

/// The Nail fight with the extreme choices made on the screen (defence x10, damage x10, enemy power x5, EXP x10): the
/// same checks as the scripted loss of `--save`, with the choices taken from the screen.
fn screen_scripted_loss(
    rom: &[u8],
    sram: &[u8],
    slot: usize,
    nail: &[u8],
    nail_slot: usize,
    out: &mut String,
) -> anyhow::Result<bool> {
    let mut checks = Checks { out, ok: true };
    let mut h = field_harness(rom, sram, slot, CheatSidecar::empty())?;
    checks.check(open_cheats(&mut h)?, "the Cheats screen opens from the Menu list".to_owned());
    goto(&mut h, Row::Defence)?;
    for _ in 0..4 {
        h.key(Key::Right)?;
    }
    goto(&mut h, Row::Damage)?;
    for _ in 0..4 {
        h.key(Key::Right)?;
    }
    goto(&mut h, Row::EnemyPower)?;
    for _ in 0..3 {
        h.key(Key::Right)?;
    }
    goto(&mut h, Row::Exp)?;
    for _ in 0..4 {
        h.key(Key::Right)?;
    }
    let extreme = *h.driver.cheats();
    checks.check(
        extreme.multiplier(Setting::Defence) == Multiplier::new(10, 1)
            && extreme.multiplier(Setting::Damage) == Multiplier::new(10, 1)
            && extreme.multiplier(Setting::Power) == Multiplier::new(5, 1)
            && extreme.multiplier(Setting::Exp) == Multiplier::new(10, 1),
        "the screen holds defence x10, damage x10, enemy power x5 and EXP x10".to_owned(),
    );
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    let fight = fight_gate(rom, nail, nail_slot, extreme, &mut *checks.out)?;
    checks.ok &= fight;
    Ok(checks.ok)
}

/// The cheats follow the game's own Save: the sidecar keeps them by the slot's checksum word, a save made with a cheat
/// on carries the CHEATS badge, Continue restores the choices and the badge, and the other save the slot held (changed
/// by another way) has neither. A save with every cheat off has no badge.
fn screen_persist(
    rom: &[u8],
    sram: &[u8],
    slot: usize,
    data_out: Option<&Path>,
    out: &mut String,
) -> anyhow::Result<bool> {
    let mut checks = Checks { out, ok: true };
    let mut h = field_harness(rom, sram, slot, CheatSidecar::empty())?;
    checks.check(open_cheats(&mut h)?, "the Cheats screen opens from the Menu list".to_owned());
    goto(&mut h, Row::Damage)?;
    h.key(Key::Right)?;
    goto(&mut h, Row::RandomBattles)?;
    h.key(Key::Right)?;
    let chosen = *h.driver.cheats();
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    let mut saved = Report::default();
    menu_test::menu_save(&mut h, &mut saved)?;
    checks.check(saved.passed(), "the game's Save runs to Completed from the Menu list".to_owned());
    let written = h.machine.sram().context("the cartridge has its save")?.to_vec();
    let marks = h.driver.marked_slots(&written);
    let Some(saved_slot) = marks.iter().position(|&marked| marked) else {
        checks.check(false, format!("the save made with a cheat on has the badge: {marks:?}"));
        return Ok(checks.ok);
    };
    checks.check(
        marks.iter().filter(|&&marked| marked).count() == 1,
        format!("the save made with a cheat on has the badge, and only it: {marks:?}"),
    );
    let sidecar = h.driver.cheat_sidecar().clone();
    let again = field_harness(rom, &written, saved_slot, sidecar.clone())?;
    checks.check(
        *again.driver.cheats() == chosen && block_settings(&again) == chosen.settings_bytes(),
        "Continue restores the settings of the save, and the block carries them".to_owned(),
    );
    checks.check(
        again.driver.marked_slots(&written)[saved_slot],
        "the restored save keeps its badge on the Continue list".to_owned(),
    );
    let other = field_harness(rom, sram, slot, sidecar)?;
    checks.check(
        *other.driver.cheats() == Cheats::neutral() && !other.driver.marked_slots(sram)[slot],
        "a save the slot held before (changed by another way) has no settings and no badge"
            .to_owned(),
    );
    let mut off = field_harness(rom, sram, slot, CheatSidecar::empty())?;
    let mut plain = Report::default();
    menu_test::menu_save(&mut off, &mut plain)?;
    let plain_sram = off.machine.sram().context("the cartridge has its save")?.to_vec();
    checks.check(
        plain.passed() && off.driver.marked_slots(&plain_sram) == [false; 3],
        "a save with every cheat off has no badge".to_owned(),
    );
    if let Some(dir) = data_out {
        // The save the game wrote and the sidecar with its cheats: the UI snapshots of the badges read them.
        let saves = dir.join("saves");
        std::fs::create_dir_all(&saves).with_context(|| format!("creating {}", saves.display()))?;
        std::fs::write(saves.join("ssd.cheats"), h.driver.cheat_sidecar().to_bytes())?;
        std::fs::write(dir.join("persist.sav"), &written)?;
        let _ = writeln!(
            checks.out,
            "  wrote the save of slot {} and its cheats to {}",
            saved_slot + 1,
            dir.display()
        );
    }
    Ok(checks.ok)
}

/// The zeni after the story's money event runs on the harness's machine (100 zeni, 40 given); the machine is put back as it
/// was, so the run goes on.
fn pay(h: &mut Harness) -> anyhow::Result<u16> {
    let before = h.machine.save_state();
    let (zeni, _) = run_money_event(&mut h.machine, &mut |_| ssd_oracle::Joypad::default(), false)?;
    h.machine.load_state(&before)?;
    Ok(zeni)
}

/// Opens the Options screen from the field: the Menu list's seventh row. Whether the screen is up afterwards.
fn open_options(h: &mut Harness) -> anyhow::Result<bool> {
    menu_test::menu_list(h)?;
    for _ in 0..6 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    Ok(h.driver.nav().screen() == Screen::Options && h.driver.curtain() == Curtain::Up)
}

/// The money bytes of the block, as work RAM holds them.
fn block_money(h: &Harness) -> [u8; 2] {
    let at = BLOCK_AT + ssd_menus::cheats::MONEY;
    [h.machine.wram()[at], h.machine.wram()[at + 1]]
}

/// The Difficulty page through the keys: Hard, Easy and Normal write the cheats and the block (the money pair among them),
/// the story's money event pays x1/2, x2 and x1 on them, a value changed on the Cheats screen reads Custom, the save made on
/// Easy carries the CHEATS badge and the one made on Normal does not, and Continue restores the money.
fn screen_difficulty(
    rom: &[u8],
    sram: &[u8],
    slot: usize,
    out: &mut String,
) -> anyhow::Result<bool> {
    let mut checks = Checks { out, ok: true };
    let mut h = field_harness(rom, sram, slot, CheatSidecar::empty())?;
    checks.check(open_options(&mut h)?, "the Options screen opens from the Menu list".to_owned());
    let preset = |h: &Harness| ssd_menus::options::difficulty::preset_of(h.driver.cheats());
    checks.check(
        preset(&h) == Some(1) && block_money(&h) == [1, 1],
        "Continue: Normal, money x1 in the block".to_owned(),
    );
    // Hard: power x2, EXP x1, money x1/2. The event runs on a copy of the machine.
    h.key(Key::Right)?;
    let hard = *h.driver.cheats();
    h.frame(ssd_oracle::Joypad::default())?;
    checks.check(
        preset(&h) == Some(2)
            && hard.multiplier(Setting::Power) == Multiplier::new(2, 1)
            && hard.multiplier(Setting::Exp) == Multiplier::new(1, 1)
            && block_settings(&h) == hard.settings_bytes()
            && block_money(&h) == [1, 2],
        "Hard: enemy power x2, EXP x1, money x1/2 reach the block".to_owned(),
    );
    let zeni = pay(&mut h)?;
    checks.check(
        zeni == 120,
        format!("Hard: the event adds 40 to 100 zeni and pays x1/2: {zeni} (120)"),
    );
    // Right again wraps to Easy: power x1/2, EXP x2, money x2.
    h.key(Key::Right)?;
    let easy = *h.driver.cheats();
    h.frame(ssd_oracle::Joypad::default())?;
    checks.check(
        preset(&h) == Some(0)
            && easy.multiplier(Setting::Power) == Multiplier::new(1, 2)
            && easy.multiplier(Setting::Exp) == Multiplier::new(2, 1)
            && block_settings(&h) == easy.settings_bytes()
            && block_money(&h) == [2, 1],
        "Easy: enemy power x1/2, EXP x2, money x2 reach the block".to_owned(),
    );
    let zeni = pay(&mut h)?;
    checks.check(zeni == 180, format!("Easy: the event pays x2: {zeni} (180)"));
    // A value changed on the Cheats screen: the preset reads Custom.
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    checks.check(open_cheats(&mut h)?, "the Cheats screen opens".to_owned());
    goto(&mut h, Row::Exp)?;
    h.key(Key::Right)?;
    checks.check(
        preset(&h).is_none(),
        "EXP changed on the Cheats screen: the preset reads Custom".to_owned(),
    );
    h.key(Key::Left)?;
    checks.check(preset(&h) == Some(0), "and back: Easy again".to_owned());
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    let mut saved = Report::default();
    menu_test::menu_save(&mut h, &mut saved)?;
    let written = h.machine.sram().context("the cartridge has its save")?.to_vec();
    checks.check(
        saved.passed() && h.driver.marked_slots(&written).iter().filter(|&&m| m).count() == 1,
        "the save made on Easy has the CHEATS badge".to_owned(),
    );
    let checksum = slot_checksum(&written, 0);
    let kept = h.driver.option_slots().entry_for(0, checksum).cloned();
    checks.check(
        kept.as_ref().and_then(|v| v.get("difficulty.money")) == Some(2),
        "the slot file keeps the money step".to_owned(),
    );
    // Continue on that save: the money comes back with the cheats.
    let mut again = field_harness_with(
        rom,
        &written,
        0,
        h.driver.cheat_sidecar().clone(),
        Some(h.driver.option_slots().clone()),
    )?;
    again.frame(ssd_oracle::Joypad::default())?;
    checks.check(
        preset(&again) == Some(0) && block_money(&again) == [2, 1],
        "Continue restores Easy: the cheats from ssd.cheats, the money from ssd.options".to_owned(),
    );
    // Normal is clean: the original's numbers, no badge.
    let mut normal = field_harness(rom, sram, slot, CheatSidecar::empty())?;
    let zeni = pay(&mut normal)?;
    checks.check(zeni == 140, format!("Normal: the event pays x1: {zeni} (140)"));
    let mut plain = Report::default();
    menu_test::menu_save(&mut normal, &mut plain)?;
    let plain_sram = normal.machine.sram().context("the cartridge has its save")?.to_vec();
    checks.check(
        plain.passed() && normal.driver.marked_slots(&plain_sram) == [false; 3],
        "the save made on Normal has no badge".to_owned(),
    );
    Ok(checks.ok)
}

/// The Gameplay page through the keys: the text speed row writes `$01D0` (the row counts slowest first, the byte fastest
/// first), the auto-run switch, its wait and the animation speed are kept for the slot (Save writes them to `ssd.options`,
/// Continue restores them), a finished message that waits for A gets the host's A after the wait, and the animation speed
/// only speeds up the battle's attack animation.
fn screen_gameplay(rom: &[u8], sram: &[u8], slot: usize, out: &mut String) -> anyhow::Result<bool> {
    let mut checks = Checks { out, ok: true };
    let mut h = field_harness(rom, sram, slot, CheatSidecar::empty())?;
    checks.check(open_options(&mut h)?, "the Options screen opens from the Menu list".to_owned());
    h.key(Key::Tab)?;
    checks.check(
        h.driver.nav().options_category() == ssd_menus::options::Category::Gameplay,
        "R changes to the Gameplay page".to_owned(),
    );
    // Text speed: Right is one step faster, so the byte goes down by one (wrapping from the fastest to the slowest).
    let before = h.byte(d::TEXT_SPEED);
    h.key(Key::Right)?;
    let expected = if before == 0 { 4 } else { before - 1 };
    checks.check(
        h.byte(d::TEXT_SPEED) == expected,
        format!(
            "Text speed Right: the byte {before} becomes {} ({expected})",
            h.byte(d::TEXT_SPEED)
        ),
    );
    h.key(Key::Left)?;
    checks.check(h.byte(d::TEXT_SPEED) == before, "and Left puts it back".to_owned());
    // Auto-run On, wait Short, animation x2.
    h.key(Key::Down)?;
    h.key(Key::Right)?;
    h.key(Key::Down)?;
    h.key(Key::Left)?;
    h.key(Key::Down)?;
    h.key(Key::Right)?;
    let values = h.driver.options().game.clone();
    checks.check(
        values.get("gameplay.auto_run") == Some(1)
            && values.get("gameplay.auto_wait") == Some(0)
            && values.get("gameplay.anim_speed") == Some(1),
        "Auto-run On, wait Short and animation x2 are in the slot's values".to_owned(),
    );
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    // A finished message waits for A (the work RAM patched for the driver alone: the machine is not run).
    let mut wram = h.machine.wram().to_vec();
    wram[0x071B] = 0x83;
    let mut pressed_at = None;
    for frame in 0..200 {
        if h.driver.frame(&wram, None, ssd_oracle::Joypad::default()).pad.a {
            pressed_at = Some(frame);
            break;
        }
    }
    checks.check(
        pressed_at == Some(44),
        format!("a message that waits for A gets the host's A after the Short wait: frame {pressed_at:?} (44)"),
    );
    wram[0x071B] = 0x96;
    checks.check(
        !h.driver.frame(&wram, None, ssd_oracle::Joypad::default()).pad.a,
        "a timed message gets none".to_owned(),
    );
    // The animation speed: only the attack animation is sped up.
    wram[0x071B] = 0;
    wram[0x0364] = 0xFF;
    wram[0x24] = 4;
    wram[0xAB] = 4;
    checks.check(
        h.driver.animation_factor(&wram) == 2,
        "x2 in the battle's attack animation".to_owned(),
    );
    wram[0xAB] = 3;
    checks.check(h.driver.animation_factor(&wram) == 1, "and 1 in the other states".to_owned());
    // Save writes the three keys to the slot's entry; Continue restores them.
    let mut saved = Report::default();
    menu_test::menu_save(&mut h, &mut saved)?;
    let written = h.machine.sram().context("the cartridge has its save")?.to_vec();
    let checksum = slot_checksum(&written, 0);
    let kept = h.driver.option_slots().entry_for(0, checksum).cloned();
    checks.check(
        saved.passed()
            && kept.as_ref().is_some_and(|v| {
                v.get("gameplay.auto_run") == Some(1)
                    && v.get("gameplay.auto_wait") == Some(0)
                    && v.get("gameplay.anim_speed") == Some(1)
            }),
        "the Save keeps the three values in the slot's entry".to_owned(),
    );
    let again = field_harness_with(
        rom,
        &written,
        0,
        h.driver.cheat_sidecar().clone(),
        Some(h.driver.option_slots().clone()),
    )?;
    let restored = again.driver.options().game.clone();
    checks.check(
        restored.get("gameplay.auto_run") == Some(1)
            && restored.get("gameplay.anim_speed") == Some(1),
        "Continue restores them".to_owned(),
    );
    let mut other = field_harness(rom, sram, slot, CheatSidecar::empty())?;
    other.frame(ssd_oracle::Joypad::default())?;
    checks.check(
        other.driver.options().game.get("gameplay.auto_run").is_none(),
        "a slot without an entry starts with the defaults (Off, Normal)".to_owned(),
    );
    Ok(checks.ok)
}

/// The checksum word of save slot `slot` (0 to 2) in a battery save.
fn slot_checksum(sram: &[u8], slot: usize) -> u16 {
    let at = ssd_save::SLOT_BASES[slot] + ssd_save::CHECKSUM_OFFSET;
    u16::from_le_bytes([sram[at], sram[at + 1]])
}

/// A Save writes its slot's cheats also when the save's data is the same as the last one (the checksum word does not
/// change): the game's own Save twice into slot 1, with the Cheats screen between the two, the second with other
/// cheats. The slot's entry then holds the second choice, marked, and the sidecar keeps one entry for the slot.
fn screen_resave(rom: &[u8], sram: &[u8], slot: usize, out: &mut String) -> anyhow::Result<bool> {
    let mut checks = Checks { out, ok: true };
    let mut h = field_harness(rom, sram, slot, CheatSidecar::empty())?;
    checks.check(open_cheats(&mut h)?, "the Cheats screen opens from the Menu list".to_owned());
    goto(&mut h, Row::Damage)?;
    h.key(Key::Right)?;
    let first_cheats = *h.driver.cheats();
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    let mut first = Report::default();
    menu_test::menu_save(&mut h, &mut first)?;
    checks.check(first.passed(), "the game's Save runs to Completed the first time".to_owned());
    let first_sram = h.machine.sram().context("the cartridge has its save")?.to_vec();
    let checksum = slot_checksum(&first_sram, 0);

    checks.check(open_cheats(&mut h)?, "the Cheats screen opens again after the Save".to_owned());
    goto(&mut h, Row::RandomBattles)?;
    h.key(Key::Right)?;
    let second_cheats = *h.driver.cheats();
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    let mut second = Report::default();
    menu_test::menu_save(&mut h, &mut second)?;
    checks.check(second.passed(), "the game's Save runs to Completed the second time".to_owned());
    let second_sram = h.machine.sram().context("the cartridge has its save")?.to_vec();
    checks.check(
        slot_checksum(&second_sram, 0) == checksum,
        "the second Save writes the same checksum word (the data did not change)".to_owned(),
    );
    let entry = h.driver.cheat_sidecar().entry_for(0, checksum).copied();
    checks.check(
        entry.is_some_and(|saved| {
            saved.cheats == second_cheats && saved.marked && saved.cheats != first_cheats
        }),
        format!("the second Save writes its cheats to the slot's entry: {entry:?}"),
    );
    checks.check(
        h.driver.cheat_sidecar().saves.len() == 1,
        "the slot keeps one entry, replaced by the second Save".to_owned(),
    );
    checks.check(
        h.driver.marked_slots(&second_sram)[0],
        "the slot carries the CHEATS badge after the second Save".to_owned(),
    );
    Ok(checks.ok)
}

fn timeline_input(timeline: &[ssd_oracle::Joypad], frame: u64) -> ssd_oracle::Joypad {
    usize::try_from(frame).ok().and_then(|frame| timeline.get(frame)).copied().unwrap_or_default()
}

/// A recorded session with cheat events (format 2): the battle's inputs, the block at the first frame and a
/// settings change in the middle of the battle, played twice; both must end in the same work RAM.
fn recorded_session(rom: &[u8], replay: &Replay, out: &mut String) -> anyhow::Result<bool> {
    let mut all = Cheats::neutral();
    all.set_index(Setting::Damage, 2);
    all.set_index(Setting::Exp, 2);
    let mut session = replay.clone();
    session.mode = ssd_oracle::MODE_ENHANCED;
    session.events = vec![Vec::new(); session.inputs.len()];
    session.events[WRITE_AT as usize] = cheats::block_events(&Cheats::neutral());
    let middle = session.inputs.len() / 2;
    session.events[middle] = vec![cheats::settings_event(&all)];
    let bytes = session.to_bytes(CORE_VERSION);
    let read = Replay::from_bytes(&bytes, ssd_rom::crc32(rom), CORE_VERSION)?;
    let first = played_hash(rom, &read)?;
    let second = played_hash(rom, &read)?;
    let same = first == second && read == session;
    let _ = writeln!(
        out,
        "{} the recorded cheat session (format 2, {} bytes) plays twice to the same work RAM: {first:016x}",
        if same { "pass" } else { "FAIL" },
        bytes.len(),
    );
    Ok(same)
}

/// Plays a recorded session with the wrappers installed, its host events made before each frame.
fn played_hash(rom: &[u8], replay: &Replay) -> anyhow::Result<u64> {
    let mut machine = replay.power_on(rom.to_vec())?;
    let mut table = HookTable::new();
    cheats::install(&mut table);
    machine.set_hook_table(Some(table));
    let mut runs = 0_u64;
    while machine.frame_count() < replay.frames() {
        anyhow::ensure!(runs < replay.frames(), "the machine rendered no frame in {runs} runs");
        runs += 1;
        let frame = machine.frame_count();
        apply_events(&mut machine, replay.events_at(frame))?;
        machine.run_frame(replay.input(frame), &mut NullAudio)?;
    }
    Ok(full_hash(machine.wram()))
}

/// The group spike (M5 task 1): each `secret-*` replay played twice, from power-on, once in Classic and
/// once with the extreme battle cheats (the scripted-loss gate on `$57` only), printing each battle of
/// Classic beside the same battle of the cheat run: its group, the frame it ends and the map it leaves.
/// A scripted ending broken by a cheat shows as a battle that never ends, or a map that differs.
///
/// # Errors
///
/// A replay that cannot be read or played.
pub fn spike(rom: &[u8], dir: &Path, out: &mut String) -> anyhow::Result<()> {
    let mut extreme = Cheats::neutral();
    extreme.set_index(Setting::Damage, 10);
    extreme.set_index(Setting::Defence, 10);
    extreme.set_index(Setting::Exp, 9);
    extreme.set_index(Setting::Power, 5);
    extreme.set_index(Setting::Enemies, 3);
    let mut names: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("secret-"))
                && path.extension().is_some_and(|ext| ext == "ssdreplay")
        })
        .collect();
    names.sort();
    for path in names {
        let replay = read_replay(rom, &path)?;
        let name = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_owned();
        let classic = run_frames(
            &mut replay.power_on(rom.to_vec())?,
            &Plan::default(),
            |frame| replay.input(frame),
            replay.frames(),
        )?;
        let mut machine = replay.power_on(rom.to_vec())?;
        let mut table = HookTable::new();
        cheats::install(&mut table);
        machine.set_hook_table(Some(table));
        let plan = Plan { start: Some(extreme), ..Plan::default() };
        let cheat = run_frames(&mut machine, &plan, |frame| replay.input(frame), replay.frames())?;
        let _ = writeln!(
            out,
            "{name}: {} battles in Classic, {} with the cheats",
            classic.battles.len(),
            cheat.battles.len()
        );
        for (index, battle) in classic.battles.iter().enumerate() {
            let other = cheat.battles.get(index);
            let show = |b: Option<&Battle>| {
                b.map_or_else(
                    || "none".to_owned(),
                    |b| match b.end {
                        Some(end) => format!(
                            "group ${:02X} frames {}-{end} map {} mode {}",
                            b.group, b.start, b.map_after, b.mode_after
                        ),
                        None => format!("group ${:02X} frames {}-never", b.group, b.start),
                    },
                )
            };
            let _ = writeln!(out, "  Classic {} | cheats {}", show(Some(battle)), show(other));
        }
        let _ = writeln!(
            out,
            "  end: Classic map {} mode {}, cheats map {} mode {}",
            classic.final_map, classic.final_mode, cheat.final_map, cheat.final_mode,
        );
        // A fight that ends at another frame with the cheats leaves the recorded presses out of step with the story.
        // The recording is played on from the cheat run's end of the last fight, as the player would press the same
        // buttons then: the story should come to the same map, mode and party as Classic.
        let ends = classic
            .battles
            .last()
            .and_then(|b| b.end)
            .zip(cheat.battles.last().and_then(|b| b.end));
        if let Some((classic_end, cheat_end)) = ends
            && classic_end != cheat_end
            && (classic.final_map, classic.final_mode) != (cheat.final_map, cheat.final_mode)
        {
            let shift = |frame: u64| {
                if frame < cheat_end {
                    frame
                } else {
                    (frame + classic_end).saturating_sub(cheat_end)
                }
            };
            let delta = cheat_end.abs_diff(classic_end);
            let mut machine = replay.power_on(rom.to_vec())?;
            let mut table = HookTable::new();
            cheats::install(&mut table);
            machine.set_hook_table(Some(table));
            let plan = Plan { start: Some(extreme), ..Plan::default() };
            let frames = if cheat_end > classic_end {
                replay.frames() + delta
            } else {
                replay.frames() - delta
            };
            let moved =
                run_frames(&mut machine, &plan, |frame| replay.input(shift(frame)), frames)?;
            let last = |t: &Trace| t.snaps.last().map(|s| (s.ids, s.map, s.mode));
            let _ = writeln!(
                out,
                "  continued from the cheat run's end of the fight: map {} mode {}, party {}",
                moved.final_map,
                moved.final_mode,
                if last(&moved) == last(&classic) { "as Classic" } else { "DIFFERS from Classic" },
            );
        }
    }
    Ok(())
}

/// A scenario's report: its checks are lines of the report, and the scenario passes when all of them do.
struct Checks<'a> {
    out: &'a mut String,
    ok: bool,
}

impl Checks<'_> {
    fn check(&mut self, pass: bool, text: String) {
        self.ok &= pass;
        let _ = writeln!(self.out, "{} {text}", if pass { "pass" } else { "FAIL" });
    }

    fn note(&mut self, text: String) {
        let _ = writeln!(self.out, "  {text}");
    }
}

/// The reference replays and the runs they are played in, each run made once.
struct Runs<'a> {
    rom: &'a [u8],
    dir: &'a Path,
    traces: Vec<(String, Trace)>,
}

impl Runs<'_> {
    /// The trace of `replay` (a file of the directory) played with `plan` for `frames` frames, made once per `key`.
    fn get(&mut self, key: &str, replay: &str, plan: &Plan) -> anyhow::Result<Trace> {
        self.get_capped(key, replay, plan, u64::MAX)
    }

    /// The same, for the first `frames` frames of the replay at most.
    fn get_capped(
        &mut self,
        key: &str,
        replay: &str,
        plan: &Plan,
        frames: u64,
    ) -> anyhow::Result<Trace> {
        if let Some((_, trace)) = self.traces.iter().find(|(name, _)| name == key) {
            return Ok(trace.clone());
        }
        let replay_file = read_replay(self.rom, &self.dir.join(replay))?;
        let frames = frames.min(replay_file.frames());
        let trace = play_plan(self.rom, &replay_file, plan, None, frames)?;
        self.traces.push((key.to_owned(), trace.clone()));
        Ok(trace)
    }
}

/// The Z cards hand check. `from` is the snapshot where the choice was written: every card then in the hand has the
/// `corner` (3 attack, 4 defence) at `value`, and so has every card dealt afterwards, at the frame it appears (an
/// empty slot that gets a card). The battle's attack animation borrows the first two slots for the attacker's and the
/// defender's values (zeros for an enemy's), so a slot whose attack value is zero is not a card at that moment and is
/// not counted: a dealt card always has values from 1 to 8. Returns whether all of the checked cards show `value`, and
/// how many were checked.
fn hand_shows(trace: &Trace, from: u64, corner: usize, value: u8) -> (bool, usize) {
    let from = from as usize;
    let mut cards = 0;
    let mut all = true;
    if let Some(snap) = trace.snaps.get(from) {
        for card in 0..5 {
            let base = card * 6;
            if snap.hand[base] & 0x80 == 0 && snap.hand[base + 3] != 0 {
                cards += 1;
                all &= snap.hand[base + corner] == value;
            }
        }
    }
    for index in (from + 1)..trace.snaps.len() {
        let (prev, now) = (&trace.snaps[index - 1], &trace.snaps[index]);
        for card in 0..5 {
            let base = card * 6;
            if prev.hand[base] & 0x80 != 0 && now.hand[base] & 0x80 == 0 && now.hand[base + 3] != 0
            {
                cards += 1;
                all &= now.hand[base + corner] == value;
            }
        }
    }
    (all, cards)
}

/// The Item menu's Bulma on Goku, on the map, after the Continue of [`save::continue_prefix`]: the map menu's Item (the
/// third down from Talk), the first object (Bulma), Use and Goku as the target (`menus.txt` and the crafted save of
/// the `endless` scenario).
const USE_BULMA: &str = "\
4 a
60 -
4 down
30 -
4 down
30 -
4 down
30 -
4 a
60 -
4 a
60 -
4 a
60 -
4 a
60 -
600 -
";

/// The endless items (task 2): a save with two Bulma and Goku at 10 HP, from `<base>` (a first-chapter
/// save, `.work/demo/a-goku-level-10.sav`, a copy), used once from the Item menu. Classic leaves one Bulma and heals
/// Goku by 25; with the endless items both Bulma stay and the heal is the same. The crafted save is a scratch file.
fn endless(rom: &[u8], base: &Path, checks: &mut Checks<'_>) -> anyhow::Result<()> {
    let scratch = base.with_file_name("cheats-scratch");
    std::fs::create_dir_all(&scratch)?;
    let crafted = scratch.join("endless-bulma.sav");
    let (base_text, crafted_text) = (base.display().to_string(), crafted.display().to_string());
    let args = save::parse_edit_args(&[
        base_text.as_str(),
        crafted_text.as_str(),
        "--set",
        "items=2,2",
        "--set",
        "goku.hp=10",
    ])
    .map_err(anyhow::Error::msg)?;
    let game = save::Game::from_rom(rom)?;
    save::edit(&args, Some(&game), &mut std::io::sink())?;
    let sram = std::fs::read(&crafted);
    let _ = std::fs::remove_dir_all(&scratch);
    let sram = sram?;
    let continued = save::continue_prefix(0);
    let use_bulma =
        scripts::parse_timeline(&format!("{continued}{USE_BULMA}")).map_err(anyhow::Error::msg)?;
    let drop_bulma =
        scripts::parse_timeline(&format!("{continued}{DROP_BULMA}")).map_err(anyhow::Error::msg)?;
    // One timeline from the crafted save, with the cheats or without: the Bulma left in the list and Goku's HP.
    let run =
        |timeline: &[ssd_oracle::Joypad], cheats: Option<Cheats>| -> anyhow::Result<(u8, u16)> {
            let mut saves = MemorySaveStore::default();
            saves.0.insert("sav".to_owned(), sram.clone());
            let mut machine = Machine::new(rom.to_vec(), Box::new(saves))?;
            let plan = Plan { start: cheats, ..Plan::default() };
            if cheats.is_some() {
                let mut table = HookTable::new();
                cheats::install(&mut table);
                machine.set_hook_table(Some(table));
            }
            run_frames(
                &mut machine,
                &plan,
                |frame| timeline_input(timeline, frame),
                timeline.len() as u64,
            )?;
            let wram = machine.wram();
            Ok((wram[ITEM_COUNT], read16(wram, PARTY_0 + 0x03)))
        };
    let mut endless = Cheats::neutral();
    endless.set_endless(true);
    let (classic_count, classic_hp) = run(&use_bulma, None)?;
    let (count, hp) = run(&use_bulma, Some(endless))?;
    let (dropped_classic, _) = run(&drop_bulma, None)?;
    let (dropped, _) = run(&drop_bulma, Some(endless))?;
    checks.check(
        classic_count == 1 && classic_hp == 35,
        format!("Classic: Bulma is used once (1 left) and heals Goku 10 to 35: {classic_count} left, HP {classic_hp}"),
    );
    checks.check(
        count == 2 && hp == 35,
        format!("endless items: both Bulma stay (2) and the heal is the same (35): {count} left, HP {hp}"),
    );
    checks.check(
        dropped_classic == 1 && dropped == 1,
        format!("Drop still removes one Bulma (2 to 1) with the endless items on: Classic {dropped_classic}, endless {dropped}"),
    );
    Ok(())
}

/// The Item menu's Bulma dropped (the Drop entry and its Yes), after the same keys as [`USE_BULMA`]: Drop is not an
/// effect, so the endless items do not keep it.
const DROP_BULMA: &str = "\
4 a
60 -
4 down
30 -
4 down
30 -
4 down
30 -
4 a
60 -
4 a
60 -
4 down
30 -
4 a
60 -
4 a
60 -
600 -
";

/// The scenarios (`menu-test cheats`): the names given, or all of them; the scripted loss when a save is given.
fn scenarios(
    rom: &[u8],
    dir: &Path,
    save: Option<&(PathBuf, usize)>,
    only: &[String],
    out: &mut String,
) -> anyhow::Result<bool> {
    let wanted = |name: &str| only.is_empty() || only.iter().any(|o| o == name);
    let mut checks = Checks { out, ok: true };
    let mut runs = Runs { rom, dir, traces: Vec::new() };
    let battle = "battle.ssdreplay";
    let plan_with = |setting: &dyn Fn(&mut Cheats)| {
        let mut cheats = Cheats::neutral();
        setting(&mut cheats);
        Plan { start: Some(cheats), ..Plan::default() }
    };
    if wanted("battle") {
        let replay = dir.join(battle);
        checks.note("battle: the task 1 numbers on the battle replay".to_owned());
        let ok = battle_numbers(rom, &replay, 0, checks.out)?;
        checks.ok &= ok;
    }
    if wanted("z-party") {
        checks.note("z-party: the Z cards of the whole party on the battle replay".to_owned());
        let atk = runs.get("z-atk", battle, &plan_with(&|c| c.set_z_mode(ZMode::Attack)))?;
        let (shown, cards) = hand_shows(&atk, WRITE_AT, 3, Z_VALUE);
        checks.check(
            shown && cards > 0,
            format!("attack: every card dealt shows Z in the attack corner ({cards} card-frames)"),
        );
        checks.check(
            atk.blows.first().is_some_and(|b| b.amount == 29),
            format!(
                "attack: the first hit is 29 (19 without), it is {:?}",
                atk.blows.first().map(|b| b.amount)
            ),
        );
        let def = runs.get("z-def", battle, &plan_with(&|c| c.set_z_mode(ZMode::Defence)))?;
        let (shown, cards) = hand_shows(&def, WRITE_AT, 4, Z_VALUE);
        checks.check(
            shown && cards > 0,
            format!(
                "defence: every card dealt shows Z in the defence corner ({cards} card-frames)"
            ),
        );
        let both = runs.get("z-both", battle, &plan_with(&|c| c.set_z_mode(ZMode::Both)))?;
        let (attack, _) = hand_shows(&both, WRITE_AT, 3, Z_VALUE);
        let (defence, _) = hand_shows(&both, WRITE_AT, 4, Z_VALUE);
        checks.check(attack && defence, "both: every card shows Z in both corners".to_owned());
        let enemy_blows =
            |t: &Trace| t.blows.iter().filter(|b| !b.party).map(|b| b.amount).collect::<Vec<_>>();
        let party_blows =
            |t: &Trace| t.blows.iter().filter(|b| b.party).map(|b| b.amount).collect::<Vec<_>>();
        let neutral = runs.get("z-neutral", battle, &Plan::default())?;
        checks.note(format!(
            "blows, Classic: on the enemy {:?}, on the party {:?}",
            enemy_blows(&neutral),
            party_blows(&neutral),
        ));
        // The party's hits on the enemy: Goku's 19 becomes 29, and Piccolo's 15 becomes 26 with his Z (the formula:
        // (408 x 14 x 4 + 82) / 855); a defence Z changes only the hits on the party.
        checks.check(
            enemy_blows(&atk) == [29, 26] && enemy_blows(&both) == [29, 26],
            format!(
                "attack and both: the party's hits on the enemy are 29 and 26: {:?}",
                enemy_blows(&atk)
            ),
        );
        checks.check(
            enemy_blows(&def) == [19, 15],
            format!("defence: the hits on the enemy stay 19 and 15: {:?}", enemy_blows(&def)),
        );
        checks.note(format!(
            "blows on the party: Classic {:?}, attack {:?}, defence {:?} (the Saibaman's hit under the defence Z), both {:?}",
            party_blows(&neutral),
            party_blows(&atk),
            party_blows(&def),
            party_blows(&both),
        ));
    }
    if wanted("z-fighter") {
        checks.note("z-fighter: the Z cards of one fighter on the battle replay".to_owned());
        let plan = |id: u8| {
            let mut cheats = Cheats::neutral();
            cheats.set_z_mode(ZMode::Attack);
            cheats.set_z_target(ZTarget::Fighter(id));
            Plan { start: Some(cheats), ..Plan::default() }
        };
        let goku = runs.get("z-goku", battle, &plan(GOKU))?;
        let amounts: Vec<u32> = goku.blows.iter().filter(|b| !b.party).map(|b| b.amount).collect();
        checks.check(
            amounts.first() == Some(&29) && amounts.get(1) == Some(&15),
            format!("Goku's attack: his hit 29 (19 without), Piccolo's unchanged 15: {amounts:?}"),
        );
        let piccolo = runs.get("z-piccolo", battle, &plan(PICCOLO))?;
        let amounts: Vec<u32> =
            piccolo.blows.iter().filter(|b| !b.party).map(|b| b.amount).collect();
        checks.check(
            amounts.first() == Some(&19),
            format!("Piccolo's Z: Goku's hit unchanged 19, Piccolo's changed: {amounts:?}"),
        );
    }
    if wanted("z-fighter-defence") {
        checks.note(
            "z-fighter-defence: the defence of one fighter on the secret-raditz replay".to_owned(),
        );
        // Raditz's first hit lands on Piccolo (party record 1, id 2); Goku takes his hit of 2 a little later.
        let raditz = "secret-raditz.ssdreplay";
        let defence_of = |id: u8| {
            let mut cheats = Cheats::neutral();
            cheats.set_z_mode(ZMode::Defence);
            cheats.set_z_target(ZTarget::Fighter(id));
            Plan { start: Some(cheats), ..Plan::default() }
        };
        let neutral = runs.get("raditz-neutral", raditz, &Plan::default())?;
        let piccolo = runs.get("raditz-piccolo-def", raditz, &defence_of(PICCOLO))?;
        let goku = runs.get("raditz-goku-def", raditz, &defence_of(GOKU))?;
        // The party's hits in order: (character id, HP lost).
        let hits = |t: &Trace| -> Vec<(u8, u32)> {
            t.blows
                .iter()
                .filter(|b| b.party)
                .map(|b| (t.snaps[b.frame as usize].ids[b.index as usize], b.amount))
                .collect()
        };
        let (base, with_piccolo, with_goku) = (hits(&neutral), hits(&piccolo), hits(&goku));
        checks.check(
            base.first() == Some(&(PICCOLO, 16)),
            format!("Raditz's first hit, on Piccolo, is 16 without the cheat: {base:?}"),
        );
        checks.check(
            with_piccolo.first() == Some(&(PICCOLO, 7)) && with_piccolo.get(1) == base.get(1),
            format!("with Piccolo's defence Z it is 7, Goku's hit unchanged: {with_piccolo:?}"),
        );
        checks.check(
            with_goku.first() == Some(&(PICCOLO, 16)) && with_goku.contains(&(GOKU, 1)),
            format!(
                "with Goku's defence Z his hit goes 2 to 1 and Piccolo's stays 16: {with_goku:?}"
            ),
        );
    }
    if wanted("z-mid-game") {
        checks.note(
            "z-mid-game: the Z cards switched on during the landing replay's flight, the hand rewritten".to_owned(),
        );
        // The menu is field-only: the switch is made on the map, at the first map frame from 3000 with a card in hand
        // (during a battle the attack animation borrows the first two slots and puts its own values back after it).
        let classic = runs.get("landing-classic", "landing.ssdreplay", &Plan::default())?;
        let at = (3000..classic.snaps.len())
            .find(|&frame| {
                let snap = &classic.snaps[frame];
                snap.mode == 0 && (0..5).any(|card| snap.hand[card * 6] & 0x80 == 0)
            })
            .unwrap_or(3000);
        let mut cheats = Cheats::neutral();
        cheats.set_z_mode(ZMode::Both);
        let plan = Plan { start: None, changes: vec![(at as u64, cheats)], writes: Vec::new() };
        let trace = runs.get("z-mid", "landing.ssdreplay", &plan)?;
        let before = trace.snaps[at - 1].hand;
        let before_z = (0..5).all(|card| before[card * 6 + 3] == 8 && before[card * 6 + 4] == 8);
        let (shown, cards) = hand_shows(&trace, at as u64, 3, Z_VALUE);
        let (defence, _) = hand_shows(&trace, at as u64, 4, Z_VALUE);
        checks.check(!before_z, "the hand before the switch is not all Z".to_owned());
        checks.check(
            shown && defence && cards > 0,
            format!("switched on at map frame {at}, the cards in hand show Z in both corners ({cards} card-frames)"),
        );
    }
    if wanted("zeni") {
        checks.note("zeni: the max zeni on the shop replay, with a stand-in purchase".to_owned());
        let mut cheats = Cheats::neutral();
        cheats.set_zeni(true);
        let plan = Plan {
            start: Some(cheats),
            changes: Vec::new(),
            writes: vec![(
                5000,
                vec![HostEvent::Wram { address: 0x7E_01B4, bytes: 300_u16.to_le_bytes().to_vec() }],
            )],
        };
        // The first 7000 frames of the shop replay: the purchase's stand-in and the game's own gain at frame 6629.
        let trace = runs.get_capped("zeni", "shop.ssdreplay", &plan, 7000)?;
        let zeni = |frame: usize| trace.snaps[frame].zeni;
        checks.check(
            zeni(301) == 999,
            format!("zeni is 999 on the next frame after switching on: {}", zeni(301)),
        );
        checks.check(
            zeni(5000) == 300,
            format!("the stand-in purchase leaves 300 after its frame: {}", zeni(5000)),
        );
        checks.check(zeni(5001) == 999, format!("and 999 on the next frame: {}", zeni(5001)));
        checks.check(
            trace.snaps.iter().skip(5001).all(|snap| snap.zeni == 999),
            "and it stays 999 to frame 7000".to_owned(),
        );
    }
    if wanted("money") {
        checks.note(
            "money: the zeni a story event gives, scaled by the money multiplier (a battle pays none)".to_owned(),
        );
        let symbols = Symbols::load(SYMBOLS).map_err(anyhow::Error::msg)?;
        // The shop replay holds one such event (a gift of 40 zeni): the zeni it adds, in Classic and at each money step, with
        // the cheat wrapper on the game's own code and on the routines in Rust (the recompiled ones in a build with the
        // `recomp` feature).
        let shop = read_replay(rom, &dir.join("shop.ssdreplay"))?;
        let frames = 7000.min(shop.frames());
        let classic = play_plan(rom, &shop, &Plan::default(), None, frames)?;
        let rise =
            (6000..frames as usize).find(|&f| classic.snaps[f].zeni > classic.snaps[f - 1].zeni);
        let Some(rise) = rise else {
            checks.check(false, "the shop replay has no zeni gain after frame 6000".to_owned());
            return Ok(checks.ok);
        };
        let gain = |trace: &Trace| {
            i32::from(trace.snaps[rise + 2].zeni) - i32::from(trace.snaps[rise - 1].zeni)
        };
        checks.check(
            gain(&classic) == 40,
            format!("Classic: the event adds 40 at frame {rise}: {}", gain(&classic)),
        );
        for (label, step, want) in [("x1", 1_usize, 40_i32), ("x2", 2, 80), ("x1/2", 0, 20)] {
            let mut cheats = Cheats::neutral();
            cheats.set_money_index(step);
            let plan = Plan { start: Some(cheats), ..Plan::default() };
            for (how, table) in [("original code", None), ("routines in Rust", Some(&symbols))] {
                let trace = play_plan(rom, &shop, &plan, table, frames)?;
                checks.check(
                    gain(&trace) == want,
                    format!("{label}, {how}: the event adds {want}: {}", gain(&trace)),
                );
            }
        }
        // The same event staged on the battle replay's machine (the original code): 40 added to 100 zeni.
        let replay = read_replay(rom, &dir.join(battle))?;
        for (label, step, want) in [("x1", 1_usize, 140_u16), ("x2", 2, 180), ("x1/2", 0, 120)] {
            let mut cheats = Cheats::neutral();
            cheats.set_money_index(step);
            let plan = Plan { start: Some(cheats), ..Plan::default() };
            let (zeni, amount) = money_event(rom, &replay, &plan, None)?;
            checks.check(
                zeni == want,
                format!("{label}, staged: 40 added to 100 zeni gives {want}: {zeni} (the amount reads {amount})"),
            );
        }
    }
    if wanted("gain") {
        checks.note(
            "gain: the zeni a story event gives, scaled by the zeni gained and by the money multiplier".to_owned(),
        );
        // The event staged on the battle replay's machine (the original code): 40 added to 100 zeni.
        let replay = read_replay(rom, &dir.join(battle))?;
        for (label, gain, money, want) in [
            ("x1", 0_usize, 1_usize, 140_u16),
            ("x5", 3, 1, 300),
            ("x5 with money x2", 3, 2, 500),
            ("x10 with money x1/2", 4, 0, 300),
            ("x10 with money x2: the cap", 4, 2, 900),
        ] {
            let mut cheats = Cheats::neutral();
            cheats.set_gain_index(gain);
            cheats.set_money_index(money);
            let plan = Plan { start: Some(cheats), ..Plan::default() };
            let (zeni, _) = money_event(rom, &replay, &plan, None)?;
            checks
                .check(zeni == want, format!("{label}: 40 added to 100 zeni gives {want}: {zeni}"));
        }
    }
    if wanted("sale") {
        checks.note(
            "sale: the shop's sale adds the price times the zeni gained, capped at 999".to_owned(),
        );
        // MrPopo (object 3) sells for 10 Pt.
        let replay = read_replay(rom, &dir.join(battle))?;
        let classic = sale_event(rom, &replay, &Plan::default(), 3, 100)?;
        checks.check(
            classic == 110,
            format!("Classic: selling MrPopo from 100 zeni gives 110: {classic}"),
        );
        for (label, gain, from, want) in [
            ("x1", 0_usize, 100_u16, 110_u16),
            ("x5", 3, 100, 150),
            ("x10", 4, 100, 200),
            ("x10, the cap", 4, 950, 999),
        ] {
            let mut cheats = Cheats::neutral();
            cheats.set_gain_index(gain);
            let plan = Plan { start: Some(cheats), ..Plan::default() };
            let zeni = sale_event(rom, &replay, &plan, 3, from)?;
            checks.check(
                zeni == want,
                format!("{label}: selling MrPopo from {from} zeni gives {want}: {zeni}"),
            );
        }
    }
    if wanted("deadline") {
        checks.note(
            "deadline: Piccolo at level 9 on map 5 kills the recruits in Classic, not in Remastered".to_owned(),
        );
        let replay = read_replay(rom, &dir.join(battle))?;
        let enhanced = Plan { start: Some(Cheats::neutral()), ..Plan::default() };
        for (label, plan, level, set) in [
            ("Classic, level 8", Plan::default(), 8_u8, false),
            ("Classic, level 9", Plan::default(), 9, true),
            ("Remastered, level 9", enhanced.clone(), 9, false),
            ("Remastered, level 30", enhanced, 30, false),
        ] {
            let flags = deadline_event(rom, &replay, &plan, level)?;
            checks.check(
                (flags & 0x20 != 0) == set,
                format!(
                    "{label}: the deadline's flag 0x1D is {}: $0173 = {flags:02X}",
                    if set { "set" } else { "clear" }
                ),
            );
        }
    }
    if wanted("random-off") {
        checks.note(
            "random-off: random battles off on the landing replay and the Frieza replays"
                .to_owned(),
        );
        let classic = runs.get("landing-classic", "landing.ssdreplay", &Plan::default())?;
        let mut cheats = Cheats::neutral();
        cheats.set_random_battles_off(true);
        let off = Plan { start: Some(cheats), ..Plan::default() };
        let landing = runs.get("landing-off", "landing.ssdreplay", &off)?;
        checks.note(format!(
            "landing: Classic battles at {:?}",
            classic.battles.iter().map(|b| b.start).collect::<Vec<_>>()
        ));
        checks.check(
            landing.battles.is_empty(),
            format!(
                "no random battle over the whole landing replay ({} in Classic)",
                classic.battles.len()
            ),
        );
        checks.check(
            classic.battles.iter().any(|b| b.start == 5430),
            "Classic fights at the roll of frame 5430 that the cheat passes".to_owned(),
        );
        // The story's Frieza battle: its group and the frame it starts (the roll comes a few frames before it).
        for (name, start) in [
            ("secret-frieza-forced-battle.ssdreplay", 2501),
            ("secret-frieza-random-encounter.ssdreplay", 2733),
        ] {
            let base = runs.get(&format!("{name}-classic"), name, &Plan::default())?;
            let cheat = runs.get(&format!("{name}-off"), name, &off)?;
            let story = |t: &Trace| {
                t.battles
                    .iter()
                    .find(|b| FRIEZA_GROUPS.contains(&b.group))
                    .map(|b| (b.start, b.group))
            };
            checks.check(
                story(&base).map(|s| s.0) == Some(start) && story(&cheat) == story(&base),
                format!(
                    "{name}: the story battle starts at frame {start} in Classic and with random battles off, the same group: {:?}",
                    story(&cheat)
                ),
            );
            let battles: Vec<(u64, u8)> =
                cheat.battles.iter().map(|b| (b.start, b.group)).collect();
            checks.check(
                cheat.battles.iter().all(|b| FRIEZA_GROUPS.contains(&b.group)),
                format!("{name}: with random battles off only story groups start: {battles:?}"),
            );
        }
    }
    if wanted("encounter-rate") {
        checks.note(
            "encounter-rate: the rolls on the landing replay, the battles on the story replay (48036 frames)"
                .to_owned(),
        );
        let rate = |num: u8, den: u8| {
            let mut cheats = Cheats::neutral();
            let index = Setting::Encounters
                .steps()
                .iter()
                .position(|m| m.num == num && m.den == den)
                .unwrap_or(2);
            cheats.set_index(Setting::Encounters, index);
            Plan { start: Some(cheats), ..Plan::default() }
        };
        // The rolls: the reloads of the encounter timer over the landing replay's flight.
        let classic = runs.get("landing-classic", "landing.ssdreplay", &Plan::default())?;
        let x2 = runs.get("landing-x2", "landing.ssdreplay", &rate(2, 1))?;
        let x4 = runs.get("landing-x4", "landing.ssdreplay", &rate(4, 1))?;
        // The rolls of the walk from frame 5000 on (the flight's first roll, at 4136, is a walk of its own).
        let from =
            |t: &Trace| t.rolls.iter().copied().filter(|&frame| frame >= 5000).collect::<Vec<_>>();
        let (classic_rolls, x2_rolls, x4_rolls) = (from(&classic), from(&x2), from(&x4));
        checks.check(
            classic_rolls.starts_with(&[5070, 5190, 5310]),
            format!(
                "Classic: a roll every 120 frames, at {:?}",
                &classic_rolls[..classic_rolls.len().min(4)]
            ),
        );
        checks.check(
            x2_rolls.starts_with(&[5070, 5130, 5190]),
            format!("x2: a roll every 60 frames, at {:?}", &x2_rolls[..x2_rolls.len().min(4)]),
        );
        let gaps: Vec<u64> = x4_rolls.windows(2).map(|w| w[1] - w[0]).collect();
        checks.check(
            x4_rolls.starts_with(&[5070, 5100, 5130, 5160]) && gaps.iter().all(|&gap| gap == 30),
            format!("x4: a roll every 30 frames, at {:?}", &x4_rolls[..x4_rolls.len().min(4)]),
        );
        // The battles: the story replay, its flights and walks with the random battles of the first two islands.
        let story = "story-1.ssdreplay";
        let base = runs.get("story-classic", story, &Plan::default())?;
        let half = runs.get("story-half", story, &rate(1, 2))?;
        let quarter = runs.get("story-quarter", story, &rate(1, 4))?;
        // The golden counts of the story's 48036 frames: Classic fights 11 battles; the generator keeps 6 of the rolls at
        // x1/2 and 4 at x1/4 (the first battle, at 5430, is kept by x1/2 and not by x1/4).
        let starts = |t: &Trace| t.battles.iter().map(|b| b.start).collect::<Vec<_>>();
        checks.check(
            base.battles.len() == 11 && half.battles.len() == 6 && quarter.battles.len() == 4,
            format!(
                "story battles: Classic {}, x1/2 {}, x1/4 {}: {:?} / {:?} / {:?}",
                base.battles.len(),
                half.battles.len(),
                quarter.battles.len(),
                starts(&base),
                starts(&half),
                starts(&quarter),
            ),
        );
        checks.check(
            starts(&half).first() == Some(&5430) && starts(&quarter).first() == Some(&11202),
            format!(
                "the first battle kept is 5430 at x1/2 and 11202 at x1/4: {:?}, {:?}",
                starts(&half).first(),
                starts(&quarter).first()
            ),
        );
    }
    if wanted("endless") {
        checks.note(
            "endless: Bulma used on the map from a crafted save (Classic, then endless items)"
                .to_owned(),
        );
        let base = dir.join("..").join("demo").join("a-goku-level-10.sav");
        if base.exists() {
            endless(rom, &base, &mut checks)?;
        } else {
            checks.note(format!("skipped: no base save at {}", base.display()));
        }
    }
    if wanted("recorded") {
        checks.note(
            "recorded: two format 2 sessions with Z both and random battles off replay to one hash"
                .to_owned(),
        );
        let mut cheats = Cheats::neutral();
        cheats.set_z_mode(ZMode::Both);
        cheats.set_random_battles_off(true);
        let plan = Plan { start: Some(cheats), ..Plan::default() };
        for replay in ["battle.ssdreplay", "landing.ssdreplay"] {
            let read = read_replay(rom, &dir.join(replay))?;
            let live = play_plan(rom, &read, &plan, None, read.frames())?;
            let mut session = read.clone();
            session.mode = MODE_ENHANCED;
            session.events.clone_from(&live.events);
            let bytes = session.to_bytes(CORE_VERSION);
            let back = Replay::from_bytes(&bytes, ssd_rom::crc32(rom), CORE_VERSION)?;
            let first = played_hash(rom, &back)?;
            let second = played_hash(rom, &back)?;
            checks.check(
                first == second && first == live.final_hash && back == session,
                format!(
                    "{replay}: the session ({} bytes) replays to the hash of its live run, twice: {first:016x}",
                    bytes.len()
                ),
            );
        }
    }
    if let Some((path, slot)) = save.filter(|_| wanted("scripted-loss")) {
        let ok = scripted_loss(rom, path, *slot, checks.out)?;
        checks.ok &= ok;
    }
    Ok(checks.ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_leaves_the_block_out() {
        let mut wram = vec![0_u8; 0x20000];
        let base = hash_outside_block(&wram);
        wram[(cheats::BLOCK - WRAM_BASE) as usize] = 9;
        assert_eq!(hash_outside_block(&wram), base, "the block is not compared");
        wram[0x14CF] = 1;
        assert_ne!(hash_outside_block(&wram), base);
        let mut stack = vec![0_u8; 0x20000];
        stack[0x0FC5] = 0x7F;
        assert_eq!(hash_outside_block(&stack), base, "the dead stack is not compared");
        stack[0x0F7F] = 0x7F;
        assert_ne!(hash_outside_block(&stack), base, "the bytes below it are");
    }

    #[test]
    fn the_first_hit_and_the_award_are_read_from_the_work_ram() {
        let mut wram = vec![0_u8; 0x20000];
        let mut trace = Trace::default();
        trace.observe(0, &wram);
        assert!(trace.hit.is_none());
        wram[DAMAGE] = 38;
        wram[SLOT_0 + 0x0A] = 55;
        wram[SLOT_0 + 0x03] = 55;
        wram[EXP_AWARD] = 27;
        wram[ENEMY_COUNT] = 2;
        trace.observe(1, &wram);
        wram[SLOT_0 + 0x03] = 17;
        trace.observe(2, &wram);
        let hit = trace.hit.expect("a hit");
        assert_eq!((hit.damage, hit.max_hp, hit.hp_after), (38, 55, Some(17)));
        assert_eq!((trace.exp_award, trace.enemies), (27, 2));
    }

    #[test]
    fn a_roll_is_a_reload_of_the_encounter_timer_and_a_blow_an_hp_loss_in_battle() {
        let mut wram = vec![0_u8; 0x20000];
        let mut trace = Trace::default();
        trace.observe(0, &wram);
        wram[TIMER] = 59;
        trace.observe(1, &wram);
        assert_eq!(trace.rolls, vec![1]);
        wram[MODE] = 2;
        wram[SLOT_0] = 0x40;
        wram[SLOT_0 + 3] = 55;
        trace.observe(2, &wram);
        wram[SLOT_0 + 3] = 36;
        trace.observe(3, &wram);
        assert_eq!(trace.blows, vec![Blow { frame: 3, party: false, index: 0, amount: 19 }]);
    }
}
