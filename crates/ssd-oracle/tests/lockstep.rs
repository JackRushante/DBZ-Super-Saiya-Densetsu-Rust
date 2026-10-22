//! Lockstep: a replay played on a pure machine (A) and on one with hooks (B), compared at every
//! sync event (registers, WRAM but the dead stack, SRAM, VRAM, CGRAM, OAM, NMI deltas, frame and
//! master cycle).
//!
//! The real-ROM test needs the game's ROM:
//! `SSD_ROM_PATH=... cargo test -p ssd-oracle --release --test lockstep -- --ignored --nocapture`.

mod common;

use common::{
    DEEP, DEEP_COST, DEEP_ENTRY_S, DEEP_IDLE, DEEP_LEAF, DEEP_LEAF_COST, DEEP_LEAF_COUNTER,
    DEEP_NMI_COUNTER, DEEP_NMI_LOW, LEAF, LEAF_COUNTER, MAIN_STACK, RACE_IDLE, RACE_LEAF,
    RACE_LEAF_COST, RACE_NMI_COUNTER, RACE_SEED, STACK_LEAF_X, SYNC_IDLE, deep_in_rust,
    deep_stack_rom, nmi_off_idle_rom, nmi_race_rom, race_leaf, stack_leaf_rom, sync_idle_rom,
};
use ssd_game::scripts::{BATTLE_FRAMES, WANDER_FRAMES, battle_input, wander_input};
use ssd_oracle::{
    Ctx, DEFAULT_BUDGET_FRAMES, Difference, HookAction, HookEntry, HookMode, HookStats, HookTable,
    Joypad, LockstepConfig, LockstepDivergence, LockstepReport, LockstepRun, LockstepStep, Machine,
    MemorySaveStore, NullAudio, OracleError, Replay, ReturnKind, RoutineFn, Symbols, Verdict,
    run_lockstep,
};
use std::path::Path;
use std::sync::Arc;

/// Frames of the synthetic replays.
const FRAMES: u64 = 120;
/// Sync point of [`stack_leaf_rom`]: its main loop's first instruction (`LDY`).
const STACK_LOOP: u32 = 0x00_8012;
/// Master cycles of a frame (262 lines of 1364).
const FRAME_MCLK: u64 = 262 * 1364;
/// `NMITIMEN`.
const NMITIMEN: u32 = 0x00_4200;

/// A replay of `rom` from power-on, `input(frame)` held during each of `frames` frames.
fn replay(rom: &[u8], frames: u64, input: fn(u64) -> Joypad) -> Replay {
    let machine = Machine::new(rom.to_vec(), Box::new(MemorySaveStore::default())).unwrap();
    let mut replay = Replay::of(&machine);
    replay.inputs = (0..frames).map(input).collect();
    replay
}

fn no_input(_: u64) -> Joypad {
    Joypad::default()
}

/// A config with `table` on B and the sync point `sync`, with the default budget.
fn config(table: impl Into<Option<HookTable>>, sync: u32) -> LockstepConfig {
    LockstepConfig {
        table: table.into(),
        sync_keys: vec![sync],
        budget_frames: DEFAULT_BUDGET_FRAMES,
        b_packs: None,
    }
}

/// [`FRAMES`] frames of `rom` without input, in lockstep with `config`.
fn lockstep(rom: Vec<u8>, config: &LockstepConfig) -> LockstepReport {
    let replay = replay(&rom, FRAMES, no_input);
    run_lockstep(rom, &replay, config, None).unwrap()
}

/// A routine returning with RTS; contract: 8-bit A, 16-bit X/Y (the fixtures').
fn routine(key: u32, mode: HookMode, func: RoutineFn, cycles: Option<u64>) -> HookEntry {
    HookEntry {
        name: "leaf".to_owned(),
        key,
        mode,
        action: HookAction::Routine { func, ret: ReturnKind::Rts },
        entry_m8: Some(true),
        entry_x8: Some(false),
        flags_mask: 0xFF,
        verify_sample: 1,
        bound: None,
        budget_frames: None,
        cycles,
    }
}

/// The leaf of [`nmi_race_rom`] as `func`, in Rust mode with `cycles` as its maximum cost.
fn race_table(func: RoutineFn, cycles: u64) -> HookTable {
    [routine(RACE_LEAF, HookMode::Rust, func, Some(cycles))].into_iter().collect()
}

fn leaf_stats(report: &LockstepReport) -> HookStats {
    assert_eq!(report.hook_stats.len(), 1, "{:?}", report.hook_stats);
    let (name, stats) = &report.hook_stats[0];
    assert_eq!(name, "leaf");
    *stats
}

/// The frame of every sync event of a pure run of `rom` with the sync point `sync`.
fn event_frames(rom: Vec<u8>, sync: u32) -> Vec<u64> {
    let mut m = Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap();
    m.set_hook_table(Some([HookEntry::sync_point("idle", sync)].into_iter().collect()));
    let mut frames = Vec::new();
    while m.frame_count() < FRAMES {
        let budget = FRAMES - m.frame_count();
        if let Some(event) = m.run_until_sync(&mut no_input, &mut NullAudio, budget).unwrap() {
            frames.push(event.frame);
        }
    }
    frames
}

// Review Focus 2: without hooks on B, nothing diverges

#[test]
fn without_hooks_on_b_the_machines_agree_at_every_event() {
    for (rom, sync) in [(sync_idle_rom(), SYNC_IDLE), (nmi_race_rom(), RACE_IDLE)] {
        let events = event_frames(rom.clone(), sync).len() as u64;
        let report = lockstep(rom, &config(None, sync));
        assert_eq!(report.first_divergence, None, "{report:?}");
        assert_eq!(report.verdict(), Verdict::Green);
        assert_eq!(report.max_cycle_drift, 0);
        assert_eq!((report.events, report.frames, report.unsynced_windows), (events, FRAMES, 0));
        assert!(events >= FRAMES - 2, "an event per frame: {events}");
        assert!(report.hook_stats.is_empty(), "no routine: {:?}", report.hook_stats);
    }
}

// Rust routines

#[test]
fn rust_routines_with_exact_costs_stay_in_lockstep() {
    let report =
        lockstep(nmi_race_rom(), &config(race_table(race_leaf, RACE_LEAF_COST), RACE_IDLE));
    assert_eq!(report.first_divergence, None, "{report:?}");
    assert_eq!(report.max_cycle_drift, 0);
    assert!(report.events >= FRAMES - 2, "{report:?}");
    let leaf = leaf_stats(&report);
    assert!(leaf.rust_runs > 1000, "{leaf:?}");
    assert!(leaf.timing_fallback >= report.events / 2, "the NMI usually lands in it: {leaf:?}");
}

/// [`race_leaf`], then 1 added to the seed once the NMI handler has counted 5 NMIs.
fn race_leaf_wrong_from_the_fifth_nmi(ctx: &mut Ctx<'_>) {
    race_leaf(ctx);
    if ctx.read_dp8(RACE_NMI_COUNTER as u16) >= 5 {
        let seed = ctx.read_dp16(RACE_SEED as u16);
        ctx.write_dp16(RACE_SEED as u16, seed.wrapping_add(1));
    }
}

#[test]
fn a_wrong_rust_routine_diverges_in_state_at_the_first_event_after_its_call() {
    let rom = nmi_race_rom();
    let frames = event_frames(rom.clone(), RACE_IDLE);
    let symbols = Symbols::parse(
        "[[variable]]\nname = \"seed\"\naddress = 0x7E0040\nsize = 2\n",
        Path::new("symbols.toml"),
    )
    .unwrap();
    let replay = replay(&rom, FRAMES, no_input);
    let config = config(race_table(race_leaf_wrong_from_the_fifth_nmi, RACE_LEAF_COST), RACE_IDLE);
    let report = run_lockstep(rom, &replay, &config, Some(&symbols)).unwrap();

    // Event 4 follows the fifth NMI (event k follows NMI k + 1): the first wrong call comes
    // right after it, and event 5 sees its seed.
    let Some(LockstepDivergence::State { event, frame, differences, truncated, .. }) =
        &report.first_divergence
    else {
        panic!("a state divergence: {report:?}");
    };
    assert_eq!((*event, *frame, *truncated), (5, frames[5], 0));
    assert_eq!(report.events, 5, "events compared equal before it");
    assert_eq!(report.frames, frames[5]);
    assert_eq!(report.verdict(), Verdict::Diverged);
    assert_eq!(report.max_cycle_drift, 0, "the costs stay exact");
    // The seed, and the accumulator, whose high byte keeps the seed's past the leaf's `SEP`.
    let seed = RACE_SEED as u32..RACE_SEED as u32 + 2;
    assert!(differences.iter().any(|d| matches!(d, Difference::Memory { .. })), "{differences:?}");
    for difference in differences {
        assert!(
            matches!(difference, Difference::Register { name: "A", .. })
                || matches!(difference, Difference::Memory { space: "wram", address, .. }
                    if seed.contains(address)),
            "only the seed differs: {differences:?}"
        );
    }
    let text = report.first_divergence.as_ref().unwrap().to_string();
    assert!(text.starts_with(&format!("event 5 (frame {}): state differs", frames[5])), "{text}");
    assert!(text.contains("wram[$7E:0040] seed: $"), "named after the symbols: {text}");
}

/// [`race_leaf`] declaring 2000 master cycles more than the original takes.
fn race_leaf_2000_cycles_slower(ctx: &mut Ctx<'_>) {
    race_leaf(ctx);
    ctx.set_cost(RACE_LEAF_COST + 2000);
}

#[test]
fn a_routine_declaring_2000_more_cycles_drifts() {
    let table = race_table(race_leaf_2000_cycles_slower, RACE_LEAF_COST + 2000);
    let report = lockstep(nmi_race_rom(), &config(table, RACE_IDLE));
    let Some(LockstepDivergence::CycleDrift { event: 0, drift }) = report.first_divergence else {
        panic!("a cycle drift at the first event: {report:?}");
    };
    assert_ne!(drift, 0);
    assert_eq!(report.max_cycle_drift, drift);
    assert_eq!(report.events, 0);
    let leaf = leaf_stats(&report);
    assert!(leaf.rust_runs > 0 && leaf.cost_over_bound == 0, "{leaf:?}");
    let text = report.first_divergence.unwrap().to_string();
    assert!(text.contains("cycle drift"), "{text}");
}

/// [`race_leaf`] declaring two frames more than the original takes (beyond its `cycles`): the
/// first call holds the CPU through two vblanks.
fn race_leaf_two_frames_slower(ctx: &mut Ctx<'_>) {
    race_leaf(ctx);
    ctx.set_cost(RACE_LEAF_COST + 2 * FRAME_MCLK);
}

#[test]
fn a_routine_making_b_late_by_frames_is_a_lag_drift() {
    let report = lockstep(
        nmi_race_rom(),
        &config(race_table(race_leaf_two_frames_slower, RACE_LEAF_COST), RACE_IDLE),
    );
    let Some(LockstepDivergence::LagDrift { event: 0, frame_a, frame_b, .. }) =
        report.first_divergence
    else {
        panic!("a lag drift at the first event: {report:?}");
    };
    assert!(frame_b > frame_a, "{report:?}");
    let text = report.first_divergence.unwrap().to_string();
    assert!(text.contains("lag drift"), "{text}");
}

/// [`race_leaf`], turning the NMI off once the NMI handler has counted 3 NMIs.
fn race_leaf_stopping_the_nmi(ctx: &mut Ctx<'_>) {
    race_leaf(ctx);
    if ctx.read_dp8(RACE_NMI_COUNTER as u16) >= 3 {
        ctx.write8(NMITIMEN, 0x00);
    }
}

#[test]
fn an_event_only_one_machine_reaches_is_an_event_count_divergence() {
    let rom = nmi_race_rom();
    let frames = event_frames(rom.clone(), RACE_IDLE);
    let table = race_table(race_leaf_stopping_the_nmi, RACE_LEAF_COST);
    let report = lockstep(rom, &config(table, RACE_IDLE));
    // Event 2 follows the third NMI; B takes no NMI after it, A has event 3.
    let frame = frames[3];
    assert_eq!(report.first_divergence, Some(LockstepDivergence::EventCount { a: 4, b: 3, frame }));
    assert_eq!(report.events, 3);
    let text = report.first_divergence.unwrap().to_string();
    assert_eq!(
        text,
        format!(
            "event count at frame {frame}: the original reached 4 sync events, the hooked machine 3\n"
        )
    );
}

// Unsynced windows

#[test]
fn a_run_without_nmi_has_no_events_and_is_unverified() {
    let mut config = config(None, SYNC_IDLE);
    config.budget_frames = 50;
    let report = lockstep(nmi_off_idle_rom(), &config);
    assert_eq!(report.first_divergence, None, "{report:?}");
    assert_eq!(report.events, 0);
    assert_eq!(report.frames, FRAMES, "the whole replay ran");
    assert_eq!(report.unsynced_windows, 2, "two full budgets; the 20-frame tail is no window");
    assert_eq!(report.verdict(), Verdict::Unverified);
}

#[test]
fn nmi_deltas_are_compared_in_unsynced_windows() {
    // The sync point is never reached: every window is unsynced. B's leaf turns the NMI off.
    let mut config = config(race_table(race_leaf_stopping_the_nmi, RACE_LEAF_COST), 0x00_9000);
    config.budget_frames = 30;
    let report = lockstep(nmi_race_rom(), &config);
    let Some(LockstepDivergence::LagDrift { event: 0, frame_a: 30, frame_b: 30, nmis_a, nmis_b }) =
        report.first_divergence
    else {
        panic!("a lag drift in the first window: {report:?}");
    };
    assert!(nmis_a >= 29 && nmis_b == 3, "{nmis_a} {nmis_b}");
    assert_eq!((report.events, report.unsynced_windows), (0, 0));

    // The same windows without the leaf agree.
    let mut config = config_without_table(0x00_9000);
    config.budget_frames = 30;
    let report = lockstep(nmi_race_rom(), &config);
    assert_eq!(report.first_divergence, None);
    assert_eq!((report.events, report.unsynced_windows), (0, 4));
    assert_eq!(report.verdict(), Verdict::Unverified);
}

fn config_without_table(sync: u32) -> LockstepConfig {
    config(None, sync)
}

// Dead stack

/// The leaf of [`stack_leaf_rom`] in Rust (`PHX; INC $10; PLX; RTS` without the push), with its
/// exact cost: `PHX` 30, `INC dp` 38, `PLX` 36, `RTS` 42.
fn stack_leaf(ctx: &mut Ctx<'_>) {
    let counter = ctx.read_dp8(LEAF_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(LEAF_COUNTER as u16, counter);
    ctx.set_nz16(ctx.x());
    ctx.set_cost(STACK_LEAF_COST);
}

const STACK_LEAF_COST: u64 = 146;

/// [`stack_leaf`], also writing a byte far below the stack (never pushed to).
fn stack_leaf_writing_below_the_stack(ctx: &mut Ctx<'_>) {
    stack_leaf(ctx);
    ctx.write8(0x00_1F00, 0x5A);
}

fn stack_table(func: RoutineFn) -> HookTable {
    [routine(LEAF, HookMode::Rust, func, Some(STACK_LEAF_COST))].into_iter().collect()
}

#[test]
fn the_stack_bytes_a_rust_routine_does_not_push_are_dead() {
    let rom = stack_leaf_rom();
    let report = lockstep(rom.clone(), &config(stack_table(stack_leaf), STACK_LOOP));
    assert_eq!(report.first_divergence, None, "{report:?}");
    assert!(report.events >= FRAMES - 2, "{report:?}");
    assert!(leaf_stats(&report).rust_runs > 100, "{report:?}");

    // They do differ: the original's push leaves X there, the Rust routine leaves what the NMI
    // handler pushed.
    let mut a = Machine::new(rom.clone(), Box::new(MemorySaveStore::default())).unwrap();
    let mut b = Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap();
    let sync = || HookEntry::sync_point("loop", STACK_LOOP);
    a.set_hook_table(Some([sync()].into_iter().collect()));
    let mut table = stack_table(stack_leaf);
    table.insert(sync());
    b.set_hook_table(Some(table));
    let mut dead_bytes_differed = false;
    for _ in 0..30 {
        let next = |m: &mut Machine| m.run_until_sync(&mut no_input, &mut NullAudio, 3).unwrap();
        assert_eq!(next(&mut a), next(&mut b));
        let s = usize::from(MAIN_STACK);
        let differing: Vec<usize> =
            (0..a.wram().len()).filter(|&i| a.wram()[i] != b.wram()[i]).collect();
        assert!(differing.iter().all(|&i| (s - 4..=s).contains(&i)), "{differing:X?}");
        dead_bytes_differed |= !differing.is_empty();
    }
    assert!(dead_bytes_differed, "the push of X ({STACK_LEAF_X:04X}) is missing on B");
}

/// The leaf of [`stack_leaf_rom`] handed back to its original: on B, the original's push lands
/// below the return address `call_original` pushes, deeper than A's stack ever goes.
fn stack_leaf_through_its_original(ctx: &mut Ctx<'_>) {
    ctx.call_original(LEAF, ReturnKind::Rts).unwrap();
    ctx.set_cost(0);
}

#[test]
fn the_stack_bytes_original_code_called_from_rust_pushes_are_dead() {
    let report = lockstep(
        stack_leaf_rom(),
        &config(stack_table(stack_leaf_through_its_original), STACK_LOOP),
    );
    assert_eq!(report.first_divergence, None, "{report:?}");
    assert!(report.events >= FRAMES - 2, "{report:?}");
    assert!(leaf_stats(&report).rust_runs > 100, "{report:?}");
}

#[test]
fn bytes_below_the_stack_that_were_never_pushed_to_are_compared() {
    let rom = stack_leaf_rom();
    let power_on = Machine::new(rom.clone(), Box::new(MemorySaveStore::default())).unwrap();
    let original = power_on.wram()[0x1F00];
    assert_ne!(original, 0x5A);
    let report =
        lockstep(rom, &config(stack_table(stack_leaf_writing_below_the_stack), STACK_LOOP));
    let Some(LockstepDivergence::State { event: 0, differences, .. }) = &report.first_divergence
    else {
        panic!("a state divergence: {report:?}");
    };
    assert_eq!(
        differences,
        &[Difference::Memory { space: "wram", address: 0x1F00, original, rust: 0x5A }]
    );
}

// Dead stack across events

/// The bytes [`DEEP`] pushes to that nothing pushes to again after boot: from its lowest push up
/// to the NMI handler's lowest one, as WRAM offsets.
fn deep_only() -> std::ops::RangeInclusive<usize> {
    usize::from(DEEP_ENTRY_S) - 15..=usize::from(DEEP_NMI_LOW)
}

fn deep_table(key: u32, func: RoutineFn, cycles: u64) -> HookTable {
    [routine(key, HookMode::Rust, func, Some(cycles))].into_iter().collect()
}

#[test]
fn a_dead_byte_that_differs_once_stays_dead_across_events() {
    // [`DEEP`] runs once, at boot, in Rust on B: its pushes are missing there for good, and no
    // later push reaches them. Its stack is freed with `TCS`, which is no stack switch.
    let rom = deep_stack_rom();
    let table = deep_table(DEEP, deep_in_rust, DEEP_COST);
    let report = lockstep(rom.clone(), &config(table.clone(), DEEP_IDLE));
    assert_eq!(report.first_divergence, None, "{report:?}");
    assert!(report.events >= FRAMES - 2, "{report:?}");
    assert_eq!(leaf_stats(&report).rust_runs, 1);
    let lowest = u32::from(DEEP_ENTRY_S) - 15;
    assert_eq!(report.widest_dead_stack, lowest..u32::from(MAIN_STACK) + 1);

    // The bytes do differ, at every event, and only there.
    let mut a = Machine::new(rom.clone(), Box::new(MemorySaveStore::default())).unwrap();
    let mut b = Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap();
    let sync = || HookEntry::sync_point("idle", DEEP_IDLE);
    a.set_hook_table(Some([sync()].into_iter().collect()));
    let mut table = table;
    table.insert(sync());
    b.set_hook_table(Some(table));
    for _ in 0..20 {
        let next = |m: &mut Machine| m.run_until_sync(&mut no_input, &mut NullAudio, 3).unwrap();
        assert_eq!(next(&mut a), next(&mut b));
        let differing: Vec<usize> =
            (0..a.wram().len()).filter(|&i| a.wram()[i] != b.wram()[i]).collect();
        assert!(!differing.is_empty() && differing.iter().all(|i| deep_only().contains(i)));
    }
}

/// [`DEEP_LEAF`] in Rust (`INC $10`), also writing `$A5` at `$1FE4`, below the stack the idle
/// loop and the NMI handler use but where [`DEEP`] pushed at boot, once 5 NMIs are counted.
fn deep_leaf_writing_where_the_stack_was(ctx: &mut Ctx<'_>) {
    let counter = ctx.read_dp8(DEEP_LEAF_COUNTER as u16).wrapping_add(1);
    ctx.write_dp8(DEEP_LEAF_COUNTER as u16, counter);
    ctx.set_nz8(counter);
    if ctx.read_dp8(DEEP_NMI_COUNTER as u16) >= 5 {
        ctx.write8(WHERE_THE_STACK_WAS, 0xA5);
    }
    ctx.set_cost(DEEP_LEAF_COST);
}

/// A byte [`DEEP`] pushed to at boot (the high byte of X, `$1F`).
const WHERE_THE_STACK_WAS: u32 = 0x00_1FE4;

#[test]
fn once_the_floor_has_healed_a_write_where_the_stack_was_is_reported() {
    // [`DEEP`] runs as original on both machines: its dead bytes agree, so the floor rises back
    // at the first event, and the write of event 5's window is compared.
    assert!(deep_only().contains(&(WHERE_THE_STACK_WAS as usize)));
    let table = deep_table(DEEP_LEAF, deep_leaf_writing_where_the_stack_was, DEEP_LEAF_COST);
    let report = lockstep(deep_stack_rom(), &config(table, DEEP_IDLE));
    let Some(LockstepDivergence::State { event: 5, differences, .. }) = &report.first_divergence
    else {
        panic!("a state divergence at event 5: {report:?}");
    };
    assert_eq!(
        differences,
        &[Difference::Memory { space: "wram", address: 0x1FE4, original: 0x1F, rust: 0xA5 }]
    );
}

/// [`DEEP`] handed back to its original from Rust: on B its pushes land below the return address
/// `call_original` pushes, two bytes deeper than on A, and its `TCS` runs inside the dispatch.
fn deep_through_its_original(ctx: &mut Ctx<'_>) {
    ctx.call_original(DEEP, ReturnKind::Rts).unwrap();
    ctx.set_cost(0);
}

#[test]
fn original_code_run_inside_a_dispatch_is_watched_like_stepped_code() {
    let table = deep_table(DEEP, deep_through_its_original, DEEP_COST);
    let report = lockstep(deep_stack_rom(), &config(table, DEEP_IDLE));
    assert_eq!(report.first_divergence, None, "{report:?}");
    assert_eq!(leaf_stats(&report).rust_runs, 1);
    let lowest = u32::from(DEEP_ENTRY_S) - 17;
    assert_eq!(report.widest_dead_stack, lowest..u32::from(MAIN_STACK) + 1);
}

// Reports and setup

/// WRAM offset of the 300 bytes [`race_leaf_filling_300_bytes`] fills (unused by the game).
const FILLED: usize = 0x8000;

/// [`race_leaf`], also filling 300 bytes of WRAM with `$A5`.
fn race_leaf_filling_300_bytes(ctx: &mut Ctx<'_>) {
    race_leaf(ctx);
    for offset in 0..300 {
        ctx.write8(0x7E_0000 + FILLED as u32 + offset, 0xA5);
    }
}

#[test]
fn state_divergences_keep_256_differences() {
    let rom = nmi_race_rom();
    let power_on = Machine::new(rom.clone(), Box::new(MemorySaveStore::default())).unwrap();
    let filled = &power_on.wram()[FILLED..FILLED + 300];
    let total = filled.iter().filter(|&&byte| byte != 0xA5).count();
    assert!(total > 256, "{total}");
    let table = race_table(race_leaf_filling_300_bytes, RACE_LEAF_COST);
    let report = lockstep(rom, &config(table, RACE_IDLE));
    let Some(LockstepDivergence::State { event: 0, differences, truncated, .. }) =
        &report.first_divergence
    else {
        panic!("a state divergence: {report:?}");
    };
    assert_eq!((differences.len(), *truncated), (256, total - 256));
    let text = report.first_divergence.as_ref().unwrap().to_string();
    assert!(text.contains(&format!(", {total} differences\n")), "{text}");
    assert_eq!(text.lines().count(), 1 + 256 + 1, "{text}");
    assert!(text.ends_with(&format!("… {} more differences\n", total - 256)), "{text}");
}

#[test]
fn every_divergence_has_a_line_of_text() {
    let lag =
        LockstepDivergence::LagDrift { event: 7, frame_a: 100, frame_b: 101, nmis_a: 1, nmis_b: 2 };
    assert_eq!(
        lag.to_string(),
        "event 7: lag drift: the original is at frame 100 after 1 NMI(s), the hooked machine at \
         frame 101 after 2\n"
    );
    let drift = LockstepDivergence::CycleDrift { event: 3, drift: -12 };
    assert_eq!(
        drift.to_string(),
        "event 3: cycle drift: the hooked machine arrives 12 master cycles earlier than the \
         original\n"
    );
    let later = LockstepDivergence::CycleDrift { event: 3, drift: 2000 };
    assert!(later.to_string().contains("2000 master cycles later"), "{later}");
    let count = LockstepDivergence::EventCount { a: 0, b: 1, frame: 12 };
    assert_eq!(
        count.to_string(),
        "event count at frame 12: the original reached 0 sync events, the hooked machine 1\n"
    );
}

/// `run_lockstep` with `config` on [`nmi_race_rom`], which must fail: its error message.
fn setup_error(config: &LockstepConfig) -> String {
    let rom = nmi_race_rom();
    let replay = replay(&rom, FRAMES, no_input);
    let error = run_lockstep(rom, &replay, config, None).unwrap_err();
    assert!(matches!(&error, OracleError::Lockstep(_)), "{error}");
    error.to_string()
}

#[test]
fn a_sync_point_on_a_routine_of_the_table_is_an_error() {
    let error = setup_error(&config(race_table(race_leaf, RACE_LEAF_COST), RACE_LEAF));
    assert!(error.contains("$008060"), "{error}");

    // A sync key given twice is one sync point, and so is the table's own entry for it.
    let mut config = config_without_table(RACE_IDLE);
    config.sync_keys.push(0x80_8017);
    config.table = Some([HookEntry::sync_point("idle", RACE_IDLE)].into_iter().collect());
    let rom = nmi_race_rom();
    let replay = replay(&rom, FRAMES, no_input);
    let report = run_lockstep(rom, &replay, &config, None).unwrap();
    assert_eq!(report.verdict(), Verdict::Green);
}

#[test]
fn a_sync_point_of_the_table_that_is_not_a_sync_key_is_an_error() {
    // B alone would have events there.
    let mut table = race_table(race_leaf, RACE_LEAF_COST);
    table.insert(HookEntry::sync_point("elsewhere", 0x00_801A));
    let error = setup_error(&config(table, RACE_IDLE));
    assert!(error.contains("`elsewhere`") && error.contains("$00801A"), "{error}");
}

#[test]
fn verify_routines_are_an_error() {
    // A Verify call runs the original to its return: across a window's end, B can take the
    // next NMI before stopping where A stops on the frame boundary.
    let table: HookTable = [routine(RACE_LEAF, HookMode::Verify, race_leaf, Some(RACE_LEAF_COST))]
        .into_iter()
        .collect();
    let error = setup_error(&config(table, RACE_IDLE));
    assert!(error.contains("`leaf`") && error.contains("Verify"), "{error}");
    // In Original mode it is no hook at all.
    let table: HookTable =
        [routine(RACE_LEAF, HookMode::Original, race_leaf, None)].into_iter().collect();
    assert_eq!(lockstep(nmi_race_rom(), &config(table, RACE_IDLE)).verdict(), Verdict::Green);
}

// Incremental runs

/// A [`LockstepRun`] of [`FRAMES`] frames of `rom` without input.
fn lockstep_run(rom: Vec<u8>, config: &LockstepConfig) -> LockstepRun {
    let replay = Arc::new(replay(&rom, FRAMES, no_input));
    LockstepRun::new(rom, replay, config, None).unwrap()
}

/// Every step of `run` by `next_within(frames)` calls, the `Running` ones counted apart.
fn steps(run: &mut LockstepRun, frames: u64) -> (Vec<LockstepStep>, u64) {
    let (mut steps, mut running) = (Vec::new(), 0);
    while let Some(step) = run.next_within(frames).unwrap() {
        match step {
            LockstepStep::Running { frame } => {
                assert_eq!(frame, run.machine_a().frame_count());
                running += 1;
            }
            step => steps.push(step),
        }
    }
    (steps, running)
}

#[test]
fn the_steps_of_a_fixture_are_its_events_in_order() {
    let rom = nmi_race_rom();
    let frames = event_frames(rom.clone(), RACE_IDLE);
    let config = config(race_table(race_leaf, RACE_LEAF_COST), RACE_IDLE);
    let mut run = lockstep_run(rom, &config);
    let (steps, running) = steps(&mut run, u64::MAX);
    assert_eq!(running, 0);
    // The frames after the last event are a window (not a counted one) with the NMI of the end.
    let (tail, steps) = steps.split_last().unwrap();
    assert!(matches!(tail, LockstepStep::UnsyncedWindow { .. }), "{tail:?}");
    assert_eq!(steps.len(), frames.len());
    let mut last_nmis = 0;
    for (index, (step, frame)) in steps.iter().zip(&frames).enumerate() {
        let LockstepStep::Event { index: i, frame: f, nmis, drift: 0, dead_stack } = step else {
            panic!("event {index}: {step:?}");
        };
        assert_eq!((*i, f), (index as u64, frame));
        assert!(*nmis >= 1, "{step:?}");
        assert!(dead_stack.start <= dead_stack.end && dead_stack.end <= 0x2000, "{step:?}");
        last_nmis += nmis;
    }
    let LockstepStep::UnsyncedWindow { nmis, .. } = tail else { unreachable!() };
    assert_eq!(last_nmis + nmis, run.machine_a().nmi_count());
    assert_eq!(run.next().unwrap(), None, "the replay is played");
    let report = run.report();
    assert_eq!((report.events, report.unsynced_windows), (frames.len() as u64, 0));
    assert_eq!(report.frames, FRAMES);
    assert_eq!(report.verdict(), Verdict::Green);
    assert_eq!(run.machine_a().frame_count(), run.machine_b().frame_count());
}

#[test]
fn stopping_after_k_steps_and_resuming_is_the_one_shot_report() {
    let rom = nmi_race_rom();
    let config = config(race_table(race_leaf, RACE_LEAF_COST), RACE_IDLE);
    let one_shot = lockstep(rom.clone(), &config);
    for k in [0, 1, 2, 17, 60] {
        let mut run = lockstep_run(rom.clone(), &config);
        for _ in 0..k {
            assert!(matches!(run.next().unwrap(), Some(LockstepStep::Event { .. })));
        }
        let partial = run.report();
        assert_eq!(partial.events, k, "after {k} steps");
        assert_eq!(partial.first_divergence, None);
        assert!(partial.frames < FRAMES);
        while run.next().unwrap().is_some() {}
        assert_eq!(run.report(), one_shot, "stopped after {k} steps");
    }
}

#[test]
fn slices_of_frames_give_the_same_steps_and_report() {
    let rom = nmi_race_rom();
    let config = config(race_table(race_leaf, RACE_LEAF_COST), RACE_IDLE);
    let mut whole = lockstep_run(rom.clone(), &config);
    let (expected, _) = steps(&mut whole, u64::MAX);
    for frames in [1, 2, 7] {
        let mut run = lockstep_run(rom.clone(), &config);
        let (sliced, _) = steps(&mut run, frames);
        assert_eq!(sliced, expected, "slices of {frames}");
        assert_eq!(run.report(), whole.report());
    }
    // 0 frames count as 1.
    let mut run = lockstep_run(rom, &config);
    let (sliced, _) = steps(&mut run, 0);
    assert_eq!(sliced, expected);
}

#[test]
fn unsynced_windows_are_steps_and_slices_go_on_inside_them() {
    let mut config = config(None, SYNC_IDLE);
    config.budget_frames = 50;
    let mut one_shot = lockstep_run(nmi_off_idle_rom(), &config);
    let (expected, running) = steps(&mut one_shot, u64::MAX);
    // Two full budgets, then the 20-frame tail (a step, but no counted window).
    let window = |frames| LockstepStep::UnsyncedWindow { frames, nmis: 0 };
    assert_eq!(expected, [window(50), window(50), window(20)]);
    assert_eq!(running, 0);
    assert_eq!(one_shot.report().unsynced_windows, 2);

    for frames in [1, 7, 49] {
        let mut run = lockstep_run(nmi_off_idle_rom(), &config);
        let (sliced, running) = steps(&mut run, frames);
        assert_eq!(sliced, expected, "slices of {frames}");
        assert_eq!(run.report(), one_shot.report());
        // A frames-long slice ends a window of n frames after ceil(n / frames) calls.
        let calls = |n: u64| n.div_ceil(frames);
        assert_eq!(running, calls(50) - 1 + calls(50) - 1 + calls(20) - 1, "slices of {frames}");
    }
}

#[test]
fn a_divergence_is_the_last_step() {
    let rom = nmi_race_rom();
    let config = config(race_table(race_leaf_wrong_from_the_fifth_nmi, RACE_LEAF_COST), RACE_IDLE);
    let one_shot = lockstep(rom.clone(), &config);
    let divergence = one_shot.first_divergence.clone().expect("diverges");
    for frames in [u64::MAX, 1] {
        let mut run = lockstep_run(rom.clone(), &config);
        let (steps, _) = steps(&mut run, frames);
        let Some((LockstepStep::Diverged(found), events)) = steps.split_last() else {
            panic!("ends with the divergence: {steps:?}");
        };
        assert_eq!(found, &divergence);
        assert_eq!(events.len() as u64, one_shot.events);
        assert!(events.iter().all(|step| matches!(step, LockstepStep::Event { .. })));
        assert_eq!(run.next().unwrap(), None);
        assert_eq!(run.next_within(5).unwrap(), None);
        assert_eq!(run.report(), one_shot);
    }
}

#[test]
fn both_machines_of_an_eventless_window_run_in_slices() {
    let mut config = config(None, SYNC_IDLE);
    config.budget_frames = 50;
    let mut run = lockstep_run(nmi_off_idle_rom(), &config);
    let mut before = (0, 0);
    let mut windows = Vec::new();
    loop {
        let step = run.next_within(7).unwrap();
        let now = (run.machine_a().frame_count(), run.machine_b().frame_count());
        assert!(now.0 - before.0 <= 7 && now.1 - before.1 <= 7, "{before:?} -> {now:?}");
        before = now;
        match step {
            Some(LockstepStep::Running { .. }) => {}
            Some(step) => windows.push(step),
            None => break,
        }
    }
    let window = |frames| LockstepStep::UnsyncedWindow { frames, nmis: 0 };
    assert_eq!(windows, [window(50), window(50), window(20)]);
    assert_eq!(run.report().unsynced_windows, 2);
}

// The real game

/// The canonical ROM from `SSD_ROM_PATH`.
fn real_rom() -> Vec<u8> {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let rom = ssd_rom::normalize(&std::fs::read(path).unwrap(), None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32, "not the canonical ROM");
    rom.bytes
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn real_rom_reference_replays_stay_in_lockstep_with_every_routine_in_rust() {
    let rom = real_rom();
    let symbols =
        Symbols::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml")).unwrap();
    let table = ssd_game::hook_table(&symbols, Some(HookMode::Rust)).unwrap();
    assert_eq!(
        table.len(),
        59,
        "the fifty-nine routines: the seven of M1, the fifteen of batches A to C, the five of batch D, the nine of batch E, the eleven of batch F and the twelve of batch G"
    );
    let config = LockstepConfig {
        table: Some(table),
        sync_keys: symbols.sync_points().iter().map(|point| point.address).collect(),
        budget_frames: DEFAULT_BUDGET_FRAMES,
        b_packs: None,
    };
    // The replays `make-replays` records: the scripts from power-on, blank SRAM.
    for (name, frames, script) in [
        ("battle", BATTLE_FRAMES, battle_input as fn(u64) -> Joypad),
        ("wander", WANDER_FRAMES, wander_input),
    ] {
        let replay = replay(&rom, frames, script);
        let report = run_lockstep(rom.clone(), &replay, &config, Some(&symbols)).unwrap();
        eprintln!(
            "{name}: {} events, {} frames, {} unsynced windows, max drift {}, widest dead stack {:X?}",
            report.events,
            report.frames,
            report.unsynced_windows,
            report.max_cycle_drift,
            report.widest_dead_stack
        );
        for (routine, stats) in &report.hook_stats {
            eprintln!(
                "  {routine}: calls {}, rust_runs {}, timing_fallback {}, cost_over_bound {}",
                stats.calls, stats.rust_runs, stats.timing_fallback, stats.cost_over_bound
            );
        }
        if let Some(divergence) = &report.first_divergence {
            panic!("{name}: {divergence}");
        }
        assert_eq!(report.verdict(), Verdict::Green, "{name}");
        assert_eq!(report.max_cycle_drift, 0, "{name}");
        assert_eq!(report.frames, frames, "{name}");
        for (routine, stats) in &report.hook_stats {
            assert_eq!(stats.cost_over_bound, 0, "{name}: {routine}");
            if stats.calls > 0 {
                assert!(stats.rust_runs > 0, "{name}: {routine} never ran in Rust");
            }
        }
    }
}
