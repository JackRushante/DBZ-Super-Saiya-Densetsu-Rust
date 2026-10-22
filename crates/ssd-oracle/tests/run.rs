//! The run loop, `Machine::run`: its stops, and goldens of `run_frame`, `run_frame_with`,
//! `run_until_call`, `run_until_sync` and `run_lockstep` on the synthetic fixtures, captured
//! before they became wrappers of `run`.
//!
//! Each golden is the CRC32 of the machine's `save_state()` after a fixed run and the CRC32 of a
//! text: what the calls returned (frame counts, call results, sync events) followed by the
//! `hook_stats()` (Debug format). A change of either is a change of behaviour.

mod common;

use common::{
    CALL_FLAT, CALL_INNER, CALL_INNER_COUNTER, CALL_INNER_LONG, CALL_LOOP, CALL_OUTER,
    CALL_OUTER_COUNTER, CALL_OUTER_LONG, DEEP, DEEP_COST, DEEP_IDLE, DEEP_LEAF, DEEP_LEAF_COST,
    DEEP_LEAF_COUNTER, LEAF, NESTED_INNER, NESTED_INNER_COUNTER, NESTED_OUTER,
    NESTED_OUTER_COUNTER, NESTED_OUTER_TAIL, NESTED_WRITES, RACE_IDLE, RACE_LEAF, RACE_LEAF_COST,
    SYNC_IDLE, deep_in_rust, race_leaf,
};
use ssd_oracle::{
    Ctx, HookAction, HookEntry, HookMode, HookStats, HookTable, Joypad, LockstepConfig, Machine,
    MemorySaveStore, NullAudio, Replay, ReturnKind, RoutineFn, Stopped, Stops, run_lockstep,
};
use std::fmt::Write as _;

// CPU-busy master cycles of the fixtures' instructions (8-cycle `SlowROM` and WRAM accesses, PPU
// registers and internal cycles 6).

const JSR: u64 = 46;
const JSL: u64 = 62;
const JMP: u64 = 24;
const INC_DP: u64 = 38;
const RTS: u64 = 42;
const RTL: u64 = 44;
const LDA_IMM: u64 = 16;
const STA_PPU: u64 = 30;
const PHX: u64 = 30;
const PLX: u64 = 36;
const LDY_IMM: u64 = 24;
const DEY: u64 = 14;
const BNE_TAKEN: u64 = 22;
const BNE_NOT_TAKEN: u64 = 16;

/// `inner` of [`common::call_rom`]: `INC $10; RTS`.
const CALL_INNER_COST: u64 = INC_DP + RTS;
const CALL_OUTER_COST: u64 = JSR + CALL_INNER_COST + INC_DP + RTS;
const CALL_FLAT_COST: u64 = 2 * INC_DP + RTS;
const CALL_OUTER_LONG_COST: u64 = JSL + INC_DP + RTL + INC_DP + RTS;
/// `inner` of [`common::nested_rom`].
const NESTED_INNER_COST: u64 = PHX
    + INC_DP
    + LDA_IMM
    + STA_PPU
    + LDY_IMM
    + 16 * DEY
    + 15 * BNE_TAKEN
    + BNE_NOT_TAKEN
    + PLX
    + RTS;
const NESTED_OUTER_COST: u64 = LDA_IMM + STA_PPU + JSR + NESTED_INNER_COST + STA_PPU + INC_DP + RTS;
const NESTED_OUTER_TAIL_COST: u64 = INC_DP + JMP + NESTED_INNER_COST;
/// The main loop of [`common::nested_rom`] (`LDX $10`), used as its sync point.
const NESTED_LOOP: u32 = 0x00_8016;

/// Frames the frame loops run.
const FRAMES: u64 = 20;
/// Calls `run_until_call` is asked for in each of its [`ROUNDS`], and events `run_until_sync`.
const CALLS: usize = 100;
const ROUNDS: usize = 3;
const EVENTS: usize = 20;
/// The budget of each `run_until_call` and `run_until_sync`.
const BUDGET: u64 = 3;
/// Unused ROM of every fixture: no call ever arrives there, so `run_until_call` runs its budget.
const NEVER_CALLED: u32 = 0x00_9000;
/// Frames of the lockstep replays.
const LOCKSTEP_FRAMES: u64 = 120;

/// The joypad of `frame`: A on every third frame, Right on two frames of five.
fn input(frame: u64) -> Joypad {
    Joypad { a: frame % 3 == 1, right: frame % 5 < 2, ..Joypad::default() }
}

// The routines in Rust

fn routine(
    name: &str,
    key: u32,
    mode: HookMode,
    func: RoutineFn,
    ret: ReturnKind,
    cycles: Option<u64>,
    verify_sample: u32,
) -> HookEntry {
    HookEntry {
        name: name.to_owned(),
        key,
        mode,
        action: HookAction::Routine { func, ret },
        entry_m8: Some(true),
        entry_x8: Some(false),
        flags_mask: 0xFF,
        verify_sample,
        cycles,
        bound: None,
        budget_frames: None,
    }
}

fn rts(name: &str, key: u32, mode: HookMode, func: RoutineFn, cycles: u64) -> HookEntry {
    routine(name, key, mode, func, ReturnKind::Rts, Some(cycles), 1)
}

fn inc_dp(ctx: &mut Ctx<'_>, offset: usize) {
    let value = ctx.read_dp8(offset as u16).wrapping_add(1);
    ctx.write_dp8(offset as u16, value);
    ctx.set_nz8(value);
}

/// `outer` of [`common::call_rom`], running `inner` as original code.
fn call_outer(ctx: &mut Ctx<'_>) {
    ctx.charge(JSR);
    let Ok(()) = ctx.call_original(CALL_INNER, ReturnKind::Rts) else { return };
    inc_dp(ctx, CALL_OUTER_COUNTER);
    ctx.set_cost(JSR + INC_DP + RTS);
}

/// `flat` of [`common::call_rom`] through `inner`.
fn call_flat(ctx: &mut Ctx<'_>) {
    let Ok(()) = ctx.call_original(CALL_INNER, ReturnKind::Rts) else { return };
    inc_dp(ctx, CALL_OUTER_COUNTER);
    ctx.set_cost(CALL_FLAT_COST - CALL_INNER_COST);
}

/// `outer_long` of [`common::call_rom`], running `inner_long` as original code.
fn call_outer_long(ctx: &mut Ctx<'_>) {
    ctx.charge(JSL);
    let Ok(()) = ctx.call_original(CALL_INNER_LONG, ReturnKind::Rtl) else { return };
    inc_dp(ctx, CALL_OUTER_COUNTER);
    ctx.set_cost(JSL + INC_DP + RTS);
}

fn call_inner(ctx: &mut Ctx<'_>) {
    inc_dp(ctx, CALL_INNER_COUNTER);
    ctx.set_cost(CALL_INNER_COST);
}

/// `inner` of [`common::nested_rom`]: X kept, so N and Z come from it.
fn nested_inner_work(ctx: &mut Ctx<'_>) {
    let value = ctx.read_dp8(NESTED_INNER_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(NESTED_INNER_COUNTER as u16, value);
    let [_, (coldata, color), _] = NESTED_WRITES;
    ctx.write8(coldata, color);
    ctx.set_al(color);
    ctx.set_y(0);
    let x = ctx.x();
    ctx.set_nz16(x);
}

fn nested_inner(ctx: &mut Ctx<'_>) {
    nested_inner_work(ctx);
    ctx.set_cost(NESTED_INNER_COST);
}

/// `outer` of [`common::nested_rom`] with `inner` inlined.
fn nested_outer(ctx: &mut Ctx<'_>) {
    let [(inidisp, brightness), _, (coldata, color)] = NESTED_WRITES;
    ctx.write8(inidisp, brightness);
    nested_inner_work(ctx);
    ctx.write8(coldata, color);
    inc_dp(ctx, NESTED_OUTER_COUNTER);
    ctx.set_cost(NESTED_OUTER_COST);
}

fn nested_outer_tail(ctx: &mut Ctx<'_>) {
    inc_dp(ctx, NESTED_OUTER_COUNTER);
    nested_inner_work(ctx);
    ctx.set_cost(NESTED_OUTER_TAIL_COST);
}

fn deep_leaf(ctx: &mut Ctx<'_>) {
    inc_dp(ctx, DEEP_LEAF_COUNTER);
    ctx.set_cost(DEEP_LEAF_COST);
}

// The fixtures

/// A synthetic ROM with its hook table: sync points, and routines in `Rust` and `Verify` mode.
struct Fixture {
    name: &'static str,
    rom: fn() -> Vec<u8>,
    table: fn() -> HookTable,
    /// The routine (or sync point) `run_until_call` waits for.
    call: u32,
}

fn sync_point(key: u32) -> HookEntry {
    HookEntry::sync_point("sync", key)
}

const FIXTURES: [Fixture; 6] = [
    Fixture {
        name: "sync_idle",
        rom: common::sync_idle_rom,
        table: || [sync_point(SYNC_IDLE)].into_iter().collect(),
        call: SYNC_IDLE,
    },
    Fixture {
        name: "nmi_race",
        rom: common::nmi_race_rom,
        table: || {
            let leaf = routine(
                "leaf",
                RACE_LEAF,
                HookMode::Verify,
                race_leaf,
                ReturnKind::Rts,
                Some(RACE_LEAF_COST),
                3,
            );
            [sync_point(RACE_IDLE), leaf].into_iter().collect()
        },
        call: RACE_LEAF,
    },
    Fixture {
        name: "hdma_idle",
        rom: common::hdma_idle_rom,
        table: || {
            let leaf = rts("leaf", RACE_LEAF, HookMode::Rust, race_leaf, RACE_LEAF_COST);
            [sync_point(RACE_IDLE), leaf].into_iter().collect()
        },
        call: RACE_LEAF,
    },
    Fixture {
        name: "call",
        rom: || common::call_rom(true),
        table: || {
            [
                sync_point(CALL_LOOP),
                rts("outer", CALL_OUTER, HookMode::Rust, call_outer, CALL_OUTER_COST),
                rts("flat", CALL_FLAT, HookMode::Verify, call_flat, CALL_FLAT_COST),
                rts(
                    "outer_long",
                    CALL_OUTER_LONG,
                    HookMode::Rust,
                    call_outer_long,
                    CALL_OUTER_LONG_COST,
                ),
                rts("inner", CALL_INNER, HookMode::Verify, call_inner, CALL_INNER_COST),
            ]
            .into_iter()
            .collect()
        },
        call: CALL_OUTER,
    },
    Fixture {
        name: "nested",
        rom: || common::nested_rom(true),
        table: || {
            let verify = |name, key, func, cycles| {
                routine(name, key, HookMode::Verify, func, ReturnKind::Rts, Some(cycles), 2)
            };
            [
                sync_point(NESTED_LOOP),
                verify("outer", NESTED_OUTER, nested_outer, NESTED_OUTER_COST),
                verify("outer_tail", NESTED_OUTER_TAIL, nested_outer_tail, NESTED_OUTER_TAIL_COST),
                verify("inner", NESTED_INNER, nested_inner, NESTED_INNER_COST),
            ]
            .into_iter()
            .collect()
        },
        call: NESTED_INNER,
    },
    Fixture {
        name: "deep_stack",
        rom: common::deep_stack_rom,
        table: || {
            [
                sync_point(DEEP_IDLE),
                rts("deep", DEEP, HookMode::Rust, deep_in_rust, DEEP_COST),
                rts("leaf", DEEP_LEAF, HookMode::Verify, deep_leaf, DEEP_LEAF_COST),
            ]
            .into_iter()
            .collect()
        },
        call: DEEP_LEAF,
    },
];

fn machine(fixture: &Fixture) -> Machine {
    let mut m = Machine::new((fixture.rom)(), Box::new(MemorySaveStore::default())).unwrap();
    m.set_hook_table(Some((fixture.table)()));
    m
}

/// The hook counters as text (Debug format), without `sync_inside_dispatch`, the counters of
/// declared instructions and `window_fit`, which came after the goldens were captured: no fixture
/// reaches a sync point inside a dispatch or declares its instructions.
fn stats_text(stats: &[(String, HookStats)]) -> String {
    for (name, s) in stats {
        assert_eq!(s.sync_inside_dispatch, 0, "{name}: {s:?}");
        assert_eq!((s.declared_runs, s.instructions_mismatch), (0, 0), "{name}: {s:?}");
    }
    let text = format!("{stats:?}")
        .replace(", sync_inside_dispatch: 0 }", " }")
        .replace(", declared_runs: 0, declared_instructions: 0.0, instructions_mismatch: 0", "");
    without_window_fit(&text)
}

/// `text` without the `window_fit: N, ` counters, which came after the goldens were captured.
fn without_window_fit(text: &str) -> String {
    let mut rest = text;
    let mut out = String::new();
    while let Some(at) = rest.find("window_fit: ") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "window_fit: ".len()..];
        let digits = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
        // Drop the counter and the ", " after it.
        rest = after[digits..].strip_prefix(", ").unwrap_or(&after[digits..]);
    }
    out.push_str(rest);
    out
}

/// The golden of a run that left `m` after returning `results`.
fn golden(fixture: &str, m: &Machine, mut results: String) -> Golden {
    let _ =
        write!(results, "\nmaster cycles {}, NMIs {}\n", m.total_master_cycles(), m.nmi_count());
    results.push_str(&stats_text(&m.hook_stats()));
    Golden {
        fixture: fixture.to_owned(),
        state: ssd_rom::crc32(&m.save_state()),
        text: ssd_rom::crc32(results.as_bytes()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Golden {
    fixture: String,
    state: u32,
    text: u32,
}

/// Compares `actual` with `expected` (fixture, state CRC, text CRC); on a mismatch, panics with
/// every actual golden, ready to paste.
fn check(wrapper: &str, expected: &[(&str, u32, u32)], actual: &[Golden]) {
    let expected: Vec<Golden> = expected
        .iter()
        .map(|&(fixture, state, text)| Golden { fixture: fixture.to_owned(), state, text })
        .collect();
    if expected != actual {
        let mut lines = String::new();
        for g in actual {
            let _ =
                writeln!(lines, "    (\"{}\", 0x{:08X}, 0x{:08X}),", g.fixture, g.state, g.text);
        }
        panic!("{wrapper}: the goldens changed; actual:\n{lines}");
    }
}

#[test]
fn run_frame_goldens() {
    let actual: Vec<Golden> = FIXTURES
        .iter()
        .map(|fixture| {
            let mut m = machine(fixture);
            let mut results = String::new();
            for frame in 0..FRAMES {
                m.run_frame(input(frame), &mut NullAudio).unwrap();
                let _ = write!(results, "{} ", m.frame_count());
            }
            golden(fixture.name, &m, results)
        })
        .collect();
    check("run_frame", &RUN_FRAME, &actual);
}

#[test]
fn run_frame_with_goldens() {
    let actual: Vec<Golden> = FIXTURES
        .iter()
        .map(|fixture| {
            let mut m = machine(fixture);
            let mut results = String::new();
            while m.frame_count() < FRAMES {
                m.run_frame_with(&mut input, &mut NullAudio).unwrap();
                let _ = write!(results, "{} ", m.frame_count());
            }
            golden(fixture.name, &m, results)
        })
        .collect();
    check("run_frame_with", &RUN_FRAME_WITH, &actual);
}

#[test]
fn run_until_call_goldens() {
    let actual: Vec<Golden> = FIXTURES
        .iter()
        .map(|fixture| {
            let mut m = machine(fixture);
            let mut results = String::new();
            let mut call = |m: &mut Machine, address, budget| {
                let called = m.run_until_call(address, &mut input, &mut NullAudio, budget).unwrap();
                let _ =
                    write!(results, "{called}@{}:{} ", m.frame_count(), m.total_master_cycles());
            };
            call(&mut m, fixture.call, 0);
            for _ in 0..ROUNDS {
                for _ in 0..CALLS {
                    call(&mut m, fixture.call, BUDGET);
                }
                call(&mut m, NEVER_CALLED, BUDGET);
            }
            golden(fixture.name, &m, results)
        })
        .collect();
    check("run_until_call", &RUN_UNTIL_CALL, &actual);
}

#[test]
fn run_until_sync_goldens() {
    let actual: Vec<Golden> = FIXTURES
        .iter()
        .map(|fixture| {
            let mut m = machine(fixture);
            let mut results = String::new();
            let none = m.run_until_sync(&mut input, &mut NullAudio, 0).unwrap();
            let _ = writeln!(results, "{none:?} at frame {}", m.frame_count());
            for _ in 0..EVENTS {
                let event = m.run_until_sync(&mut input, &mut NullAudio, BUDGET).unwrap();
                let _ = writeln!(results, "{event:?} at frame {}", m.frame_count());
            }
            golden(fixture.name, &m, results)
        })
        .collect();
    check("run_until_sync", &RUN_UNTIL_SYNC, &actual);
}

/// The lockstep fixtures: B's routines in `Rust` (lockstep refuses `Verify`), the sync point.
fn lockstep_fixtures() -> [(&'static str, Vec<u8>, HookTable, u32); 3] {
    let table = |entries: Vec<HookEntry>| entries.into_iter().collect::<HookTable>();
    [
        (
            "nmi_race",
            common::nmi_race_rom(),
            table(vec![rts("leaf", RACE_LEAF, HookMode::Rust, race_leaf, RACE_LEAF_COST)]),
            RACE_IDLE,
        ),
        (
            "call",
            common::call_rom(true),
            table(vec![
                rts("outer", CALL_OUTER, HookMode::Rust, call_outer, CALL_OUTER_COST),
                rts("flat", CALL_FLAT, HookMode::Rust, call_flat, CALL_FLAT_COST),
                rts(
                    "outer_long",
                    CALL_OUTER_LONG,
                    HookMode::Rust,
                    call_outer_long,
                    CALL_OUTER_LONG_COST,
                ),
            ]),
            CALL_LOOP,
        ),
        (
            "deep_stack",
            common::deep_stack_rom(),
            table(vec![
                rts("deep", DEEP, HookMode::Rust, deep_in_rust, DEEP_COST),
                rts("leaf", DEEP_LEAF, HookMode::Rust, deep_leaf, DEEP_LEAF_COST),
            ]),
            DEEP_IDLE,
        ),
    ]
}

#[test]
fn run_lockstep_goldens() {
    let actual: Vec<(&str, u32)> = lockstep_fixtures()
        .into_iter()
        .map(|(name, rom, table, sync)| {
            let machine = Machine::new(rom.clone(), Box::new(MemorySaveStore::default())).unwrap();
            let mut replay = Replay::of(&machine);
            replay.inputs = (0..LOCKSTEP_FRAMES).map(input).collect();
            let config = LockstepConfig {
                table: Some(table),
                sync_keys: vec![sync],
                budget_frames: 30,
                b_packs: None,
            };
            let report = run_lockstep(rom, &replay, &config, None).unwrap();
            let text = format!(
                "{} events, {} frames, {} unsynced, drift {}, dead stack {:?}\n{:?}\n{}",
                report.events,
                report.frames,
                report.unsynced_windows,
                report.max_cycle_drift,
                report.widest_dead_stack,
                report.first_divergence,
                stats_text(&report.hook_stats)
            );
            (name, ssd_rom::crc32(text.as_bytes()))
        })
        .collect();
    if actual != RUN_LOCKSTEP {
        let mut lines = String::new();
        for (name, text) in &actual {
            let _ = writeln!(lines, "    (\"{name}\", 0x{text:08X}),");
        }
        panic!("run_lockstep: the goldens changed; actual:\n{lines}");
    }
}

// `Machine::run` and its stops

fn no_input(_: u64) -> Joypad {
    Joypad::default()
}

fn pure(rom: Vec<u8>) -> Machine {
    Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap()
}

fn run(m: &mut Machine, stops: Stops, budget_frames: u64) -> Stopped {
    m.run(&stops, &mut no_input, &mut NullAudio, budget_frames).unwrap()
}

fn instructions(n: u64) -> Stops {
    Stops { instructions: Some(n), ..Stops::default() }
}

fn full_pc(m: &Machine) -> u32 {
    let r = m.cpu_registers();
    (u32::from(r.pbr) << 16) | u32::from(r.pc)
}

fn stats(m: &Machine, name: &str) -> HookStats {
    let all = m.hook_stats();
    all.iter()
        .find(|(n, _)| n == name)
        .map(|&(_, s)| s)
        .unwrap_or_else(|| panic!("{name}: {all:?}"))
}

#[test]
fn instructions_stop_right_before_the_next_opcode_fetch() {
    let mut m = pure(common::jsr_leaf_rom());
    m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    // Off a boundary first: `instructions(0)` completes the instruction in progress, no more.
    while m.at_fetch_boundary() {
        m.tick(Joypad::default(), &mut NullAudio).unwrap();
    }
    let fetches = m.opcode_fetches();
    assert_eq!(run(&mut m, instructions(0), 1), Stopped::Instructions);
    assert!(m.at_fetch_boundary() && !m.is_mid_instruction());
    assert_eq!(m.opcode_fetches(), fetches, "never stopped mid-instruction");

    // At a boundary, 0 stops at once.
    let cycles = m.total_master_cycles();
    assert_eq!(run(&mut m, instructions(0), 1), Stopped::Instructions);
    assert_eq!(m.total_master_cycles(), cycles);

    for n in [1, 1, 1, 2, 7, 1, 30, 1] {
        // A twin ticks to the opcode fetch of the n + 1st instruction: the stop is right before.
        let mut twin = m.snapshot();
        let fetches = twin.opcode_fetches();
        let mut before_fetch = twin.total_master_cycles();
        while twin.opcode_fetches() < fetches + n + 1 {
            before_fetch = twin.total_master_cycles();
            twin.tick(Joypad::default(), &mut NullAudio).unwrap();
        }
        assert_eq!(run(&mut m, instructions(n), 1), Stopped::Instructions);
        assert_eq!(m.opcode_fetches(), fetches + n, "{n} instructions");
        assert_eq!(m.total_master_cycles(), before_fetch, "{n}: at the next fetch");
        assert!(m.at_fetch_boundary());
    }
}

#[test]
fn with_an_nmi_pending_an_instruction_stop_lands_on_the_handlers_first_instruction() {
    let mut m = pure(common::sync_idle_rom());
    for _ in 0..3 {
        let nmis = m.nmi_count();
        let mut previous = full_pc(&m);
        while m.nmi_count() == nmis {
            previous = full_pc(&m);
            let fetches = m.opcode_fetches();
            assert_eq!(run(&mut m, instructions(1), 2), Stopped::Instructions);
            assert_eq!(m.opcode_fetches(), fetches + 1, "the entry fetches no opcode");
        }
        // The instruction the NMI followed ran, then the whole entry sequence.
        assert_eq!(full_pc(&m), 0x00_8040, "the handler's first instruction");
        assert!(m.at_fetch_boundary() && !m.is_handling_interrupt());
        assert_ne!(previous & 0xFFFF, 0x8040, "came from the main loop: {previous:06X}");
        assert!(m.cpu_registers().s < common::MAIN_STACK, "the entry pushed the return");
    }
}

#[test]
fn stack_above_stops_at_the_first_boundary_with_s_above() {
    let mut m = pure(common::jsr_leaf_rom());
    m.run_frame(Joypad::default(), &mut NullAudio).unwrap();
    while full_pc(&m) != LEAF {
        run(&mut m, instructions(1), 1);
    }
    let s = m.cpu_registers().s;
    let cycles = m.total_master_cycles();
    assert_eq!(run(&mut m, Stops { stack_above: Some(s - 1), ..Stops::default() }, 1), {
        Stopped::StackAbove
    });
    assert_eq!(m.total_master_cycles(), cycles, "already above: at once");

    // `INC $10; RTS`: S rises with the RTS's pulls, which end at the return address.
    let fetches = m.opcode_fetches();
    let stops = Stops { stack_above: Some(s), ..Stops::default() };
    assert_eq!(run(&mut m, stops, 1), Stopped::StackAbove);
    assert_eq!(m.opcode_fetches(), fetches + 2);
    assert_eq!((full_pc(&m), m.cpu_registers().s), (0x00_800A, s + 2));
    assert!(m.at_fetch_boundary());
}

#[test]
fn the_first_stop_reached_wins() {
    let from = {
        let mut m = machine(&FIXTURES[0]);
        m.run_until_sync(&mut no_input, &mut NullAudio, 3).unwrap().unwrap();
        m
    };
    let alone = |stops: Stops| {
        let mut m = from.snapshot();
        let stopped = run(&mut m, stops, 5);
        (stopped, m.total_master_cycles())
    };
    let frame_end = Stops { frame_end: true, ..Stops::default() };
    let sync = Stops { sync: true, ..Stops::default() };
    let call = Stops { call: Some(SYNC_IDLE), ..Stops::default() };
    // In the order they come: the next pass through the idle loop, 40 instructions, 9000, the
    // frame's end, the sync event after the NMI, 30 000 instructions.
    let candidates =
        [call, instructions(40), instructions(9000), frame_end, sync, instructions(30_000)];
    let single: Vec<(Stopped, u64)> = candidates.iter().map(|&stops| alone(stops)).collect();
    assert!(single.windows(2).all(|pair| pair[0].1 < pair[1].1), "{single:?}");
    assert!(matches!(single[0].0, Stopped::Call { .. }), "{single:?}");
    assert_eq!(single[3].0, Stopped::FrameEnd);
    assert!(matches!(single[4].0, Stopped::Sync(event) if event.key == SYNC_IDLE));
    assert_eq!(single[5].0, Stopped::Instructions);
    for (i, a) in candidates.iter().enumerate() {
        for (j, b) in candidates.iter().enumerate().skip(i + 1) {
            if a.instructions.is_some() && b.instructions.is_some() {
                continue;
            }
            let both = Stops {
                frame_end: a.frame_end || b.frame_end,
                call: a.call.or(b.call),
                sync: a.sync || b.sync,
                instructions: a.instructions.or(b.instructions),
                ..Stops::default()
            };
            assert_eq!(alone(both), single[i], "{a:?} with {b:?}: {:?}", single[j]);
        }
    }
}

#[test]
fn a_run_without_its_stop_ends_at_the_budget() {
    let mut m = machine(&FIXTURES[0]);
    let start = m.frame_count();
    assert_eq!(run(&mut m, Stops::default(), 3), Stopped::Budget);
    assert_eq!(m.frame_count(), start + 3);
    assert!(m.hook_stats()[0].1.calls > 0, "arrivals were dispatched");

    let call = Stops { call: Some(NEVER_CALLED), stack_above: Some(0xFFFF), ..Stops::default() };
    assert_eq!(run(&mut m, call, 2), Stopped::Budget);
    assert_eq!(m.frame_count(), start + 5);

    // A budget of 0 runs nothing, whatever the stops.
    let cycles = m.total_master_cycles();
    let every = Stops { frame_end: true, sync: true, instructions: Some(0), ..Stops::default() };
    assert_eq!(run(&mut m, every, 0), Stopped::Budget);
    assert_eq!(m.total_master_cycles(), cycles);

    // Unhooked too.
    let mut m = pure(common::sync_idle_rom());
    assert_eq!(run(&mut m, Stops::default(), 2), Stopped::Budget);
    assert_eq!(m.frame_count(), 2);
}

#[test]
fn a_call_stop_gives_the_counters_of_its_routine() {
    let mut m = machine(&FIXTURES[2]);
    // A mirror of the leaf's address (bank $80).
    let stops = Stops { call: Some(0x80_0000 | RACE_LEAF), ..Stops::default() };
    for calls in 1..=40 {
        let Stopped::Call { stats: leaf, .. } = run(&mut m, stops, 1) else { panic!("no call") };
        assert_eq!(leaf.calls, calls);
        assert_eq!(leaf, stats(&m, "leaf"));
    }
    let leaf = stats(&m, "leaf");
    assert!(leaf.rust_runs > 0 && leaf.timing_fallback > 0, "{leaf:?}");
}

/// `INC $10` of the leaf of [`common::nmi_leaf_rom`], with its `RTS`.
fn leaf_in_rust(ctx: &mut Ctx<'_>) {
    inc_dp(ctx, common::LEAF_COUNTER);
    ctx.set_cost(INC_DP + RTS);
}

#[test]
fn sync_arrivals_inside_a_dispatch_are_counted_and_are_no_events() {
    // The RTS of a Verify leaf as the sync point: every arrival is inside the leaf's original.
    let mut m = pure(common::nmi_leaf_rom());
    let leaf = rts("leaf", LEAF, HookMode::Verify, leaf_in_rust, INC_DP + RTS);
    m.set_hook_table(Some([leaf, sync_point(LEAF + 2)].into_iter().collect()));
    assert_eq!(m.run_until_sync(&mut no_input, &mut NullAudio, 4).unwrap(), None);
    let (point, leaf) = (stats(&m, "sync"), stats(&m, "leaf"));
    assert!(leaf.verified_ok > 100, "{leaf:?}");
    // Thousands of arrivals, but one delayed event per NMI: the leaf runs between any two.
    assert_eq!(m.nmi_count(), 3);
    assert_eq!(point.sync_inside_dispatch, 3, "once per NMI: {point:?}");
    assert_eq!(point.calls, 0, "inside a Verify call's original, arrivals are not dispatched");

    // The same sync point with the leaf original: an event per NMI, none inside a dispatch.
    let mut m = pure(common::nmi_leaf_rom());
    m.set_hook_table(Some([sync_point(LEAF + 2)].into_iter().collect()));
    let event = m.run_until_sync(&mut no_input, &mut NullAudio, 4).unwrap().unwrap();
    assert_eq!(event.key, LEAF + 2);
    assert_eq!(stats(&m, "sync").sync_inside_dispatch, 0);

    // Inside original code a Rust routine calls: `inner` of `call_rom`, called from `outer` only.
    let table = || -> HookTable {
        let outer = rts("outer", CALL_OUTER, HookMode::Rust, call_outer, CALL_OUTER_COST);
        [outer, sync_point(CALL_INNER)].into_iter().collect()
    };
    let mut m = pure(common::call_rom(true));
    m.set_hook_table(Some(table()));
    let mut events = 0;
    while m.frame_count() < 10 {
        let budget = 10 - m.frame_count();
        events +=
            u64::from(m.run_until_sync(&mut no_input, &mut NullAudio, budget).unwrap().is_some());
    }
    let (inner, outer) = (stats(&m, "sync"), stats(&m, "outer"));
    assert!(inner.sync_inside_dispatch > 0, "{inner:?}");
    // An NMI delays one event at most (which comes after the dispatch, as one of `events`).
    assert!(inner.sync_inside_dispatch <= m.nmi_count(), "{}: {inner:?}", m.nmi_count());
    // Events come from the calls that ran as original (their arrival at `inner` is top-level).
    assert!(outer.timing_fallback > 0 && events <= outer.timing_fallback, "{events}: {outer:?}");

    // Lockstep reports the sync points' counters of B.
    let rom = common::call_rom(true);
    let mut replay = Replay::of(&pure(rom.clone()));
    replay.inputs = vec![Joypad::default(); 30];
    let routines = table().entries().filter(|e| e.name == "outer").cloned().collect();
    let config = LockstepConfig {
        table: Some(routines),
        sync_keys: vec![CALL_INNER],
        budget_frames: 30,
        b_packs: None,
    };
    let report = run_lockstep(rom, &replay, &config, None).unwrap();
    assert_eq!(report.hook_stats.len(), 1, "{:?}", report.hook_stats);
    let [(name, sync)] = &report.sync_stats[..] else { panic!("{:?}", report.sync_stats) };
    assert_eq!(name, "sync_point_008060");
    assert!(sync.sync_inside_dispatch > 0, "{sync:?}");
}

// The goldens (synthetic fixtures only): fixture, CRC32 of the state, CRC32 of the text.

const RUN_FRAME: [(&str, u32, u32); 6] = [
    ("sync_idle", 0xEFC2DD65, 0x3909525E),
    ("nmi_race", 0x4607F566, 0x83697D91),
    ("hdma_idle", 0x21FEFADD, 0xBF79C2E6),
    ("call", 0xB6B93848, 0xA676BE30),
    ("nested", 0xDA5B1B5C, 0x8997CBDE),
    ("deep_stack", 0x8407383A, 0xF7CFC45A),
];

const RUN_FRAME_WITH: [(&str, u32, u32); 6] = [
    ("sync_idle", 0xEFC2DD65, 0x3909525E),
    ("nmi_race", 0x4607F566, 0x83697D91),
    ("hdma_idle", 0x21FEFADD, 0xBF79C2E6),
    ("call", 0xB6B93848, 0xA676BE30),
    ("nested", 0xDB2D8BE4, 0x8997CBDE),
    ("deep_stack", 0x8407383A, 0xF7CFC45A),
];

const RUN_UNTIL_CALL: [(&str, u32, u32); 6] = [
    ("sync_idle", 0xA322E155, 0x41A75D58),
    ("nmi_race", 0x6AF7FCBD, 0x56281A69),
    ("hdma_idle", 0xB9B03ABC, 0x12587688),
    ("call", 0x42AA2858, 0xBA94B45A),
    ("nested", 0xA94CFC70, 0xB0E252C4),
    ("deep_stack", 0xB31412AE, 0x7D2C5F40),
];

const RUN_UNTIL_SYNC: [(&str, u32, u32); 6] = [
    ("sync_idle", 0x4A063095, 0xBC64F4AE),
    ("nmi_race", 0xD7BB90C4, 0xEE0782FC),
    ("hdma_idle", 0x2D86FD88, 0x625FDE1B),
    ("call", 0x34352E71, 0x1248E3CA),
    ("nested", 0x25798687, 0xE1145885),
    ("deep_stack", 0x087B8D23, 0xFA343D47),
];

/// Lockstep: fixture, CRC32 of the report's text.
const RUN_LOCKSTEP: [(&str, u32); 3] =
    [("nmi_race", 0xB3CC9D67), ("call", 0x2AFC3F6F), ("deep_stack", 0x40A70F3B)];
