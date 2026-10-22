//! `standalone-diff`: a replay played on two free-running machines, the original (no hooks) and
//! the standalone game (`--hooks rust`: every routine of `re/symbols.toml` in Rust, and with the
//! `recomp` feature the recompiled routines too), each fed the joypad of the replay and nothing
//! else. Lockstep (`verify-replays`) pins B to A's timeline at every sync event; this does not,
//! which is how the shipped game runs.
//!
//! B runs a frame at a time as the app does (`Machine::run_frame_with`); a Rust routine that
//! spans frames (a decompression with the screen off) renders them inside one call, so B can
//! come back several frames on. After each B frame A is brought to B's master cycle and the two
//! are compared there: master cycles, NMIs taken, registers, WRAM (the dead stack left out, as
//! lockstep does), VRAM, CGRAM, OAM and SRAM. The first stop that differs is reported with what
//! differs and the dispatches that rendered frames. Exit 0 when every stop is identical, 1 on the
//! first difference.

use crate::shared::read_replay;
use anyhow::Context;
use ssd_oracle::{HookMode, Machine, NullAudio, Stopped, Stops, Symbols, apply_events};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// Routines named in the line of a replay that is identical.
const SHOWN_ROUTINES: usize = 6;

/// What the run found.
#[derive(Debug)]
pub struct Outcome {
    /// The frame B had reached before the stop of the first difference, if there is one.
    pub first_difference: Option<u64>,
}

/// Counters of the stops A was brought to.
#[derive(Default)]
struct Totals {
    stops: u64,
    /// Stops where B was holding the CPU for the rest of a Rust call's charge: the routine had
    /// run, the original had not finished it yet (see `Machine::charge_pending`).
    pending_stops: u64,
    /// The widest gap between A's master cycle and B's at a stop (A runs on to B's, so A's is
    /// the same or a few cycles past it: the grain of A's ticks).
    max_gap: i64,
    /// Stops that held more than one frame (the frame counters then differ, which is a failure).
    multi_frame_stops: u64,
    /// The routines whose call rendered a frame, with the most frames one call rendered.
    spanning: BTreeMap<String, u64>,
}

/// Plays `replay` on a pure machine and on a standalone one, comparing them after each frame of
/// B. `verbose` prints a line per stop. With `cheats`, B also has the cheat wrappers of Enhanced mode installed
/// (`ssd_game::cheats`) at a neutral block: the shipped game with the cheats menu untouched must still equal
/// the original. With `mods`, B runs on that pack set's image (the empty set's, zero-padded to 2 MiB) with the
/// redirectors installed (`ssd_game::mods`): the same must hold.
///
/// # Errors
///
/// The replay cannot be read or a run fails.
pub fn run(
    rom: &[u8],
    symbols: &Symbols,
    replay_path: &Path,
    verbose: bool,
    cheats: bool,
    mods: Option<&ssd_mod::Image>,
    out: &mut dyn Write,
) -> anyhow::Result<Outcome> {
    let replay = read_replay(replay_path, rom)?;
    let mut table = ssd_game::hook_table(symbols, Some(HookMode::Rust))?;
    if cheats {
        ssd_game::cheats::install(&mut table);
    }
    if mods.is_some() {
        ssd_game::mods::install(&mut table);
    }
    let mut a = replay.power_on(rom.to_vec())?;
    let mut b = match mods {
        None => replay.power_on(rom.to_vec())?,
        Some(image) => {
            let mut on_packs = replay.clone();
            on_packs.packs = Some(crate::shared::pack_tag(image));
            on_packs.power_on_image(image.bytes.clone())?
        }
    };
    b.set_hook_table(Some(table));
    let name =
        replay_path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let stops = Stops { frame_end: true, after_dispatch: true, ..Stops::default() };
    let mut stack_floor = a.cpu_registers().s;
    let mut totals = Totals::default();
    let mut a_events_done = 0_u64;
    while b.frame_count() < replay.frames() {
        let frame_before = b.frame_count();
        apply_events(&mut b, replay.events_at(frame_before))?;
        // The same loop as `run_frame_with`, but told of the dispatches that render frames.
        let mut rendering = Vec::new();
        loop {
            let before = b.frame_count();
            let stopped = b
                .run(&stops, &mut |f| replay.input(f), &mut NullAudio, 1)
                .with_context(|| format!("{name}: machine B, frame {frame_before}"))?;
            match stopped {
                Stopped::Dispatched { name, rendered: true, .. } => {
                    let spanned = b.frame_count() - before;
                    let most = totals.spanning.entry(name.clone()).or_default();
                    *most = (*most).max(spanned);
                    rendering.push(format!("{name} (+{spanned} frames)"));
                    if b.frame_count() > frame_before {
                        break;
                    }
                }
                Stopped::Dispatched { .. } => {}
                _ => break,
            }
        }
        // A runs on to the master cycle B reached.
        while a.total_master_cycles() < b.total_master_cycles() {
            let frame = a.frame_count();
            if a_events_done <= frame && a.at_fetch_boundary() {
                apply_events(&mut a, replay.events_at(frame))?;
                a_events_done = frame + 1;
            }
            a.tick(replay.input(a.frame_count()), &mut NullAudio).context("machine A")?;
            stack_floor = stack_floor.min(a.cpu_registers().s);
        }
        totals.stops += 1;
        totals.multi_frame_stops += u64::from(b.frame_count() > frame_before + 1);
        let gap = a.total_master_cycles() as i64 - b.total_master_cycles() as i64;
        totals.max_gap = totals.max_gap.max(gap);
        let pending = b.charge_pending();
        totals.pending_stops += u64::from(pending);
        let diff = compare(&a, &b, pending, &mut stack_floor);
        if verbose {
            let drift = b.total_master_cycles().wrapping_sub(a.total_master_cycles()) as i64;
            let verdict = if diff.is_empty() { "same" } else { "DIFFERENT" };
            writeln!(
                out,
                "  frame {frame_before} -> {}: B - A cycles {drift}, {verdict}",
                b.frame_count()
            )?;
        }
        if !diff.is_empty() {
            let rendered =
                if rendering.is_empty() { "none".to_owned() } else { rendering.join(", ") };
            writeln!(
                out,
                "{name}: first difference at B's stop after frame {frame_before} (B frame {}, A frame {}); dispatches that rendered: {rendered}",
                b.frame_count(),
                a.frame_count(),
            )?;
            for line in &diff {
                writeln!(out, "  {line}")?;
            }
            return Ok(Outcome { first_difference: Some(frame_before) });
        }
    }
    // The recompiled blocks render frames too: name the first few only.
    let mut spanning: Vec<String> =
        totals.spanning.iter().map(|(name, frames)| format!("{name} x{frames}")).collect();
    let more = spanning.len().saturating_sub(SHOWN_ROUTINES);
    spanning.truncate(SHOWN_ROUTINES);
    let mut spanning = if spanning.is_empty() { "none".to_owned() } else { spanning.join(", ") };
    if more > 0 {
        spanning = format!("{spanning} and {more} more");
    }
    writeln!(
        out,
        "{name}: identical at {} stops over {} frames ({} inside the charge of a Rust call, compared without registers and WRAM; {} held several frames; A ends at most {} master cycles past B; routines whose call rendered a frame: {spanning})",
        totals.stops,
        b.frame_count(),
        totals.pending_stops,
        totals.multi_frame_stops,
        totals.max_gap,
    )?;
    Ok(Outcome { first_difference: None })
}

/// What differs between A and B; empty when nothing does. A is at B's master cycle or just past
/// it (by at most 12: the grain of A's ticks, as in `framecheck`).
///
/// With `pending` (B still holds the CPU for the rest of a Rust call's charge) the registers and
/// WRAM are not compared: the routine has run whole and written its results, which the original
/// writes over the rest of the charge. What the picture is made of (VRAM, CGRAM, OAM), the
/// frame, the NMIs and the master cycle are compared all the same.
fn compare(a: &Machine, b: &Machine, pending: bool, stack_floor: &mut u16) -> Vec<String> {
    let mut lines = Vec::new();
    let cycles = b.total_master_cycles().wrapping_sub(a.total_master_cycles()) as i64;
    if !(-12..=0).contains(&cycles) {
        lines.push(format!(
            "master cycles: A {} B {} (B - A = {cycles})",
            a.total_master_cycles(),
            b.total_master_cycles()
        ));
    }
    if a.frame_count() != b.frame_count() {
        lines.push(format!("frames rendered: A {} B {}", a.frame_count(), b.frame_count()));
    }
    if a.nmi_count() != b.nmi_count() {
        lines.push(format!("NMIs taken: A {} B {}", a.nmi_count(), b.nmi_count()));
    }
    let (ra, rb) = (a.cpu_registers(), b.cpu_registers());
    if !pending && format!("{ra:?}") != format!("{rb:?}") {
        lines.push(format!("registers:\n    A {ra:?}\n    B {rb:?}"));
    }
    if !pending && a.is_mid_instruction() != b.is_mid_instruction() {
        lines.push("one machine is between instruction cycles, the other not".to_owned());
    }
    let (wa, wb) = (a.wram(), b.wram());
    let dead = usize::from(*stack_floor) + 1..=usize::from(ra.s);
    let differs = |i: &usize| wa[*i] != wb[*i];
    let all: Vec<usize> = (0..wa.len()).filter(differs).collect();
    let live: Vec<usize> = all.iter().copied().filter(|i| !dead.contains(i)).collect();
    // The dead stack heals as in lockstep: it rises to just below the lowest byte still differing.
    *stack_floor = dead.clone().find(differs).map_or(ra.s, |i| (i - 1) as u16);
    if !pending && !live.is_empty() {
        let shown: Vec<String> = live
            .iter()
            .take(24)
            .map(|&i| format!("{i:05X}: A={:02X} B={:02X}", wa[i], wb[i]))
            .collect();
        lines.push(format!("WRAM: {} bytes differ; {}", live.len(), shown.join(", ")));
    }
    let words = |label: &str, x: &[u16], y: &[u16], lines: &mut Vec<String>| {
        let count = x.iter().zip(y).filter(|(p, q)| p != q).count();
        if count > 0 {
            let first = x.iter().zip(y).position(|(p, q)| p != q).unwrap_or(0);
            lines.push(format!("{label}: {count} words differ, first at {first:04X}"));
        }
    };
    words("VRAM", a.vram(), b.vram(), &mut lines);
    words("CGRAM", a.cgram(), b.cgram(), &mut lines);
    if a.oam() != b.oam() {
        lines.push("OAM differs".to_owned());
    }
    if a.sram() != b.sram() {
        lines.push("SRAM differs".to_owned());
    }
    lines
}
