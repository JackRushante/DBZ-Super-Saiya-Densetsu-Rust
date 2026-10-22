//! `verify-replays`: every replay of a directory (`*.ssdreplay`, default `.work/replays`) played
//! in lockstep (`ssd_oracle::run_lockstep`) on the canonical ROM of the cache: machine A pure,
//! machine B with every routine of `re/symbols.toml` in the mode asked (`rust` by default;
//! `original` hooks nothing on B, a check of the runner itself), both with the sync points of the
//! file. One line per replay (events, frames, unsynced windows, max cycle drift, then the counters
//! of the routines and the sync points); exit 0 when every replay is green, 1 when one diverged, 2
//! when none diverged but one had no sync event or the directory has no replay (unverified:
//! nothing was compared). The counters are judged too ([`judge`]): with `--hooks rust`, a cost over
//! the declared maximum or a missing cost is a divergence, a routine called but never run in Rust
//! leaves the replay unverified; a sync point reached inside a dispatch where it would have been an
//! event leaves it unverified in both modes. A missing directory or a replay that cannot be read is
//! an error (exit 1); a bad command line exits 64.

use crate::shared::{read_replay, render_table};
use anyhow::Context;
use ssd_oracle::{
    DEFAULT_BUDGET_FRAMES, HookMode, HookStats, LockstepConfig, LockstepReport, Symbols, Verdict,
    run_lockstep,
};
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// `--hooks`: what machine B runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hooks {
    /// Every routine of the symbols file in Rust (those kept on the original stay unhooked).
    Rust,
    /// No routine hooked: B runs as A.
    Original,
}

/// The command line of `verify-replays`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub hooks: Hooks,
    /// The replays' directory; `None`: `.work/replays` of the repository.
    pub dir: Option<PathBuf>,
    /// Machine B runs with the battle cheats installed, at a neutral block: the replays must still be
    /// identical to the original (`--cheats-identity`).
    pub cheats_identity: bool,
    /// Machine B runs on the zero-padded 2 MiB image of the empty pack set, with the redirectors
    /// installed: the replays must still be identical to the original (`--mods-identity`).
    pub mods_identity: bool,
}

/// Parses the arguments after `verify-replays`: `[--hooks rust|original] [--cheats-identity]
/// [--mods-identity] [dir]`.
///
/// # Errors
///
/// An unknown option or mode, a missing mode, or more than one directory.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut parsed =
        Args { hooks: Hooks::Rust, dir: None, cheats_identity: false, mods_identity: false };
    let mut args = args.iter();
    while let Some(&arg) = args.next() {
        match arg {
            "--hooks" => {
                parsed.hooks = match args.next().copied() {
                    Some("rust") => Hooks::Rust,
                    Some("original") => Hooks::Original,
                    Some(other) => return Err(format!("unknown --hooks mode `{other}`")),
                    None => return Err("--hooks needs a mode: rust or original".to_owned()),
                };
            }
            "--cheats-identity" => parsed.cheats_identity = true,
            "--mods-identity" => parsed.mods_identity = true,
            option if option.starts_with('-') => return Err(format!("unknown option `{option}`")),
            dir if parsed.dir.is_none() => parsed.dir = Some(PathBuf::from(dir)),
            dir => return Err(format!("one directory only (`{dir}` is a second one)")),
        }
    }
    Ok(parsed)
}

/// The lockstep setup for `hooks`: B's table built from `symbols` as the app builds it for its
/// `--hooks` (`ssd_game::hook_table`), and the sync points of `symbols`. A warning (on stderr)
/// names the Rust routines without `cycles`, which never run in Rust. With `cheats_identity`, B also
/// has the cheat wrappers; with `mods`, B runs on that image (the empty pack set's, see
/// `--mods-identity`) with the redirectors installed.
///
/// # Errors
///
/// A routine of `symbols` without a Rust function.
pub fn config(
    symbols: &Symbols,
    hooks: Hooks,
    cheats_identity: bool,
    mods: Option<&ssd_mod::Image>,
) -> anyhow::Result<LockstepConfig> {
    let mut table = match hooks {
        Hooks::Rust => {
            let table = ssd_game::hook_table(symbols, Some(HookMode::Rust))?;
            if let Some(warning) = ssd_game::missing_cycles_warning(&table) {
                eprintln!("verify-replays: warning: {warning}");
            }
            Some(table)
        }
        Hooks::Original => None,
    };
    if cheats_identity {
        let mut cheat_table = table.take().unwrap_or_default();
        ssd_game::cheats::install(&mut cheat_table);
        table = Some(cheat_table);
    }
    let b_packs = mods.map(|image| {
        let mut mod_table = table.take().unwrap_or_default();
        ssd_game::mods::install(&mut mod_table);
        table = Some(mod_table);
        (crate::shared::pack_tag(image), image.bytes.clone())
    });
    Ok(LockstepConfig {
        table,
        sync_keys: symbols.sync_points().iter().map(|point| point.address).collect(),
        budget_frames: DEFAULT_BUDGET_FRAMES,
        b_packs,
    })
}

/// The `*.ssdreplay` files of `dir`, by name.
///
/// # Errors
///
/// The directory cannot be read.
pub fn replay_paths(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.with_context(|| format!("reading {}", dir.display()))?.path();
        if path.extension().is_some_and(|extension| extension == "ssdreplay") && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// The exit code for these verdicts: 1 if one diverged, else 2 if one is unverified (or there is
/// none: nothing was verified), else 0.
#[must_use]
pub fn exit_code(verdicts: &[Verdict]) -> u8 {
    if verdicts.contains(&Verdict::Diverged) {
        1
    } else if verdicts.is_empty() || verdicts.contains(&Verdict::Unverified) {
        2
    } else {
        0
    }
}

/// A replay's verdict once the routines' counters are judged, with the reasons the counters
/// changed it (empty when they did not).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judgement {
    pub verdict: Verdict,
    pub reasons: Vec<String>,
}

/// The lockstep's verdict, judged against the routines' counters with `--hooks rust`: a routine
/// that ran over its declared maximum cost (`cost_over_bound`, so the timing window it was checked
/// against was too short) or without a declared cost (`cost_missing`) makes it `Diverged`; a
/// routine called but never run in Rust (every call fell back to the original) makes a green
/// replay `Unverified`, since lockstep then compared nothing of that routine. A routine never
/// called by the replay is not judged. With `--hooks original` the verdict is the lockstep's.
///
/// In both modes, a sync point that B reached inside a dispatch where it would have been an
/// event (`sync_inside_dispatch`) makes a green replay `Unverified`: lockstep did not compare
/// there.
#[must_use]
pub fn judge(report: &LockstepReport, hooks: Hooks) -> Judgement {
    let mut judgement = Judgement { verdict: report.verdict(), reasons: Vec::new() };
    let names = |stats: &[(String, HookStats)], pick: fn(&HookStats) -> bool| -> String {
        let picked: Vec<&str> =
            stats.iter().filter(|(_, s)| pick(s)).map(|(name, _)| name.as_str()).collect();
        picked.join(", ")
    };
    let unverified = |judgement: &mut Judgement, reason: String| {
        if judgement.verdict == Verdict::Green {
            judgement.verdict = Verdict::Unverified;
        }
        judgement.reasons.push(reason);
    };
    if hooks == Hooks::Rust {
        let costs = names(&report.hook_stats, |s| s.cost_over_bound > 0 || s.cost_missing > 0);
        if !costs.is_empty() {
            judgement.verdict = Verdict::Diverged;
            judgement.reasons.push(format!("cost over bound or missing: {costs}"));
        }
        // With recompiled routines (`rc_...`) in the table, calls made by generated code never
        // reach the hooks, and some recompiled routines can never run whole in Rust (a main
        // loop, a wait for the sound CPU): the rule applies to a table without them only.
        let recompiled = report.hook_stats.iter().any(|(name, _)| name.starts_with("rc_"));
        let never = if recompiled {
            String::new()
        } else {
            names(&report.hook_stats, |s| s.calls > 0 && s.rust_runs == 0)
        };
        if !never.is_empty() {
            unverified(&mut judgement, format!("called but never run in Rust: {never}"));
        }
    }
    let inside = names(&report.sync_stats, |s| s.sync_inside_dispatch > 0);
    if !inside.is_empty() {
        unverified(&mut judgement, format!("sync point reached inside a dispatch: {inside}"));
    }
    judgement
}

/// The line of a replay: `<name>: <verdict>: <events> events, <frames> frames, <unsynced>
/// unsynced windows, max drift <drift>`, then the widest dead stack excluded at an event, if any
/// (`, dead stack up to $0FC1-$0FFF`, bank-0 addresses). `verdict` is the judged one ([`judge`]).
#[must_use]
pub fn summary_line(name: &str, report: &LockstepReport, verdict: Verdict) -> String {
    let verdict = match verdict {
        Verdict::Green => "green",
        Verdict::Diverged => "DIVERGED",
        Verdict::Unverified if report.events == 0 => "UNVERIFIED (no sync event)",
        Verdict::Unverified => "UNVERIFIED",
    };
    let mut line = format!(
        "{name}: {verdict}: {} events, {} frames, {} unsynced windows, max drift {}",
        report.events, report.frames, report.unsynced_windows, report.max_cycle_drift
    );
    let dead = &report.widest_dead_stack;
    if !dead.is_empty() {
        let _ = write!(line, ", dead stack up to ${:04X}-${:04X}", dead.start, dead.end - 1);
    }
    line
}

const COLUMNS: [&str; 7] = [
    "entry",
    "calls",
    "rust_runs",
    "timing_fallback",
    "cost_mismatch",
    "cost_over_bound",
    "sync_inside_dispatch",
];

/// The counters on machine B of the routines, then of the sync points, one indented row each
/// under a header (nothing without either); a column that does not apply to the entry shows `-`.
#[must_use]
pub fn routine_table(
    routines: &[(String, HookStats)],
    sync_points: &[(String, HookStats)],
) -> String {
    // Recompiled routines (`rc_`, `rm_`) that were never called are not listed: there are
    // thousands of them.
    let listed = routines
        .iter()
        .filter(|(name, s)| s.calls > 0 || !(name.starts_with("rc_") || name.starts_with("rm_")));
    let routine_rows = listed.map(|(name, s)| {
        let counters = [s.rust_runs, s.timing_fallback, s.cost_mismatch, s.cost_over_bound];
        let mut row = vec![name.clone(), s.calls.to_string()];
        row.extend(counters.iter().map(u64::to_string));
        row.push("-".to_owned());
        row
    });
    let sync_rows = sync_points.iter().map(|(name, s)| {
        let mut row = vec![name.clone(), s.calls.to_string()];
        row.extend(std::iter::repeat_n("-".to_owned(), 4));
        row.push(s.sync_inside_dispatch.to_string());
        row
    });
    let rows: Vec<Vec<String>> = routine_rows.chain(sync_rows).collect();
    render_table(&COLUMNS, &rows)
}

/// Plays every replay of `dir` in lockstep on `rom` with `config`, printing each one's result to
/// `out` as it ends; returns the exit code ([`exit_code`]) of the verdicts judged for `hooks`.
///
/// # Errors
///
/// The directory or a replay cannot be read, a replay is not for `rom` or this emulator core
/// (record it again with `make-replays`), or a run fails.
pub fn verify(
    rom: &[u8],
    symbols: &Symbols,
    config: &LockstepConfig,
    hooks: Hooks,
    dir: &Path,
    out: &mut dyn Write,
) -> anyhow::Result<u8> {
    let paths = replay_paths(dir)?;
    if paths.is_empty() {
        writeln!(out, "verify-replays: no *.ssdreplay in {} (unverified)", dir.display())?;
        return Ok(exit_code(&[]));
    }
    let mut verdicts = Vec::new();
    for path in &paths {
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let replay = read_replay(path, rom)?;
        let start = Instant::now();
        let report = run_lockstep(rom.to_vec(), &replay, config, Some(symbols))
            .with_context(|| format!("playing {name}"))?;
        let seconds = start.elapsed().as_secs_f64();
        let judgement = judge(&report, hooks);
        writeln!(out, "{} ({seconds:.1} s)", summary_line(&name, &report, judgement.verdict))?;
        if let Some(divergence) = &report.first_divergence {
            for line in divergence.to_string().lines() {
                writeln!(out, "  {line}")?;
            }
        }
        for reason in &judgement.reasons {
            writeln!(out, "  {reason}")?;
        }
        write!(out, "{}", routine_table(&report.hook_stats, &report.sync_stats))?;
        out.flush()?;
        verdicts.push(judgement.verdict);
    }
    let count = |verdict| verdicts.iter().filter(|&&v| v == verdict).count();
    writeln!(
        out,
        "verify-replays: {} green, {} diverged, {} unverified",
        count(Verdict::Green),
        count(Verdict::Diverged),
        count(Verdict::Unverified)
    )?;
    Ok(exit_code(&verdicts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_oracle::LockstepDivergence;

    fn report(events: u64, divergence: Option<LockstepDivergence>) -> LockstepReport {
        LockstepReport {
            events,
            frames: 7200,
            unsynced_windows: 4,
            max_cycle_drift: 0,
            first_divergence: divergence,
            hook_stats: Vec::new(),
            sync_stats: Vec::new(),
            widest_dead_stack: 0..0,
        }
    }

    #[test]
    fn the_arguments_default_to_rust_and_the_work_directory() {
        assert_eq!(
            parse_args(&[]),
            Ok(Args {
                hooks: Hooks::Rust,
                dir: None,
                cheats_identity: false,
                mods_identity: false
            })
        );
        assert_eq!(
            parse_args(&["--hooks", "original", "some/dir"]),
            Ok(Args {
                hooks: Hooks::Original,
                dir: Some(PathBuf::from("some/dir")),
                cheats_identity: false,
                mods_identity: false,
            })
        );
        assert_eq!(
            parse_args(&["other", "--hooks", "rust"]),
            Ok(Args {
                hooks: Hooks::Rust,
                dir: Some(PathBuf::from("other")),
                cheats_identity: false,
                mods_identity: false,
            })
        );
        assert_eq!(
            parse_args(&["--cheats-identity", "--mods-identity"]),
            Ok(Args { hooks: Hooks::Rust, dir: None, cheats_identity: true, mods_identity: true }),
            "the two identities combine"
        );
        for bad in [&["--hooks"][..], &["--hooks", "verify"], &["--fast"], &["a", "b"]] {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_divergence_wins_over_an_unverified_replay_and_both_over_green() {
        use Verdict::{Diverged, Green, Unverified};
        assert_eq!(exit_code(&[Green, Green]), 0);
        assert_eq!(exit_code(&[Green, Unverified]), 2);
        assert_eq!(exit_code(&[Unverified, Diverged, Green]), 1);
        assert_eq!(exit_code(&[]), 2, "no replay verifies nothing");
    }

    #[test]
    fn a_replay_without_events_is_unverified_never_green() {
        let unverified = report(0, None);
        assert_eq!(unverified.verdict(), Verdict::Unverified);
        assert_eq!(exit_code(&[unverified.verdict()]), 2);
        assert!(
            summary_line("intro.ssdreplay", &unverified, Verdict::Unverified)
                .contains("UNVERIFIED (no sync event)")
        );
        let green = summary_line("battle.ssdreplay", &report(3880, None), Verdict::Green);
        assert_eq!(
            green,
            "battle.ssdreplay: green: 3880 events, 7200 frames, 4 unsynced windows, max drift 0"
        );
        let mut with_dead_stack = report(3880, None);
        with_dead_stack.widest_dead_stack = 0x0FC1..0x1000;
        assert!(
            summary_line("b", &with_dead_stack, Verdict::Green)
                .ends_with("max drift 0, dead stack up to $0FC1-$0FFF")
        );
        let diverged = report(12, Some(LockstepDivergence::CycleDrift { event: 12, drift: 6 }));
        assert!(
            summary_line("x", &diverged, Verdict::Diverged).starts_with("x: DIVERGED: 12 events")
        );
    }

    fn with_stats(stats: &[(&str, HookStats)]) -> LockstepReport {
        let mut green = report(3880, None);
        green.hook_stats = stats.iter().map(|(name, s)| ((*name).to_owned(), *s)).collect();
        green
    }

    #[test]
    fn rust_routines_that_never_ran_or_overran_their_cost_are_not_green() {
        let ran = HookStats { calls: 10, rust_runs: 9, timing_fallback: 1, ..HookStats::default() };
        let unused = HookStats::default();
        let fell_back = HookStats { calls: 4, timing_fallback: 4, ..HookStats::default() };
        let over = HookStats { cost_over_bound: 1, ..ran };
        let missing = HookStats { cost_missing: 4, ..fell_back };

        let fine = judge(&with_stats(&[("a", ran), ("b", unused)]), Hooks::Rust);
        assert_eq!(fine, Judgement { verdict: Verdict::Green, reasons: Vec::new() });

        let never = judge(&with_stats(&[("a", ran), ("b", fell_back)]), Hooks::Rust);
        assert_eq!(never.verdict, Verdict::Unverified);
        assert_eq!(never.reasons, ["called but never run in Rust: b"]);
        assert_eq!(
            summary_line("r", &with_stats(&[]), never.verdict),
            "r: UNVERIFIED: 3880 events, 7200 frames, 4 unsynced windows, max drift 0"
        );

        let costs = judge(&with_stats(&[("a", over), ("b", missing)]), Hooks::Rust);
        assert_eq!(costs.verdict, Verdict::Diverged);
        assert_eq!(
            costs.reasons,
            ["cost over bound or missing: a, b", "called but never run in Rust: b"]
        );
        assert_eq!(exit_code(&[costs.verdict]), 1);

        let mut diverged = with_stats(&[("b", fell_back)]);
        diverged.first_divergence = Some(LockstepDivergence::CycleDrift { event: 3, drift: 2 });
        assert_eq!(judge(&diverged, Hooks::Rust).verdict, Verdict::Diverged);

        let original = judge(&with_stats(&[("b", fell_back), ("a", over)]), Hooks::Original);
        assert_eq!(original, Judgement { verdict: Verdict::Green, reasons: Vec::new() });
    }

    #[test]
    fn a_sync_point_reached_inside_a_dispatch_leaves_the_replay_unverified() {
        let ran = HookStats { calls: 10, rust_runs: 10, ..HookStats::default() };
        let quiet = HookStats { calls: 5000, ..HookStats::default() };
        let inside = HookStats { sync_inside_dispatch: 2, ..quiet };
        let with_sync = |sync: &[(&str, HookStats)]| {
            let mut report = with_stats(&[("a", ran)]);
            report.sync_stats = sync.iter().map(|(name, s)| ((*name).to_owned(), *s)).collect();
            report
        };

        let fine = judge(&with_sync(&[("idle", quiet)]), Hooks::Rust);
        assert_eq!(fine, Judgement { verdict: Verdict::Green, reasons: Vec::new() });

        for hooks in [Hooks::Rust, Hooks::Original] {
            let judged = judge(&with_sync(&[("idle", inside), ("menu", quiet)]), hooks);
            assert_eq!(judged.verdict, Verdict::Unverified, "{hooks:?}");
            assert_eq!(judged.reasons, ["sync point reached inside a dispatch: idle"]);
        }

        let mut diverged = with_sync(&[("idle", inside)]);
        diverged.first_divergence = Some(LockstepDivergence::CycleDrift { event: 3, drift: 2 });
        assert_eq!(judge(&diverged, Hooks::Rust).verdict, Verdict::Diverged);
    }

    #[test]
    fn the_routine_table_has_a_header_and_a_row_per_routine_and_sync_point() {
        assert_eq!(routine_table(&[], &[]), "");
        let stats = HookStats {
            calls: 10,
            rust_runs: 7,
            timing_fallback: 3,
            cost_mismatch: 1,
            cost_over_bound: 2,
            ..HookStats::default()
        };
        let idle = HookStats { calls: 4000, sync_inside_dispatch: 1, ..HookStats::default() };
        let table =
            routine_table(&[("rng_next_a".to_owned(), stats)], &[("idle".to_owned(), idle)]);
        let rows: Vec<Vec<&str>> =
            table.lines().map(|line| line.split_whitespace().collect()).collect();
        assert_eq!(
            rows,
            [
                COLUMNS.to_vec(),
                vec!["rng_next_a", "10", "7", "3", "1", "2", "-"],
                vec!["idle", "4000", "-", "-", "-", "-", "1"],
            ]
        );
        assert!(table.lines().all(|line| line.starts_with("  ")), "{table}");
    }

    #[test]
    fn only_replay_files_are_played_in_name_order() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["b.ssdreplay", "a.ssdreplay", "notes.txt", "c.ssdreplay.tmp"] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        std::fs::create_dir(dir.path().join("d.ssdreplay")).unwrap();
        let names: Vec<String> = replay_paths(dir.path())
            .unwrap()
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.ssdreplay", "b.ssdreplay"]);
        assert!(replay_paths(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn an_empty_directory_verifies_nothing_and_a_missing_one_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (symbols, config) = (Symbols::default(), LockstepConfig::default());
        let mut out = Vec::new();
        let code = verify(&[], &symbols, &config, Hooks::Rust, dir.path(), &mut out).unwrap();
        assert_eq!(code, 2);
        assert!(String::from_utf8(out).unwrap().contains("no *.ssdreplay"));

        let missing = dir.path().join("missing");
        let error =
            verify(&[], &symbols, &config, Hooks::Rust, &missing, &mut Vec::new()).unwrap_err();
        assert!(format!("{error:#}").contains("missing"), "{error:#}");
    }

    #[test]
    fn the_rust_config_hooks_every_routine_of_the_repository_and_its_sync_points() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../re/symbols.toml");
        let symbols = Symbols::load(path).unwrap();
        let rust = config(&symbols, Hooks::Rust, false, None).unwrap();
        let table = rust.table.unwrap();
        assert_eq!(table.len(), symbols.routines().len());
        assert!(table.entries().all(|entry| entry.mode == HookMode::Rust));
        let sync: Vec<u32> = symbols.sync_points().iter().map(|point| point.address).collect();
        assert_ne!(sync, Vec::<u32>::new());
        assert_eq!(rust.sync_keys, sync);
        assert_eq!(rust.budget_frames, DEFAULT_BUDGET_FRAMES);
        let original = config(&symbols, Hooks::Original, false, None).unwrap();
        assert!(original.table.is_none());
        assert_eq!(original.sync_keys, sync);
        assert!(original.b_packs.is_none(), "without --mods-identity B runs on the ROM");
    }

    #[test]
    fn the_mods_identity_puts_the_redirectors_on_b_and_its_image_there() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../re/symbols.toml");
        let symbols = Symbols::load(path).unwrap();
        let rom = vec![0_u8; ssd_mod::ROM_LEN];
        let image = ssd_mod::build(&rom, &[]).unwrap();
        let config = config(&symbols, Hooks::Rust, false, Some(&image)).unwrap();
        let table = config.table.unwrap();
        assert!(ssd_game::mods::installed_in(Some(&table)), "the redirectors are on B");
        let (tag, bytes) = config.b_packs.unwrap();
        assert_eq!(tag, crate::shared::pack_tag(&image));
        assert_eq!(bytes.len(), 0x20_0000, "the zero-padded 2 MiB image");
    }
}
