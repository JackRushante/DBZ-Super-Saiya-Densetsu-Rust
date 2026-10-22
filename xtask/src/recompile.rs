//! `recompile`: turns the ROM's code into Rust (`ssd-recomp`), from the function entries and
//! M/X widths the trace runs of `.work/re/` observed. The output goes to `.work/recomp/` (never
//! tracked: it is derived from the ROM) where the `recomp` feature of `ssd-game` picks it up.

use crate::trace::Run;
use anyhow::Context;
use ssd_oracle::Symbols;
use ssd_recomp::Hints;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

/// The M/X bits of P (`0x20`, `0x10`) of a native-mode sample.
fn mx_bits(m8: bool, x8: bool) -> u8 {
    (u8::from(m8) << 5) | (u8::from(x8) << 4)
}

/// The generator's hints from the traces and the symbols.
#[must_use]
pub fn hints(runs: &[Run], symbols: &Symbols, routines_csv: Option<&str>) -> Hints {
    let mut h = Hints::default();
    for run in runs {
        for (&pc, row) in &run.exec {
            for &(m8, x8, e) in &row.modes {
                if !e {
                    h.widths_at.entry(pc).or_default().insert(mx_bits(m8, x8));
                }
            }
        }
        for edge in run.calls.iter().chain(&run.jumps) {
            h.entries.insert(edge.target);
            if matches!(edge.op.as_str(), "JMPI" | "JMPIX" | "JSRX") {
                h.ind_targets.entry(edge.source).or_default().insert(edge.target);
            }
        }
    }
    for routine in symbols.routines() {
        h.entries.insert(routine.address & 0x7F_FFFF);
    }
    for point in symbols.sync_points() {
        h.stops.insert(point.address & 0x7F_FFFF);
    }
    // The routines the cheat wrappers hook: generated code calls its callees directly, so it must hand
    // over in front of them, where the wrappers' hooks are.
    for address in ssd_game::cheats::entry_points() {
        h.stops.insert(address & 0x7F_FFFF);
    }
    // The same for the pack redirectors (`ssd_game::mods`).
    for address in ssd_game::mods::entry_points() {
        h.stops.insert(address & 0x7F_FFFF);
    }
    // Widths at the entries.
    let entries: Vec<u32> = h.entries.iter().chain(&h.stops).copied().collect();
    for entry in entries {
        if let Some(widths) = h.widths_at.get(&entry) {
            h.entry_mx.insert(entry, widths.clone());
        }
    }
    // Return kinds from the routine table.
    if let Some(text) = routines_csv {
        let mut lines = text.lines();
        if let Some(header) = lines.next() {
            let columns: Vec<&str> = header.split(',').collect();
            let col = |name: &str| columns.iter().position(|c| *c == name);
            if let (Some(entry), Some(returns)) = (col("entry"), col("returns")) {
                for line in lines {
                    let f: Vec<&str> = line.split(',').collect();
                    let (Some(e), Some(r)) = (f.get(entry), f.get(returns)) else { continue };
                    if let Ok(addr) = u32::from_str_radix(e, 16) {
                        match *r {
                            "rtl" => {
                                h.ret_long.insert(addr & 0x7F_FFFF, true);
                            }
                            "rts" => {
                                h.ret_long.insert(addr & 0x7F_FFFF, false);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    h
}

/// Runs the generator and writes `.work/recomp/generated.rs`; prints the report.
///
/// # Errors
///
/// The traces are missing or the output cannot be written.
pub fn run(
    rom: &[u8],
    symbols: &Symbols,
    runs: &[Run],
    root: &Path,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    if runs.is_empty() {
        writeln!(
            out,
            "recompile: no trace run in .work/re/: the entries come from the symbols file and the static call graph only"
        )?;
    }
    let csv = std::fs::read_to_string(root.join(".work").join("re").join("routines.csv")).ok();
    let mut hints = hints(runs, symbols, csv.as_deref());
    // Code the replays ran that the traces did not reach (`xtask causes --entries`).
    if let Ok(text) =
        std::fs::read_to_string(root.join(".work").join("re").join("extra-entries.txt"))
    {
        hints
            .extra_roots
            .extend(text.lines().filter_map(|l| u32::from_str_radix(l.trim(), 16).ok()));
    }
    // The native NMI and IRQ handlers: entered by the CPU with whatever widths it had.
    for vector in [0x7FEA_usize, 0x7FEE] {
        if let Some(bytes) = rom.get(vector..vector + 2) {
            let target = u32::from(u16::from_le_bytes([bytes[0], bytes[1]]));
            if target >= 0x8000 {
                hints.entries.insert(target);
                hints.entry_mx.insert(target, [0x00, 0x10, 0x20, 0x30].into_iter().collect());
            }
        }
    }
    let rom = rom.to_vec();
    let output = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || ssd_recomp::generate(&rom, &hints))
        .context("starting the generator thread")?
        .join()
        .map_err(|_| anyhow::anyhow!("the generator panicked"))?;
    let dir = root.join(".work").join("recomp");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("generated.rs");
    std::fs::write(&path, &output.source).with_context(|| format!("writing {}", path.display()))?;
    std::fs::write(dir.join("map.txt"), &output.map).context("writing map.txt")?;
    let r = &output.report;
    writeln!(out, "recompile: wrote {} ({} KiB)", path.display(), r.bytes / 1024)?;
    writeln!(
        out,
        "  {} function variants, {} routines registered, {} instructions",
        r.variants, r.routines, r.instructions
    )?;
    let rejected: BTreeSet<_> = r.rejected.iter().collect();
    writeln!(out, "  {} blocks the original can be met at", r.mid_entries)?;
    writeln!(out, "  {} entries rejected", rejected.len())?;
    for (address, reason) in rejected.iter().take(40) {
        writeln!(out, "    {address:06X}: {reason}")?;
    }
    writeln!(out, "  places that hand control back to the original, by reason:")?;
    for (reason, n) in &r.bails {
        writeln!(out, "    {n:6}  {reason}")?;
    }
    Ok(())
}
