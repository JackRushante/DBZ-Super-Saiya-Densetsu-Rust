//! `probe`: what is needed to write the Rust version of a routine, and the check that it
//! matches.
//!
//! `probe <symbol>` (or `probe --address A --returns rts|rtl`) plays the reference replays with a
//! counting hook at the routine's entry (nothing is replaced: the original always runs) and
//! reports, over every call: where it was called from, the registers and widths at entry, the
//! registers at exit, the cost in master cycles, the WRAM and register writes (from the CPU write
//! log; stack pushes left out), and a few full samples with the values of the WRAM the code
//! reads. Reads come from the static listing of the routine (the direct operands), as the core
//! has no read log.
//!
//! `probe <symbol> --verify` checks the Rust function of `ssd-game`: first every call of every
//! replay in `Verify` (sample 1) with only that routine hooked, then the whole-run lockstep with
//! only that routine in `Rust` (`--quick` skips it). It exits 0 when everything matched, 1 on a
//! divergence or a cost problem, 2 when no replay called the routine.

use crate::disasm::{self, Namer, Options, Region, RomImage, data_operand, region};
use crate::shared::{read_replay, render_table};
use crate::verify_replays::{self, Hooks};
use anyhow::{Context, bail};
use ssd_oracle::{
    HookEntry, HookMode, HookStats, HookTable, LockstepConfig, Machine, NullAudio, Registers,
    Replay, ReturnKind, Stopped, Stops, Symbols, hook_key,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// What to probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Symbol(String),
    Address { address: u32, returns: ReturnKind, name: Option<String> },
}

/// The command line of `probe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub target: Target,
    pub verify: bool,
    /// With `--verify`: skip the whole-run lockstep.
    pub quick: bool,
    pub dir: Option<PathBuf>,
    pub samples: usize,
    /// Calls costing more master cycles than this are counted.
    pub cycles: Option<u64>,
    /// Frames a call may take before it counts as without a return (3 by default).
    pub frames: u64,
    pub m8: Option<bool>,
    pub x8: Option<bool>,
}

fn parse_bool(text: &str) -> Option<bool> {
    match text {
        "true" | "yes" | "8" => Some(true),
        "false" | "no" | "16" => Some(false),
        _ => None,
    }
}

/// Frames a probed call may take by default.
const DEFAULT_FRAMES: u64 = 3;

/// Parses `<symbol> | --address <24-bit> --returns rts|rtl [--name n]`, then `[--entry-m8 b]
/// [--entry-x8 b] [--cycles N] [--frames N] [--samples N] [--dir D] [--verify] [--quick]`.
///
/// # Errors
///
/// An unknown option, a missing or bad value, or no target.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let (mut symbol, mut address, mut returns, mut name) = (None, None, None, None);
    let mut parsed = Args {
        target: Target::Symbol(String::new()),
        verify: false,
        quick: false,
        dir: None,
        samples: 6,
        cycles: None,
        frames: DEFAULT_FRAMES,
        m8: None,
        x8: None,
    };
    let mut it = args.iter();
    let value = |it: &mut std::slice::Iter<'_, &str>, flag: &str| {
        it.next().map(|v| (*v).to_owned()).ok_or(format!("{flag} needs a value"))
    };
    while let Some(&arg) = it.next() {
        match arg {
            "--verify" => parsed.verify = true,
            "--quick" => parsed.quick = true,
            "--address" => {
                let v = value(&mut it, arg)?;
                address = Some(disasm::parse_address(&v).map_err(|e| format!("{e:#}"))?);
            }
            "--returns" => {
                returns = Some(match value(&mut it, arg)?.as_str() {
                    "rts" => ReturnKind::Rts,
                    "rtl" => ReturnKind::Rtl,
                    other => return Err(format!("--returns takes rts or rtl, not `{other}`")),
                });
            }
            "--name" => name = Some(value(&mut it, arg)?),
            "--m8" => parsed.m8 = Some(true),
            "--m16" => parsed.m8 = Some(false),
            "--x8" => parsed.x8 = Some(true),
            "--x16" => parsed.x8 = Some(false),
            "--entry-m8" => {
                let v = value(&mut it, arg)?;
                parsed.m8 = Some(parse_bool(&v).ok_or(format!("bad {arg} `{v}`"))?);
            }
            "--entry-x8" => {
                let v = value(&mut it, arg)?;
                parsed.x8 = Some(parse_bool(&v).ok_or(format!("bad {arg} `{v}`"))?);
            }
            "--cycles" => {
                let v = value(&mut it, arg)?;
                parsed.cycles = Some(v.parse().map_err(|_| format!("bad --cycles `{v}`"))?);
            }
            "--frames" => {
                let v = value(&mut it, arg)?;
                parsed.frames =
                    v.parse().ok().filter(|&n| n > 0).ok_or(format!("bad --frames `{v}`"))?;
            }
            "--samples" => {
                let v = value(&mut it, arg)?;
                parsed.samples = v.parse().map_err(|_| format!("bad --samples `{v}`"))?;
            }
            "--dir" => parsed.dir = Some(PathBuf::from(value(&mut it, arg)?)),
            option if option.starts_with("--") => return Err(format!("unknown option `{option}`")),
            sym if symbol.is_none() => symbol = Some(sym.to_owned()),
            other => return Err(format!("one symbol only (`{other}` is a second one)")),
        }
    }
    parsed.target = match (symbol, address, returns) {
        (Some(s), None, None) => Target::Symbol(s),
        (None, Some(address), Some(returns)) => Target::Address { address, returns, name },
        (None, Some(_), None) => return Err("--address needs --returns rts|rtl".to_owned()),
        (None, None, _) => return Err("needs a symbol or --address with --returns".to_owned()),
        (Some(_), ..) => return Err("a symbol or --address, not both".to_owned()),
    };
    if parsed.verify && matches!(parsed.target, Target::Address { .. }) {
        return Err("--verify needs a symbol of re/symbols.toml with a Rust function".to_owned());
    }
    Ok(parsed)
}

/// What the probe knows about the routine to watch.
#[derive(Debug, Clone)]
pub struct Subject {
    pub name: String,
    pub address: u32,
    pub returns: ReturnKind,
    pub entry_m8: Option<bool>,
    pub entry_x8: Option<bool>,
    pub cycles: Option<u64>,
    /// Frames a call may take before it counts as without a return.
    pub frames: u64,
}

/// Resolves the target against the symbols.
///
/// # Errors
///
/// A symbol that is not a routine.
pub fn subject(args: &Args, symbols: &Symbols) -> anyhow::Result<Subject> {
    let mut subject = match &args.target {
        Target::Symbol(name) => {
            let r = symbols.routines().iter().find(|r| r.name == *name).with_context(|| {
                format!("`{name}` is not a [[routine]] of re/symbols.toml (use --address)")
            })?;
            Subject {
                name: r.name.clone(),
                address: r.address,
                returns: r.returns,
                entry_m8: r.entry_m8,
                entry_x8: r.entry_x8,
                cycles: r.cycles,
                frames: args.frames,
            }
        }
        Target::Address { address, returns, name } => {
            let known = symbols.routine_for_key(*address);
            Subject {
                name: name
                    .clone()
                    .or_else(|| known.map(|r| r.name.clone()))
                    .unwrap_or_else(|| format!("routine_{address:06x}")),
                address: hook_key(*address),
                returns: *returns,
                entry_m8: known.and_then(|r| r.entry_m8),
                entry_x8: known.and_then(|r| r.entry_x8),
                cycles: known.and_then(|r| r.cycles),
                frames: args.frames,
            }
        }
    };
    subject.entry_m8 = args.m8.or(subject.entry_m8);
    subject.entry_x8 = args.x8.or(subject.entry_x8);
    subject.cycles = args.cycles.or(subject.cycles);
    Ok(subject)
}

/// The values a register took at entry or exit.
#[derive(Debug, Default, Clone)]
pub struct RegStat {
    pub min: Option<u16>,
    pub max: Option<u16>,
    pub distinct: BTreeSet<u16>,
    pub overflow: bool,
}

const DISTINCT_CAP: usize = 12;

impl RegStat {
    fn add(&mut self, value: u16) {
        self.min = Some(self.min.map_or(value, |m| m.min(value)));
        self.max = Some(self.max.map_or(value, |m| m.max(value)));
        if self.distinct.len() < DISTINCT_CAP {
            self.distinct.insert(value);
        } else if !self.distinct.contains(&value) {
            self.overflow = true;
        }
    }

    fn text(&self, width: usize) -> String {
        let (Some(min), Some(max)) = (self.min, self.max) else { return "-".to_owned() };
        let value = |v: u16| format!("{v:0width$X}");
        let range = if min == max { value(min) } else { format!("{}..{}", value(min), value(max)) };
        let list: Vec<String> = self.distinct.iter().map(|&v| value(v)).collect();
        if self.overflow {
            format!("{range} (more than {DISTINCT_CAP} values)")
        } else if self.distinct.len() > 1 {
            format!("{range} ({})", list.join(" "))
        } else {
            range
        }
    }
}

/// A written byte over the calls.
#[derive(Debug, Default, Clone)]
pub struct WriteStat {
    /// Calls that wrote it.
    pub calls: u64,
    pub values: BTreeSet<u8>,
    last_call: u64,
}

/// One call in full.
#[derive(Debug, Clone)]
pub struct Sample {
    pub replay: String,
    pub frame: u64,
    pub site: u32,
    pub regs_in: Registers,
    pub regs_out: Registers,
    pub cost: u64,
    pub writes: Vec<(u32, u8)>,
    /// Low WRAM (`$7E:0000-$1FFF`) at entry.
    pub wram: Vec<u8>,
}

/// What the replays showed.
#[derive(Debug, Default)]
pub struct Probe {
    pub calls: u64,
    pub interrupted: u64,
    pub no_return: u64,
    /// Arrivals at the entry inside a call (recursion).
    pub nested: u64,
    pub per_replay: Vec<(String, u64, u64)>,
    pub sites: BTreeMap<u32, u64>,
    /// `(m8, x8, e)` at entry.
    pub modes: BTreeMap<(bool, bool, bool), u64>,
    pub dps: BTreeSet<u16>,
    pub dbrs: BTreeSet<u8>,
    pub entry: [RegStat; 3],
    pub exit: [RegStat; 3],
    /// How many calls changed A, X, Y, P, D, DBR (in that order).
    pub changed: [u64; 6],
    pub costs: BTreeMap<u64, u64>,
    pub over_cycles: u64,
    /// Clean calls whose real cost had no timing event inside its window at the entry, so that
    /// `Rust` mode would run them atomically given an exact per-call bound (see
    /// `ssd_oracle::call_fits_window`).
    pub window_fit: u64,
    pub wram_writes: BTreeMap<u32, WriteStat>,
    pub mmio_writes: BTreeMap<u16, u64>,
    pub other_writes: BTreeMap<u32, u64>,
    /// Calls with a write to the stack area under S, left out of the writes.
    pub stack_writes: u64,
    pub samples: Vec<Sample>,
}

/// Bytes under S that count as the stack of a call.
const STACK_WINDOW: u16 = 0x40;

fn full_pc(r: &Registers) -> u32 {
    (u32::from(r.pbr) << 16) | u32::from(r.pc)
}

/// Plays `replay` on `rom` with a counting hook at the subject's entry and adds what it sees to
/// `probe`.
///
/// # Errors
///
/// The replay cannot be played.
pub fn probe_replay(
    rom: &[u8],
    replay: &Replay,
    name: &str,
    subject: &Subject,
    samples: usize,
    probe: &mut Probe,
) -> anyhow::Result<()> {
    let mut machine = replay.power_on(rom.to_vec())?;
    let mut table = HookTable::new();
    table.insert(HookEntry::sync_point(subject.name.clone(), subject.address));
    machine.set_hook_table(Some(table));
    machine.set_cpu_write_log(true);
    let mut input = |frame: u64| replay.input(frame);
    let stops = Stops { after_dispatch: true, ..Stops::default() };
    let before = probe.calls;
    while machine.frame_count() < replay.frames() {
        let budget = replay.frames() - machine.frame_count();
        match machine.run(&stops, &mut input, &mut NullAudio, budget)? {
            Stopped::Dispatched { .. } => {
                if !track_call(&mut machine, replay, name, subject, samples, probe)? {
                    break;
                }
            }
            Stopped::Budget => break,
            _ => {}
        }
    }
    let stats = machine.hook_stats();
    let arrivals = stats.iter().find(|(n, _)| *n == subject.name).map_or(0, |(_, s)| s.calls);
    let top = probe.calls - before;
    probe.nested += arrivals.saturating_sub(top);
    probe.per_replay.push((name.to_owned(), top, replay.frames()));
    Ok(())
}

/// Follows one call from its entry to its return. Returns false at the end of the replay.
fn track_call(
    machine: &mut Machine,
    replay: &Replay,
    name: &str,
    subject: &Subject,
    samples: usize,
    probe: &mut Probe,
) -> anyhow::Result<bool> {
    let regs_in = machine.cpu_registers();
    let p_in = u8::from(regs_in.p);
    let s = regs_in.s;
    let read = |machine: &Machine, offset: u16| {
        machine.peek(u32::from(s.wrapping_add(offset))).map_or(0, u32::from)
    };
    let mut ret = (read(machine, 1) | (read(machine, 2) << 8)) + 1;
    ret = if subject.returns == ReturnKind::Rtl {
        (read(machine, 3) << 16) | (ret & 0xFFFF)
    } else {
        (u32::from(regs_in.pbr) << 16) | (ret & 0xFFFF)
    };
    let site_len = if subject.returns == ReturnKind::Rtl { 4 } else { 3 };
    let site = (ret & 0xFF_0000) | (ret.wrapping_sub(site_len) & 0xFFFF);
    probe.calls += 1;
    *probe.sites.entry(site).or_default() += 1;
    *probe
        .modes
        .entry((p_in & 0x20 != 0, p_in & 0x10 != 0, regs_in.emulation_mode))
        .or_default() += 1;
    probe.dps.insert(regs_in.d);
    probe.dbrs.insert(regs_in.dbr);
    probe.entry[0].add(regs_in.a);
    probe.entry[1].add(regs_in.x);
    probe.entry[2].add(regs_in.y);
    let wanted_sample = probe.samples.len() < samples;
    let snapshot = wanted_sample.then(|| machine.wram()[..0x2000].to_vec());
    machine.take_cpu_writes();
    let busy0 = machine.cpu_busy_cycles();
    let frame0 = machine.frame_count();
    // The timing window at the entry, as `Rust` mode would see it.
    let until_event = machine.master_cycles_until_timing_event();
    let gpdma = machine.gpdma_requested();
    let stops = Stops { returned_above: Some(s), interrupt: true, ..Stops::default() };
    let mut input = |frame: u64| replay.input(frame);
    let mut interrupted = false;
    let mut regs_out = None;
    loop {
        let left = replay.frames().saturating_sub(machine.frame_count()).min(subject.frames);
        if left == 0 {
            return Ok(false);
        }
        match machine.run(&stops, &mut input, &mut NullAudio, left)? {
            Stopped::Interrupt => interrupted = true,
            Stopped::Returned => {
                let r = machine.cpu_registers();
                if full_pc(&r) == ret {
                    regs_out = Some(r);
                    break;
                }
            }
            Stopped::Budget => break,
            _ => {}
        }
    }
    let _ = frame0;
    let Some(regs_out) = regs_out else {
        probe.no_return += 1;
        return Ok(machine.frame_count() < replay.frames());
    };
    let cost = machine.cpu_busy_cycles() - busy0;
    let writes = machine.take_cpu_writes();
    probe.interrupted += u64::from(interrupted);
    let p_out = u8::from(regs_out.p);
    probe.exit[0].add(regs_out.a);
    probe.exit[1].add(regs_out.x);
    probe.exit[2].add(regs_out.y);
    for (i, changed) in [
        regs_in.a != regs_out.a,
        regs_in.x != regs_out.x,
        regs_in.y != regs_out.y,
        p_in != p_out,
        regs_in.d != regs_out.d,
        regs_in.dbr != regs_out.dbr,
    ]
    .into_iter()
    .enumerate()
    {
        probe.changed[i] += u64::from(changed);
    }
    if interrupted {
        // The handler's cost and writes are in the numbers: leave them out.
        return Ok(true);
    }
    *probe.costs.entry(cost).or_default() += 1;
    probe.window_fit += u64::from(ssd_oracle::call_fits_window(until_event, gpdma, cost));
    probe.over_cycles += u64::from(subject.cycles.is_some_and(|max| cost > max));
    let mut own = Vec::new();
    let mut stack = false;
    for &(bus, value) in &writes {
        match region(bus) {
            Region::Wram(offset) => {
                if (bus >> 16) & 0x7F == 0 && offset < 0x2000 && {
                    let o = offset as u16;
                    o <= s && o >= s.saturating_sub(STACK_WINDOW)
                } {
                    stack = true;
                    continue;
                }
                let stat = probe.wram_writes.entry(0x7E_0000 + offset).or_default();
                if stat.calls == 0 || stat.last_call != probe.calls {
                    stat.calls += 1;
                    stat.last_call = probe.calls;
                }
                stat.values.insert(value);
                own.push((0x7E_0000 + offset, value));
            }
            Region::Mmio(register) => {
                *probe.mmio_writes.entry(register).or_default() += 1;
                own.push((bus, value));
            }
            _ => {
                *probe.other_writes.entry(bus).or_default() += 1;
                own.push((bus, value));
            }
        }
    }
    probe.stack_writes += u64::from(stack);
    if let Some(wram) = snapshot {
        // Samples differ by what goes in and out: registers and the writes with their values.
        let seen = probe.samples.iter().any(|s| {
            s.regs_in.a == regs_in.a
                && s.regs_in.x == regs_in.x
                && s.regs_in.y == regs_in.y
                && s.regs_out.a == regs_out.a
                && s.regs_out.x == regs_out.x
                && s.regs_out.y == regs_out.y
                && s.writes == own
        });
        if !seen {
            probe.samples.push(Sample {
                replay: name.to_owned(),
                frame: machine.frame_count(),
                site,
                regs_in,
                regs_out,
                cost,
                writes: own,
                wram,
            });
        }
    }
    Ok(true)
}

/// The direct WRAM bytes the routine's listing reads: `(bus address, size)`.
fn static_reads(listing: &disasm::Listing, options: &Options) -> Vec<(u32, u32)> {
    let mut reads = BTreeSet::new();
    for line in &listing.lines {
        let instr = &line.instr;
        let reading = matches!(
            instr.mnemonic,
            "LDA"
                | "LDX"
                | "LDY"
                | "ADC"
                | "SBC"
                | "AND"
                | "ORA"
                | "EOR"
                | "CMP"
                | "CPX"
                | "CPY"
                | "BIT"
                | "INC"
                | "DEC"
                | "ASL"
                | "LSR"
                | "ROL"
                | "ROR"
                | "TSB"
                | "TRB"
        );
        if !reading {
            continue;
        }
        let Some(op) = data_operand(instr, options.dbr, options.dp) else { continue };
        if op.indexed {
            continue;
        }
        if let Region::Wram(offset) = region(op.bus) {
            let wide = match instr.mnemonic {
                "LDX" | "LDY" | "CPX" | "CPY" => !line.widths.x8,
                _ => !line.widths.m8,
            };
            reads.insert((0x7E_0000 + offset, if wide { 2 } else { 1 }));
        }
    }
    reads.into_iter().collect()
}

fn regs_text(r: &Registers) -> String {
    format!(
        "A={:04X} X={:04X} Y={:04X} S={:04X} D={:04X} DBR={:02X} P={:02X}{}",
        r.a,
        r.x,
        r.y,
        r.s,
        r.d,
        r.dbr,
        u8::from(r.p),
        if r.emulation_mode { " E" } else { "" }
    )
}

fn flags_text(p: u8) -> String {
    "NVMXDIZC"
        .chars()
        .zip((0..8).rev())
        .map(|(c, bit)| if (p >> bit) & 1 != 0 { c } else { c.to_ascii_lowercase() })
        .collect()
}

/// Merges consecutive addresses into `start-end` runs.
fn runs(addresses: &[u32]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for &a in addresses {
        match out.last_mut() {
            Some(last) if a == last.1 + 1 => last.1 = a,
            _ => out.push((a, a)),
        }
    }
    out
}

/// The report of a finished probe.
#[must_use]
pub fn report(
    probe: &Probe,
    subject: &Subject,
    listing: &disasm::Listing,
    options: &Options,
    rom: &RomImage<'_>,
    namer: &Namer<'_>,
) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "probe {} at {:06X} ({}): {} calls, {} interrupted, {} without a return, {} nested",
        subject.name,
        subject.address,
        if subject.returns == ReturnKind::Rts { "rts" } else { "rtl" },
        probe.calls,
        probe.interrupted,
        probe.no_return,
        probe.nested
    );
    for (name, calls, frames) in &probe.per_replay {
        let _ = writeln!(out, "  {name}: {calls} calls in {frames} frames");
    }
    if probe.calls == 0 {
        let _ = writeln!(out, "no replay called it: record one that does (xtask make-replays)");
        return out;
    }
    let modes: Vec<String> = probe
        .modes
        .iter()
        .map(|(&(m8, x8, e), n)| {
            format!(
                "M{} X{}{} x{n}",
                if m8 { 8 } else { 16 },
                if x8 { 8 } else { 16 },
                if e { " E" } else { "" }
            )
        })
        .collect();
    let _ = writeln!(out, "entry widths: {}", modes.join(", "));
    let hex =
        |s: &BTreeSet<u16>| s.iter().map(|v| format!("{v:04X}")).collect::<Vec<_>>().join(" ");
    let _ = writeln!(out, "direct page: {}   data bank: {}", hex(&probe.dps), {
        probe.dbrs.iter().map(|v| format!("{v:02X}")).collect::<Vec<_>>().join(" ")
    });
    let sites: Vec<String> = {
        let mut v: Vec<(&u32, &u64)> = probe.sites.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        v.iter().take(12).map(|(a, n)| format!("{a:06X} x{n}")).collect()
    };
    let _ = writeln!(
        out,
        "call sites ({} distinct, the JSR/JSL address): {}",
        probe.sites.len(),
        sites.join(", ")
    );
    for (label, stat) in ["A", "X", "Y"].iter().zip(&probe.entry) {
        let _ = writeln!(out, "entry {label}: {}", stat.text(4));
    }
    for (label, stat) in ["A", "X", "Y"].iter().zip(&probe.exit) {
        let _ = writeln!(out, "exit  {label}: {}", stat.text(4));
    }
    let names = ["A", "X", "Y", "P", "D", "DBR"];
    let changed: Vec<String> =
        names.iter().zip(&probe.changed).map(|(n, c)| format!("{n} {c}")).collect();
    let _ = writeln!(out, "calls that changed a register: {}", changed.join(", "));
    let total: u64 = probe.costs.values().sum();
    if total > 0 {
        let weighted: u64 = probe.costs.iter().map(|(c, n)| c * n).sum();
        let hist: Vec<String> =
            probe.costs.iter().take(16).map(|(c, n)| format!("{c} x{n}")).collect();
        let _ = writeln!(
            out,
            "cost in CPU-busy master cycles over {total} clean calls: min {} max {} mean {:.1}; {}{}",
            probe.costs.keys().next().unwrap(),
            probe.costs.keys().next_back().unwrap(),
            weighted as f64 / total as f64,
            hist.join(", "),
            if probe.costs.len() > 16 { ", ..." } else { "" }
        );
        if let Some(max) = subject.cycles {
            let _ = writeln!(out, "declared cycles {max}: {} calls cost more", probe.over_cycles);
        }
    }
    // What a per-call bound could unlock: the calls that run clear of timing events. The rest
    // span an NMI, an IRQ or HDMA and fall back to the original in Rust mode, whatever the bound.
    let _ = writeln!(
        out,
        "timing window (exact per-call bound): {} of {} calls fit ({:.1}%); {} clean calls span a timing event, {} interrupted, {} without a return in {} frames",
        probe.window_fit,
        probe.calls,
        100.0 * probe.window_fit as f64 / probe.calls as f64,
        total - probe.window_fit,
        probe.interrupted,
        probe.no_return,
        subject.frames,
    );
    let _ = writeln!(
        out,
        "writes (CPU write log, clean calls; pushes to the {STACK_WINDOW} bytes under S left out, {} calls had some):",
        probe.stack_writes
    );
    let addresses: Vec<u32> = probe.wram_writes.keys().copied().collect();
    if addresses.is_empty() {
        let _ = writeln!(out, "  no WRAM writes");
    }
    for (start, end) in runs(&addresses) {
        let mut line = format!("  wram {start:06X}");
        if end != start {
            let _ = write!(line, "-{end:06X}");
        }
        if let Some(name) = namer.data(start) {
            let _ = write!(line, " ({name})");
        }
        let _ = writeln!(out, "{line}");
        let bytes: Vec<String> = (start..=end)
            .map(|a| {
                let s = &probe.wram_writes[&a];
                let values: Vec<String> =
                    s.values.iter().take(4).map(|v| format!("{v:02X}")).collect();
                let more = if s.values.len() > 4 { ",.." } else { "" };
                format!("{a:06X}: {} calls, values {}{more}", s.calls, values.join(","))
            })
            .collect();
        for b in bytes.iter().take(8) {
            let _ = writeln!(out, "      {b}");
        }
        if bytes.len() > 8 {
            let _ = writeln!(out, "      ... {} more bytes", bytes.len() - 8);
        }
    }
    for (register, n) in &probe.mmio_writes {
        let _ = writeln!(out, "  register ${register:04X}: {n} writes");
    }
    for (bus, n) in &probe.other_writes {
        let _ = writeln!(out, "  other {bus:06X}: {n} writes");
    }
    let reads = static_reads(listing, options);
    if reads.is_empty() {
        let _ = writeln!(out, "static direct WRAM reads: none");
    } else {
        let text: Vec<String> = reads
            .iter()
            .map(|&(a, size)| {
                let name = namer.data(a).map_or_else(String::new, |n| format!(" {n}"));
                format!("{a:06X}{}{name}", if size == 2 { "w" } else { "" })
            })
            .collect();
        let _ = writeln!(out, "static direct WRAM reads (w = 16 bit): {}", text.join(", "));
    }
    for (i, s) in probe.samples.iter().enumerate() {
        let _ = writeln!(
            out,
            "sample {}: {} frame {} from {:06X}, {} cycles",
            i + 1,
            s.replay,
            s.frame,
            s.site,
            s.cost
        );
        let _ = writeln!(
            out,
            "  in : {}  [{}]",
            regs_text(&s.regs_in),
            flags_text(u8::from(s.regs_in.p))
        );
        let _ = writeln!(
            out,
            "  out: {}  [{}]",
            regs_text(&s.regs_out),
            flags_text(u8::from(s.regs_out.p))
        );
        if !reads.is_empty() {
            let values: Vec<String> = reads
                .iter()
                .map(|&(a, size)| {
                    let o = (a - 0x7E_0000) as usize;
                    let bytes: String = (0..size as usize)
                        .rev()
                        .map(|i| s.wram.get(o + i).map_or("??".to_owned(), |b| format!("{b:02X}")))
                        .collect();
                    format!("{a:06X}={bytes}")
                })
                .collect();
            let _ = writeln!(out, "  reads at entry: {}", values.join(" "));
        }
        let writes: Vec<String> =
            s.writes.iter().map(|&(a, v)| format!("{a:06X}={v:02X}")).collect();
        let _ = writeln!(
            out,
            "  writes: {}",
            if writes.is_empty() { "none".to_owned() } else { writes.join(" ") }
        );
    }
    let _ = rom;
    out
}

/// The `*.ssdreplay` files of the directory, with their names.
fn replays(rom: &[u8], dir: &Path) -> anyhow::Result<Vec<(String, Replay)>> {
    verify_replays::replay_paths(dir)?
        .into_iter()
        .map(|path| {
            let name =
                path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            Ok((name, read_replay(&path, rom)?))
        })
        .collect()
}

/// Runs the probe over the replays of `dir` and returns the report.
///
/// # Errors
///
/// A replay cannot be read or played.
pub fn run_probe(
    rom: &[u8],
    symbols: &Symbols,
    subject: &Subject,
    args: &Args,
    dir: &Path,
    observed: &dyn Fn(u32) -> Option<(bool, bool)>,
) -> anyhow::Result<String> {
    let mut probe = Probe::default();
    let replays = replays(rom, dir)?;
    if replays.is_empty() {
        bail!("no *.ssdreplay in {}", dir.display());
    }
    for (name, replay) in &replays {
        probe_replay(rom, replay, name, subject, args.samples, &mut probe)?;
    }
    let options = Options { m8: args.m8, x8: args.x8, ..Options::default() };
    let image = RomImage(rom);
    let known = symbols.routine_for_key(subject.address).map(|r| (r.entry_m8, r.entry_x8));
    let (entry, source) = disasm::entry_widths(&options, known, observed(subject.address));
    let mut listing = disasm::disassemble(&image, subject.address, entry, &options, observed);
    listing.entry_source = source;
    let namer = Namer { symbols: Some(symbols) };
    let mut text = report(&probe, subject, &listing, &options, &image, &namer);
    let _ = writeln!(text, "\nlisting (entry widths from {source}):");
    text.push_str(&disasm::render(
        &listing,
        &image,
        &namer,
        &options,
        &format!("{} {:06X}", subject.name, subject.address),
    ));
    Ok(text)
}

/// The counters of the `Verify` pass over the replays, as a table.
fn verify_table(rows: &[(String, HookStats)]) -> String {
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|(name, s)| {
            vec![
                name.clone(),
                s.calls.to_string(),
                s.verified_ok.to_string(),
                s.divergences.to_string(),
                s.cost_mismatch.to_string(),
                s.cost_missing.to_string(),
                s.cost_over_bound.to_string(),
                s.interrupted.to_string(),
                s.contract_mismatch.to_string(),
                s.sampled_out.to_string(),
                s.no_return.to_string(),
            ]
        })
        .collect();
    render_table(
        &[
            "replay",
            "calls",
            "verified",
            "divergences",
            "cost_mismatch",
            "cost_missing",
            "cost_over_bound",
            "interrupted",
            "contract_mismatch",
            "sampled_out",
            "no_return",
        ],
        &rows,
    )
}

/// `--verify`: the routine alone in `Verify`, then in lockstep in `Rust`. Returns the exit code.
///
/// # Errors
///
/// The routine has no Rust function, or a replay cannot be played.
pub fn verify(
    rom: &[u8],
    symbols: &Symbols,
    name: &str,
    dir: &Path,
    quick: bool,
    out: &mut dyn std::io::Write,
) -> anyhow::Result<u8> {
    let full = ssd_game::hook_table(symbols, None)?;
    let entry = full
        .entries()
        .find(|e| e.name == name)
        .cloned()
        .with_context(|| format!("`{name}` is not a [[routine]] of re/symbols.toml"))?;
    let replays = replays(rom, dir)?;
    anyhow::ensure!(!replays.is_empty(), "no *.ssdreplay in {}", dir.display());
    let verify_entry = HookEntry { mode: HookMode::Verify, verify_sample: 1, ..entry.clone() };
    let mut table = HookTable::new();
    table.insert(verify_entry);
    let symbols_arc = std::sync::Arc::new(symbols.clone());
    let mut rows = Vec::new();
    let mut total = HookStats::default();
    let mut first_divergences = Vec::new();
    writeln!(out, "probe --verify {name}: every call of every replay in Verify (sample 1)")?;
    for (replay_name, replay) in &replays {
        let mut machine = replay.power_on(rom.to_vec())?;
        machine.set_hook_table(Some(table.clone()));
        machine.set_symbols(Some(std::sync::Arc::clone(&symbols_arc)));
        machine.set_divergence_dir(None);
        while machine.frame_count() < replay.frames() {
            machine.run_frame_with(&mut |frame| replay.input(frame), &mut NullAudio)?;
        }
        let stats = machine
            .hook_stats()
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, s)| s)
            .unwrap_or_default();
        total.calls += stats.calls;
        total.verified_ok += stats.verified_ok;
        total.divergences += stats.divergences;
        total.cost_mismatch += stats.cost_mismatch;
        total.cost_missing += stats.cost_missing;
        total.cost_over_bound += stats.cost_over_bound;
        total.instructions_mismatch += stats.instructions_mismatch;
        total.window_fit += stats.window_fit;
        total.interrupted += stats.interrupted;
        total.no_return += stats.no_return;
        first_divergences
            .extend(machine.divergences().iter().take(2).map(|d| format!("{replay_name}: {d}")));
        rows.push((replay_name.clone(), stats));
    }
    write!(out, "{}", verify_table(&rows))?;
    for d in &first_divergences {
        for line in d.lines() {
            writeln!(out, "  {line}")?;
        }
    }
    let called = total.calls > 0;
    let clean = total.divergences == 0
        && total.cost_mismatch == 0
        && total.cost_missing == 0
        && total.cost_over_bound == 0
        && total.instructions_mismatch == 0;
    writeln!(
        out,
        "verify pass: {} calls, {} verified, {} divergences, {} cost_mismatch, {} cost_missing, {} instructions_mismatch;          {} interrupted, {} no_return, {} fit the timing window",
        total.calls,
        total.verified_ok,
        total.divergences,
        total.cost_mismatch,
        total.cost_missing,
        total.instructions_mismatch,
        total.interrupted,
        total.no_return,
        total.window_fit
    )?;
    let mut code = if !called { 2 } else { u8::from(!clean) };
    if !quick && called {
        writeln!(out, "\nlockstep pass: the routine alone in Rust, whole replays")?;
        let rust_entry = HookEntry { mode: HookMode::Rust, ..entry };
        let mut table = HookTable::new();
        table.insert(rust_entry);
        let config = LockstepConfig {
            table: Some(table),
            sync_keys: symbols.sync_points().iter().map(|p| p.address).collect(),
            budget_frames: ssd_oracle::DEFAULT_BUDGET_FRAMES,
            b_packs: None,
        };
        let lockstep = verify_replays::verify(rom, symbols, &config, Hooks::Rust, dir, out)?;
        code = match (code, lockstep) {
            (1, _) | (_, 1) => 1,
            (2, _) | (_, 2) => 2,
            _ => 0,
        };
    }
    writeln!(
        out,
        "probe --verify {name}: {}",
        match code {
            0 => "OK",
            1 => "FAILED",
            _ => "UNVERIFIED (not called, or never run in Rust)",
        }
    )?;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{LEAF, leaf_rom};
    use ssd_oracle::{MemorySaveStore, Recorder};

    /// A replay of the fixture ROM, `frames` frames without input.
    fn fixture_replay(rom: &[u8], frames: u64) -> Replay {
        let mut machine = Machine::new(rom.to_vec(), Box::new(MemorySaveStore::default())).unwrap();
        let mut recorder = Recorder::start(&machine).unwrap();
        while machine.frame_count() < frames {
            recorder
                .run_frame(&mut machine, ssd_oracle::Joypad::default(), &mut NullAudio)
                .unwrap();
        }
        recorder.into_replay()
    }

    fn subject() -> Subject {
        Subject {
            name: "fixture_leaf".to_owned(),
            address: LEAF,
            returns: ReturnKind::Rts,
            entry_m8: Some(true),
            entry_x8: Some(false),
            cycles: None,
            frames: 3,
        }
    }

    #[test]
    fn the_arguments_take_a_symbol_or_an_address_with_its_return() {
        let a =
            parse_args(&["foo", "--verify", "--quick", "--samples", "3", "--dir", "d"]).unwrap();
        assert_eq!(a.target, Target::Symbol("foo".to_owned()));
        assert!(a.verify && a.quick);
        assert_eq!((a.samples, a.dir), (3, Some(PathBuf::from("d"))));
        let a = parse_args(&[
            "--address",
            "05B572",
            "--returns",
            "rtl",
            "--name",
            "x",
            "--entry-m8",
            "true",
            "--entry-x8",
            "false",
            "--cycles",
            "99",
        ])
        .unwrap();
        assert_eq!(
            a.target,
            Target::Address {
                address: 0x05_B572,
                returns: ReturnKind::Rtl,
                name: Some("x".to_owned())
            }
        );
        assert_eq!((a.m8, a.x8, a.cycles), (Some(true), Some(false), Some(99)));
        assert_eq!(a.frames, 3, "the default budget of a probed call");
        assert_eq!(parse_args(&["a", "--frames", "12"]).unwrap().frames, 12);
        for bad in [
            &[][..],
            &["a", "--address", "05B572", "--returns", "rts"],
            &["--address", "05B572"],
            &["--address", "05B572", "--returns", "jmp"],
            &["--address", "05B572", "--returns", "rts", "--verify"],
            &["a", "b"],
            &["a", "--cycles", "x"],
            &["a", "--frames", "0"],
            &["a", "--frames", "x"],
        ] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_probe_counts_the_calls_of_the_fixture_leaf_and_what_it_writes() {
        let rom = leaf_rom();
        let replay = fixture_replay(&rom, 6);
        let mut probe = Probe::default();
        probe_replay(&rom, &replay, "fixture", &subject(), 2, &mut probe).unwrap();
        assert!(probe.calls > 100, "{} calls", probe.calls);
        assert_eq!(probe.no_return, 0);
        // The only call site is the JSR at $8007.
        assert_eq!(probe.sites.keys().copied().collect::<Vec<_>>(), [0x00_8007]);
        assert_eq!(probe.modes.keys().copied().collect::<Vec<_>>(), [(true, false, false)]);
        // INC $10: one WRAM byte written, one new value per call.
        assert_eq!(probe.wram_writes.keys().copied().collect::<Vec<_>>(), [0x7E_0010]);
        let stat = &probe.wram_writes[&0x7E_0010];
        assert_eq!(stat.calls, probe.calls - probe.interrupted);
        // INC dp (5 CPU cycles) + RTS (6): the cost is a constant of the code.
        assert_eq!(probe.costs.len(), 1, "{:?}", probe.costs);
        assert_eq!(probe.samples.len(), 2);
        assert_eq!(probe.samples[0].writes.len(), 1);
        assert_eq!(probe.samples[0].regs_out.s, probe.samples[0].regs_in.s + 2);
        assert_eq!(probe.changed[0], 0, "INC dp leaves A alone");
    }

    #[test]
    fn the_report_names_calls_sites_costs_writes_and_the_listing() {
        let rom = leaf_rom();
        let replay = fixture_replay(&rom, 3);
        let mut probe = Probe::default();
        let subject = subject();
        probe_replay(&rom, &replay, "fixture", &subject, 1, &mut probe).unwrap();
        let image = RomImage(&rom);
        let options = Options::default();
        let (w, _) = disasm::entry_widths(&options, Some((Some(true), Some(false))), None);
        let listing = disasm::disassemble(&image, LEAF, w, &options, &|_| None);
        let namer = Namer { symbols: None };
        let text = report(&probe, &subject, &listing, &options, &image, &namer);
        assert!(text.starts_with("probe fixture_leaf at 008040 (rts):"), "{text}");
        assert!(text.contains("call sites (1 distinct"), "{text}");
        assert!(text.contains("008007 x"), "{text}");
        assert!(text.contains("wram 7E0010"), "{text}");
        assert!(text.contains("static direct WRAM reads (w = 16 bit): 7E0010"), "{text}");
        assert!(text.contains("sample 1:"), "{text}");
        // The leaf is 80 master cycles on a ROM with no NMI, HDMA or IRQ: every call fits.
        assert_eq!(probe.window_fit, probe.calls - probe.interrupted, "{probe:?}");
        assert!(text.contains("timing window (exact per-call bound):"), "{text}");
        assert!(text.contains("cost in CPU-busy master cycles"), "{text}");
    }

    #[test]
    fn a_routine_nobody_calls_says_so() {
        let rom = leaf_rom();
        let replay = fixture_replay(&rom, 2);
        let mut probe = Probe::default();
        let mut subject = subject();
        subject.address = 0x00_8100;
        probe_replay(&rom, &replay, "fixture", &subject, 1, &mut probe).unwrap();
        assert_eq!(probe.calls, 0);
        let image = RomImage(&rom);
        let options = Options::default();
        let listing =
            disasm::disassemble(&image, 0x8100, disasm::Widths::ASSUMED, &options, &|_| None);
        let text = report(&probe, &subject, &listing, &options, &image, &Namer { symbols: None });
        assert!(text.contains("no replay called it"), "{text}");
    }

    #[test]
    fn verify_names_a_routine_that_is_not_in_the_symbols_file() {
        let mut out = Vec::new();
        let dir = tempfile::tempdir().unwrap();
        let error = verify(&[], &Symbols::default(), "no_such_routine", dir.path(), true, &mut out)
            .unwrap_err();
        assert!(format!("{error:#}").contains("no_such_routine"), "{error:#}");
        assert!(out.is_empty(), "nothing is played before the routine is found");
    }

    #[test]
    fn a_symbol_resolves_to_its_address_and_the_flags_override_its_contract() {
        let symbols = Symbols::parse(
            "[[routine]]
name = \"leaf\"
address = 0x008040
returns = \"rts\"
timing = \"pure\"
             mode = \"original\"
entry_m8 = true
cycles = 40
",
            std::path::Path::new("symbols.toml"),
        )
        .unwrap();
        let by_name = parse_args(&["leaf", "--cycles", "50", "--m16"]).unwrap();
        let found = super::subject(&by_name, &symbols).unwrap();
        assert_eq!((found.name.as_str(), found.address), ("leaf", 0x00_8040));
        assert_eq!((found.entry_m8, found.cycles), (Some(false), Some(50)));
        let by_address = parse_args(&["--address", "808040", "--returns", "rts"]).unwrap();
        let found = super::subject(&by_address, &symbols).unwrap();
        assert_eq!(
            (found.name.as_str(), found.address, found.entry_m8),
            ("leaf", 0x00_8040, Some(true))
        );
        assert!(super::subject(&parse_args(&["nope"]).unwrap(), &symbols).is_err());
    }

    #[test]
    fn registers_and_writes_are_summarised() {
        let mut stat = RegStat::default();
        assert_eq!(stat.text(4), "-");
        stat.add(5);
        assert_eq!(stat.text(4), "0005");
        stat.add(0x20);
        assert_eq!(stat.text(4), "0005..0020 (0005 0020)");
        for v in 100..140 {
            stat.add(v);
        }
        assert!(stat.text(4).contains("more than"));
        assert_eq!(runs(&[1, 2, 3, 7, 9, 10]), [(1, 3), (7, 7), (9, 10)]);
        assert_eq!(flags_text(0x30), "nvMXdizc");
        let table = verify_table(&[(
            "battle".to_owned(),
            HookStats { calls: 3, verified_ok: 3, ..HookStats::default() },
        )]);
        assert!(table.contains("battle") && table.contains("verified"), "{table}");
    }
}
