//! The hook modes of the command line: the symbols a run reads, the hook table built from them,
//! and the report a verify run prints at exit.

use anyhow::Context;
use ssd_oracle::{
    DEFAULT_BUDGET_FRAMES, HookEntry, HookMode, HookStats, HookTable, LockstepConfig, Machine,
    Symbols,
};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Where verify runs keep divergence reports unless `--divergence-dir` says otherwise.
pub const DEFAULT_DIVERGENCE_DIR: &str = ".work/divergences";

/// `--hooks`: which routines the Rust code takes over.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum HooksArg {
    /// No hook table: the original game code runs, as before M1.
    #[default]
    Original,
    /// The Rust routines replace the original code.
    Rust,
    /// The original code runs and every Rust routine is checked against it.
    Verify,
}

/// Everything a hooked run installs on its machine.
pub struct HookSetup {
    table: HookTable,
    symbols: Arc<Symbols>,
    verify: bool,
    divergence_dir: Option<PathBuf>,
}

impl HookSetup {
    /// Installs the hook table, the symbols that name variables in divergence reports, and the
    /// report directory.
    pub fn install(&self, machine: &mut Machine) {
        machine.set_symbols(Some(Arc::clone(&self.symbols)));
        machine.set_divergence_dir(self.divergence_dir.clone());
        machine.set_hook_table(Some(self.table.clone()));
    }

    /// Whether the routines are checked against the original (the run ends with a report).
    pub fn is_verify(&self) -> bool {
        self.verify
    }

    pub fn divergence_dir(&self) -> Option<&Path> {
        self.divergence_dir.as_deref()
    }
}

/// Loads the symbols and builds the hook table for `mode`; `None` when nothing is hooked.
///
/// With `--hooks original` the symbols are not needed, but a file given with `--symbols` is still
/// read, so a typo does not go unnoticed. In `rust` mode a warning lists the routines without a
/// `cycles` cost. A `--divergence-dir` outside `--hooks verify` is ignored with a warning: only
/// `verify` compares anything.
///
/// # Errors
///
/// A symbols file that cannot be read or is not valid, or a routine of it that `ssd-game` has no
/// Rust function for. The message names the file.
pub fn prepare(
    mode: HooksArg,
    symbols_arg: Option<&Path>,
    divergence_dir_arg: Option<&Path>,
) -> anyhow::Result<Option<HookSetup>> {
    if let Some(warning) = divergence_dir_warning(mode, divergence_dir_arg) {
        log::warn!("{warning}");
    }
    let hook_mode = match mode {
        HooksArg::Original if symbols_arg.is_none() => return Ok(None),
        HooksArg::Original => None,
        HooksArg::Rust => Some(HookMode::Rust),
        HooksArg::Verify => Some(HookMode::Verify),
    };
    let path = symbols_path(symbols_arg);
    let symbols = Symbols::load(&path)?;
    let Some(hook_mode) = hook_mode else { return Ok(None) };
    let table = ssd_game::hook_table(&symbols, Some(hook_mode))
        .with_context(|| format!("{}", path.display()))?;
    if let Some(warning) = ssd_game::missing_cycles_warning(&table) {
        log::warn!("{}: {warning}", path.display());
    }
    let verify = hook_mode == HookMode::Verify;
    let divergence_dir = verify.then(|| {
        divergence_dir_arg.map_or_else(|| PathBuf::from(DEFAULT_DIVERGENCE_DIR), Path::to_path_buf)
    });
    Ok(Some(HookSetup { table, symbols: Arc::new(symbols), verify, divergence_dir }))
}

/// The symbols file: `--symbols`, else the default one ([`default_symbols_path`]).
fn symbols_path(symbols_arg: Option<&Path>) -> PathBuf {
    symbols_arg.map_or_else(
        || {
            let exe = std::env::current_exe().ok();
            default_symbols_path(
                std::env::current_dir().ok().as_deref(),
                exe.as_deref().and_then(Path::parent),
            )
        },
        Path::to_path_buf,
    )
}

impl HooksArg {
    /// The name the panel shows.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Rust => "Rust",
            Self::Verify => "Verify",
        }
    }

    /// The modes the panel offers, in its order.
    pub const ALL: [HooksArg; 3] = [Self::Rust, Self::Verify, Self::Original];

    /// The global mode of a routine in `mode`: what the panel shows when every routine has it.
    #[must_use]
    pub fn of(mode: HookMode) -> Self {
        match mode {
            HookMode::Original => Self::Original,
            // A patch is Rust code that takes no time (the redirectors of packs, the story fixes).
            HookMode::Rust | HookMode::Patch => Self::Rust,
            HookMode::Verify => Self::Verify,
        }
    }
}

/// What the debug panel needs to change the hook mode of a running machine: the symbols of the
/// run, whatever mode it started in, and where `Verify` saves divergence reports.
pub struct HookSwitch {
    symbols: Arc<Symbols>,
    divergence_dir: PathBuf,
}

impl HookSwitch {
    /// Loads the symbols file (`--symbols`, else the default one).
    ///
    /// # Errors
    ///
    /// The file cannot be read or is not valid; the message names it.
    pub fn load(
        symbols_arg: Option<&Path>,
        divergence_dir_arg: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let symbols = Symbols::load(symbols_path(symbols_arg))?;
        let divergence_dir = divergence_dir_arg
            .map_or_else(|| PathBuf::from(DEFAULT_DIVERGENCE_DIR), Path::to_path_buf);
        Ok(Self { symbols: Arc::new(symbols), divergence_dir })
    }

    /// The hook table of `mode` (`None` for the original code).
    ///
    /// # Errors
    ///
    /// A routine of the symbols that `ssd-game` has no Rust function for.
    pub fn table(&self, mode: HooksArg) -> anyhow::Result<Option<HookTable>> {
        let hook_mode = match mode {
            HooksArg::Original => return Ok(None),
            HooksArg::Rust => HookMode::Rust,
            HooksArg::Verify => HookMode::Verify,
        };
        Ok(Some(ssd_game::hook_table(&self.symbols, Some(hook_mode))?))
    }

    /// The symbols of the run.
    #[must_use]
    pub fn symbols(&self) -> &Arc<Symbols> {
        &self.symbols
    }

    /// Where divergence reports go (from `--divergence-dir`, else the default).
    #[must_use]
    pub fn divergence_dir(&self) -> &Path {
        &self.divergence_dir
    }

    /// Every routine of the symbols with its Rust function, in the mode the symbols give it: the
    /// table a divergence report is looked up in (see `ssd_oracle::repro::reproduce`).
    ///
    /// # Errors
    ///
    /// As for [`HookSwitch::table`].
    pub fn resolver_table(&self) -> anyhow::Result<HookTable> {
        Ok(ssd_game::hook_table(&self.symbols, None)?)
    }

    /// The lockstep configuration `verify-replays` uses in Rust mode: the routines in Rust, and the
    /// symbols' sync points as the sync keys.
    ///
    /// # Errors
    ///
    /// As for [`HookSwitch::table`].
    pub fn lockstep_config(&self) -> anyhow::Result<LockstepConfig> {
        let table = ssd_game::hook_table(&self.symbols, Some(HookMode::Rust))?;
        if let Some(warning) = ssd_game::missing_cycles_warning(&table) {
            log::warn!("{warning}");
        }
        Ok(LockstepConfig {
            table: Some(table),
            sync_keys: self.symbols.sync_points().iter().map(|point| point.address).collect(),
            budget_frames: DEFAULT_BUDGET_FRAMES,
            b_packs: None,
        })
    }

    /// Installs the table of `mode` on the machine, live: the dispatch counters of the routines
    /// that stay hooked are kept ([`Machine::set_hook_table`]). The symbols are always set, and
    /// the divergence directory only for `Verify`, the one mode that compares.
    ///
    /// # Errors
    ///
    /// As for [`HookSwitch::table`]; the machine is left as it was.
    pub fn apply(&self, machine: &mut Machine, mode: HooksArg) -> anyhow::Result<()> {
        let table = self.table(mode)?;
        machine.set_symbols(Some(Arc::clone(&self.symbols)));
        machine.set_divergence_dir((mode == HooksArg::Verify).then(|| self.divergence_dir.clone()));
        machine.set_hook_table(table);
        Ok(())
    }

    /// Installs, live, a table in which every routine has the mode `modes` gives it by name; a
    /// routine it does not name is original. The dispatch counters are kept as in
    /// [`HookSwitch::apply`]. The divergence directory is set when a routine is `Verify`, and the
    /// machine has no table when none is hooked (as in the original mode).
    ///
    /// # Errors
    ///
    /// A routine of the symbols that `ssd-game` has no Rust function for; the machine is left as
    /// it was.
    pub fn apply_modes(
        &self,
        machine: &mut Machine,
        modes: &[(String, HookMode)],
    ) -> anyhow::Result<()> {
        let table = ssd_game::hook_table(&self.symbols, None)?
            .entries()
            .map(|entry| HookEntry {
                mode: modes
                    .iter()
                    .find(|(name, _)| *name == entry.name)
                    .map_or(HookMode::Original, |&(_, mode)| mode),
                ..entry.clone()
            })
            .collect::<HookTable>();
        let hooked = table.entries().any(|entry| entry.mode != HookMode::Original);
        let verify = table.entries().any(|entry| entry.mode == HookMode::Verify);
        machine.set_symbols(Some(Arc::clone(&self.symbols)));
        machine.set_divergence_dir(verify.then(|| self.divergence_dir.clone()));
        machine.set_hook_table(hooked.then_some(table));
        Ok(())
    }
}

/// What to warn about a `--divergence-dir` given with a mode that saves no reports, if anything.
fn divergence_dir_warning(mode: HooksArg, divergence_dir: Option<&Path>) -> Option<String> {
    let dir = divergence_dir.filter(|_| mode != HooksArg::Verify)?;
    let mode = match mode {
        HooksArg::Original => "original",
        HooksArg::Rust => "rust",
        HooksArg::Verify => unreachable!("filtered above"),
    };
    Some(format!(
        "--divergence-dir {} is ignored with --hooks {mode}: only --hooks verify compares the \
         routines with the original and saves divergence reports",
        dir.display()
    ))
}

/// `re/symbols.toml` of the workspace containing `start` (or a parent of it): the nearest
/// directory with a `Cargo.toml` declaring `[workspace]` and an `re/symbols.toml` next to it.
pub fn find_workspace_symbols(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|dir| {
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
        let declares_workspace =
            manifest.lines().any(|line| line.trim_start().starts_with("[workspace"));
        let symbols = dir.join("re").join("symbols.toml");
        (declares_workspace && symbols.is_file()).then_some(symbols)
    })
}

/// The symbols file when `--symbols` is not given: the workspace's found from `start` (the
/// current directory), else the workspace's found from `exe_dir` (an executable built in
/// `target/` run from elsewhere), else `symbols.toml` in `exe_dir`.
pub fn default_symbols_path(start: Option<&Path>, exe_dir: Option<&Path>) -> PathBuf {
    [start, exe_dir]
        .into_iter()
        .flatten()
        .find_map(find_workspace_symbols)
        .unwrap_or_else(|| exe_dir.unwrap_or(Path::new("")).join("symbols.toml"))
}

/// The widest line of the stats tables.
const MAX_WIDTH: usize = 120;

/// The columns of the first table, after the routine: what the checks found.
const CHECK_HEADERS: [&str; 6] =
    ["calls", "verified_ok", "divergences", "interrupted", "hdma_crossed", "sampled_out"];

/// The columns of the second table, after the routine: the calls that could not be checked, and
/// the mean cost of the original.
const OTHER_HEADERS: [&str; 5] =
    ["contract_mismatch", "stack_switched", "no_return", "mean_cpu_cycles", "mean_instructions"];

fn check_cells(s: &HookStats) -> Vec<String> {
    [s.calls, s.verified_ok, s.divergences, s.interrupted, s.hdma_crossed, s.sampled_out]
        .map(|count| count.to_string())
        .to_vec()
}

fn other_cells(s: &HookStats) -> Vec<String> {
    let counts =
        [s.contract_mismatch, s.stack_switched, s.no_return].map(|count| count.to_string());
    let means = [s.mean_cpu_cycles, s.mean_instructions].map(|mean| format!("{mean:.1}"));
    counts.into_iter().chain(means).collect()
}

/// The counters the tables have no column for, with their names, those that are not zero: shown
/// on lines under their routine's row of the second table.
fn extra_counters(s: &HookStats) -> Vec<(&'static str, u64)> {
    [
        ("timing_fallback", s.timing_fallback),
        ("cost_mismatch", s.cost_mismatch),
        ("instructions_mismatch", s.instructions_mismatch),
        ("cost_missing", s.cost_missing),
        ("cost_over_bound", s.cost_over_bound),
        ("nested", s.nested),
        ("nested_overflow", s.nested_overflow),
        ("reentered", s.reentered),
    ]
    .into_iter()
    .filter(|&(_, count)| count > 0)
    .collect()
}

/// The extra counters of `s` as `name=count`, indented, on as few lines as [`MAX_WIDTH`] allows.
fn extra_lines(s: &HookStats) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for (name, count) in extra_counters(s) {
        let pair = format!("{name}={count}");
        match lines.last_mut() {
            Some(line) if line.len() + 1 + pair.len() <= MAX_WIDTH => {
                line.push(' ');
                line.push_str(&pair);
            }
            _ => lines.push(format!("    {pair}")),
        }
    }
    lines
}

/// A table as lines: the header, then a line for each row of `rows`. The first column (the
/// routine) is to the left, the others to the right, each as wide as its widest cell.
fn render(headers: &[&str], rows: &[Vec<String>]) -> Vec<String> {
    let widths: Vec<usize> = (0..headers.len())
        .map(|column| {
            rows.iter().map(|row| row[column].len()).fold(headers[column].len(), usize::max)
        })
        .collect();
    let line = |cells: &[&str]| {
        let mut out = String::new();
        for (column, cell) in cells.iter().enumerate() {
            let width = widths[column];
            let _ = match column {
                0 => write!(out, "{cell:<width$}"),
                _ => write!(out, "  {cell:>width$}"),
            };
        }
        out
    };
    let mut lines = vec![line(headers)];
    lines.extend(rows.iter().map(|row| line(&row.iter().map(String::as_str).collect::<Vec<_>>())));
    lines
}

/// Two tables with one row per routine, under each other with an empty line between, none wider
/// than [`MAX_WIDTH`] columns for routine names up to 30 characters: the dispatch counters of a
/// verify run (what the checks found), then the calls that could not be checked and the mean
/// cost of the original. Under each routine's row of the second, indented, the counters without
/// a column (`timing_fallback`, `cost_*`, `nested*`, `reentered`) that are not zero as
/// `name=count`.
pub fn stats_table(stats: &[(String, HookStats)]) -> String {
    let table = |headers: &[&str], cells: fn(&HookStats) -> Vec<String>| {
        let rows: Vec<Vec<String>> = stats
            .iter()
            .map(|(name, s)| std::iter::once(name.clone()).chain(cells(s)).collect())
            .collect();
        let headers: Vec<&str> =
            std::iter::once("routine").chain(headers.iter().copied()).collect();
        render(&headers, &rows)
    };
    let mut lines = table(&CHECK_HEADERS, check_cells);
    lines.push(String::new());
    let others = table(&OTHER_HEADERS, other_cells);
    lines.push(others[0].clone());
    for (row, (_, s)) in others[1..].iter().zip(stats) {
        lines.push(row.clone());
        lines.extend(extra_lines(s));
    }
    lines.join("\n")
}

/// Verify calls whose Rust result differed from the original's, over all routines.
pub fn divergence_count(stats: &[(String, HookStats)]) -> u64 {
    stats.iter().map(|(_, s)| s.divergences).sum()
}

/// Divergence reports that could not be saved, over all routines.
pub fn report_error_count(stats: &[(String, HookStats)]) -> u64 {
    stats.iter().map(|(_, s)| s.report_errors).sum()
}

/// The line a verify run with divergences ends with: where the reports are, or that some could
/// not be saved (the log has the reason).
pub fn divergence_summary(divergences: u64, report_errors: u64, dir: Option<&Path>) -> String {
    let mut summary = format!("verify: {divergences} divergence(s)");
    if report_errors > 0 {
        let _ = write!(summary, "; {report_errors} report(s) could not be saved, see the log");
    } else if let Some(dir) = dir {
        let _ = write!(summary, "; reports in {}", dir.display());
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_oracle::{HookMode, Symbols};

    const REPO_SYMBOLS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml");

    fn routine(name: &str, address: &str, mode: &str, extra: &str) -> String {
        format!(
            "[[routine]]\nname = \"{name}\"\naddress = {address}\nreturns = \"rtl\"\n\
             timing = \"pure\"\nmode = \"{mode}\"\n{extra}\n"
        )
    }

    fn stats(calls: u64, verified_ok: u64, divergences: u64) -> HookStats {
        HookStats { calls, verified_ok, divergences, ..HookStats::default() }
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_workspace_symbols_are_found_from_any_directory_below_the_root() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("Cargo.toml"), "[workspace]\nmembers = []\n");
        write(&dir.path().join("re/symbols.toml"), "");
        let deep = dir.path().join("crates/app/src");
        std::fs::create_dir_all(&deep).unwrap();
        // A package manifest on the way up is not the workspace root.
        write(&dir.path().join("crates/app/Cargo.toml"), "[package]\nname = \"app\"\n");
        let expected = dir.path().join("re").join("symbols.toml");
        assert_eq!(find_workspace_symbols(&deep), Some(expected.clone()));
        assert_eq!(find_workspace_symbols(dir.path()), Some(expected));
    }

    #[test]
    fn a_symbols_file_next_to_a_package_manifest_is_not_a_workspace_file() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("Cargo.toml"), "[package]\nname = \"alone\"\n");
        write(&dir.path().join("re/symbols.toml"), "");
        assert_eq!(find_workspace_symbols(dir.path()), None);
        // A workspace manifest without the file does not count either.
        let other = tempfile::tempdir().unwrap();
        write(&other.path().join("Cargo.toml"), "[workspace]\n");
        assert_eq!(find_workspace_symbols(other.path()), None);
    }

    /// A directory that is a workspace root with an `re/symbols.toml`.
    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("Cargo.toml"),
            "[workspace]
",
        );
        write(&dir.path().join("re/symbols.toml"), "");
        dir
    }

    #[test]
    fn the_default_symbols_path_prefers_the_workspace_of_the_current_directory() {
        let (cwd_workspace, exe_workspace) = (workspace(), workspace());
        let exe_dir = exe_workspace.path().join("target").join("release");
        assert_eq!(
            default_symbols_path(Some(&cwd_workspace.path().join("crates")), Some(&exe_dir)),
            cwd_workspace.path().join("re").join("symbols.toml")
        );
    }

    #[test]
    fn the_default_symbols_path_walks_up_from_the_executable_when_the_current_directory_has_none() {
        let (elsewhere, install) = (tempfile::tempdir().unwrap(), workspace());
        let exe_dir = install.path().join("target").join("release");
        let expected = install.path().join("re").join("symbols.toml");
        assert_eq!(default_symbols_path(Some(elsewhere.path()), Some(&exe_dir)), expected);
        assert_eq!(default_symbols_path(None, Some(&exe_dir)), expected);
    }

    #[test]
    fn the_default_symbols_path_falls_back_to_the_executable_directory() {
        let (elsewhere, install) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        // A package manifest on the way up is not a workspace root either.
        write(
            &install.path().join("Cargo.toml"),
            "[package]
name = \"alone\"
",
        );
        write(&install.path().join("re/symbols.toml"), "");
        let exe_dir = install.path().join("bin");
        let expected = exe_dir.join("symbols.toml");
        assert_eq!(default_symbols_path(Some(elsewhere.path()), Some(&exe_dir)), expected);
        assert_eq!(default_symbols_path(None, Some(&exe_dir)), expected);
        assert_eq!(default_symbols_path(None, None), Path::new("symbols.toml"));
    }

    #[test]
    fn original_without_a_symbols_file_installs_nothing_and_reads_nothing() {
        // The default symbols path does not have to exist in this mode.
        assert!(prepare(HooksArg::Original, None, None).unwrap().is_none());
    }

    #[test]
    fn rust_and_verify_hook_every_routine_of_the_repository_symbols() {
        let symbols = Symbols::load(REPO_SYMBOLS).unwrap();
        let routines = symbols.routines().len();
        assert!(routines > 0);

        let setup = prepare(HooksArg::Rust, Some(Path::new(REPO_SYMBOLS)), None).unwrap().unwrap();
        assert!(!setup.is_verify());
        assert_eq!(setup.table.len(), routines);
        assert!(setup.table.entries().all(|entry| entry.mode == HookMode::Rust));

        let setup =
            prepare(HooksArg::Verify, Some(Path::new(REPO_SYMBOLS)), None).unwrap().unwrap();
        assert!(setup.is_verify());
        assert_eq!(setup.table.len(), routines);
        assert!(setup.table.entries().all(|entry| entry.mode == HookMode::Verify));
    }

    #[test]
    fn verify_reports_divergences_under_work_unless_told_otherwise() {
        let symbols = Some(Path::new(REPO_SYMBOLS));
        let setup = prepare(HooksArg::Verify, symbols, None).unwrap().unwrap();
        assert_eq!(setup.divergence_dir(), Some(Path::new(DEFAULT_DIVERGENCE_DIR)));
        let custom = Path::new("elsewhere");
        let setup = prepare(HooksArg::Verify, symbols, Some(custom)).unwrap().unwrap();
        assert_eq!(setup.divergence_dir(), Some(custom));
        // Rust runs compare nothing, so they keep no reports.
        let setup = prepare(HooksArg::Rust, symbols, None).unwrap().unwrap();
        assert_eq!(setup.divergence_dir(), None);
    }

    #[test]
    fn original_with_an_explicit_symbols_file_checks_it_and_installs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            prepare(HooksArg::Original, Some(Path::new(REPO_SYMBOLS)), None).unwrap().is_none()
        );
        let bad = dir.path().join("bad.toml");
        write(&bad, "not = [valid");
        let error = prepare(HooksArg::Original, Some(&bad), None).err().unwrap();
        assert!(format!("{error:#}").contains("bad.toml"), "{error:#}");
    }

    #[test]
    fn symbols_errors_name_the_file_and_the_routine() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.toml");
        let error = prepare(HooksArg::Rust, Some(&missing), None).err().unwrap();
        assert!(format!("{error:#}").contains("missing.toml"), "{error:#}");

        let unknown = dir.path().join("unknown.toml");
        write(&unknown, &routine("no_such_routine", "0x018000", "verify", ""));
        let error = prepare(HooksArg::Verify, Some(&unknown), None).err().unwrap();
        let message = format!("{error:#}");
        assert!(
            message.contains("unknown.toml") && message.contains("no_such_routine"),
            "{message}"
        );
    }

    #[test]
    fn install_puts_the_table_on_the_machine() {
        let setup = prepare(HooksArg::Rust, Some(Path::new(REPO_SYMBOLS)), None).unwrap().unwrap();
        let (mut machine, _store) = crate::test_support::sram_counter_machine();
        assert_eq!(machine.hook_stats(), []);
        setup.install(&mut machine);
        assert_eq!(machine.hook_stats().len(), setup.table.len());
    }

    #[test]
    fn the_panel_switches_the_hook_mode_of_a_running_machine() {
        let switch = HookSwitch::load(Some(Path::new(REPO_SYMBOLS)), None).unwrap();
        let routines = switch.symbols.routines().len();
        let (mut machine, _store) = crate::test_support::sram_counter_machine();
        assert!(machine.hook_table().is_none());
        for (mode, hook_mode) in
            [(HooksArg::Rust, HookMode::Rust), (HooksArg::Verify, HookMode::Verify)]
        {
            switch.apply(&mut machine, mode).unwrap();
            let table = machine.hook_table().expect("a table is installed");
            assert_eq!(table.len(), routines);
            assert!(table.entries().all(|entry| entry.mode == hook_mode));
        }
        switch.apply(&mut machine, HooksArg::Original).unwrap();
        assert!(machine.hook_table().is_none(), "the original code again");
        assert_eq!(HooksArg::ALL.map(HooksArg::label), ["Rust", "Verify", "Original"]);
    }

    #[test]
    fn the_panel_sets_the_mode_of_each_routine_live() {
        let switch = HookSwitch::load(Some(Path::new(REPO_SYMBOLS)), None).unwrap();
        let names: Vec<String> =
            switch.symbols().routines().iter().map(|routine| routine.name.clone()).collect();
        let (mut machine, _store) = crate::test_support::sram_counter_machine();
        // One routine in Rust and one in Verify, the others original: each in its mode.
        let modes: Vec<(String, HookMode)> = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let mode = match i {
                    0 => HookMode::Rust,
                    1 => HookMode::Verify,
                    _ => HookMode::Original,
                };
                (name.clone(), mode)
            })
            .collect();
        switch.apply_modes(&mut machine, &modes).unwrap();
        let table = machine.hook_table().expect("routines are hooked");
        assert_eq!(table.len(), names.len());
        for (name, mode) in &modes {
            let entry = table.entries().find(|entry| entry.name == *name).expect("in the table");
            assert_eq!(entry.mode, *mode, "{name}");
        }
        // Every routine original again: no table, as in the original mode.
        let original: Vec<(String, HookMode)> =
            names.iter().map(|name| (name.clone(), HookMode::Original)).collect();
        switch.apply_modes(&mut machine, &original).unwrap();
        assert!(machine.hook_table().is_none());
        assert_eq!(HooksArg::of(HookMode::Verify), HooksArg::Verify);
    }

    #[test]
    fn switching_needs_a_symbols_file_that_loads() {
        let dir = tempfile::tempdir().unwrap();
        assert!(HookSwitch::load(Some(&dir.path().join("missing.toml")), None).is_err());
    }

    /// The lines of `text` split into words.
    fn words(text: &str) -> Vec<Vec<&str>> {
        text.lines().map(|line| line.split_whitespace().collect()).collect()
    }

    #[test]
    fn the_stats_tables_have_a_row_per_routine_and_the_counters_in_order() {
        let rows = vec![
            (
                "advance_x_by_6".to_owned(),
                HookStats {
                    calls: 10,
                    verified_ok: 8,
                    divergences: 1,
                    interrupted: 0,
                    hdma_crossed: 2,
                    sampled_out: 3,
                    contract_mismatch: 4,
                    stack_switched: 5,
                    no_return: 6,
                    mean_cpu_cycles: 128.0,
                    mean_instructions: 7.25,
                    ..HookStats::default()
                },
            ),
            ("rng_next_a".to_owned(), HookStats::default()),
        ];
        let text = stats_table(&rows);
        let lines = words(&text);
        assert_eq!(
            lines[0],
            [
                "routine",
                "calls",
                "verified_ok",
                "divergences",
                "interrupted",
                "hdma_crossed",
                "sampled_out"
            ]
        );
        assert_eq!(lines[1], ["advance_x_by_6", "10", "8", "1", "0", "2", "3"]);
        assert_eq!(lines[2], ["rng_next_a", "0", "0", "0", "0", "0", "0"]);
        assert_eq!(lines[3], Vec::<&str>::new(), "an empty line between the tables");
        assert_eq!(
            lines[4],
            [
                "routine",
                "contract_mismatch",
                "stack_switched",
                "no_return",
                "mean_cpu_cycles",
                "mean_instructions"
            ]
        );
        assert_eq!(lines[5], ["advance_x_by_6", "4", "5", "6", "128.0", "7.2"]);
        assert_eq!(lines[6], ["rng_next_a", "0", "0", "0", "0.0", "0.0"]);
        assert_eq!(lines.len(), 7, "no extra line for a routine without extras");
    }

    #[test]
    fn the_stats_tables_fit_120_columns_with_the_widest_names_and_counts() {
        let big = 9_999_999_999;
        let loud = HookStats {
            calls: big,
            verified_ok: big,
            divergences: big,
            interrupted: big,
            hdma_crossed: big,
            sampled_out: big,
            contract_mismatch: big,
            stack_switched: big,
            no_return: big,
            mean_cpu_cycles: 20_930.0,
            mean_instructions: 1_234_567.8,
            timing_fallback: big,
            cost_mismatch: big,
            cost_missing: big,
            cost_over_bound: big,
            nested: big,
            nested_overflow: big,
            reentered: big,
            ..HookStats::default()
        };
        let symbols = Symbols::load(REPO_SYMBOLS).unwrap();
        let mut rows: Vec<(String, HookStats)> =
            symbols.routines().iter().map(|routine| (routine.name.clone(), loud)).collect();
        rows.push(("a_routine_name_of_thirty_chars".to_owned(), loud));
        let text = stats_table(&rows);
        let widest = text.lines().map(str::len).max().unwrap();
        assert!(widest <= 120, "{widest} columns:\n{text}");
        // The loud extras do not fit one line: they go on to the next, none of them lost.
        assert!(text.matches("reentered=9999999999").count() == rows.len(), "{text}");
    }

    #[test]
    fn the_counters_beyond_the_tables_go_on_a_line_of_their_own_when_not_zero() {
        let rows = vec![
            (
                "all_of_them".to_owned(),
                HookStats {
                    timing_fallback: 1,
                    cost_mismatch: 2,
                    cost_missing: 3,
                    cost_over_bound: 4,
                    nested: 5,
                    nested_overflow: 6,
                    reentered: 7,
                    ..HookStats::default()
                },
            ),
            ("plain".to_owned(), HookStats { calls: 3, ..HookStats::default() }),
            ("some".to_owned(), HookStats { cost_missing: 9, nested: 2, ..HookStats::default() }),
        ];
        let text = stats_table(&rows);
        let lines = words(&text);
        // Header and three rows of the first table, an empty line, the header of the second.
        assert_eq!(lines[6][0], "all_of_them");
        assert_eq!(
            lines[7],
            [
                "timing_fallback=1",
                "cost_mismatch=2",
                "cost_missing=3",
                "cost_over_bound=4",
                "nested=5",
                "nested_overflow=6",
                "reentered=7"
            ]
        );
        assert_eq!(lines[8][0], "plain");
        assert_eq!(lines[9][0], "some");
        assert_eq!(lines[10], ["cost_missing=9", "nested=2"], "only the ones that are not zero");
        assert_eq!(lines.len(), 11);
        // An extra line is indented under its routine's row.
        let extra = text.lines().nth(7).unwrap();
        assert!(extra.starts_with("    ") && !extra.starts_with("     "), "{text}");
    }

    #[test]
    fn a_divergence_dir_is_only_for_verify() {
        let dir = Path::new("elsewhere");
        assert_eq!(divergence_dir_warning(HooksArg::Verify, Some(dir)), None);
        for mode in [HooksArg::Original, HooksArg::Rust, HooksArg::Verify] {
            assert_eq!(divergence_dir_warning(mode, None), None);
        }
        let rust = divergence_dir_warning(HooksArg::Rust, Some(dir)).unwrap();
        assert!(rust.contains("--divergence-dir elsewhere is ignored"), "{rust}");
        assert!(rust.contains("--hooks rust") && rust.contains("--hooks verify"), "{rust}");
        let original = divergence_dir_warning(HooksArg::Original, Some(dir)).unwrap();
        assert!(original.contains("--hooks original"), "{original}");
        // Ignored: the setup keeps no reports.
        let symbols = Some(Path::new(REPO_SYMBOLS));
        let setup = prepare(HooksArg::Rust, symbols, Some(dir)).unwrap().unwrap();
        assert_eq!(setup.divergence_dir(), None);
    }

    #[test]
    fn divergences_are_summed_over_the_routines() {
        let rows = vec![("a".to_owned(), stats(5, 4, 1)), ("b".to_owned(), stats(9, 6, 3))];
        assert_eq!(divergence_count(&rows), 4);
        assert_eq!(divergence_count(&[("a".to_owned(), stats(5, 5, 0))]), 0);
        assert_eq!(divergence_count(&[]), 0);
    }

    #[test]
    fn report_errors_are_summed_over_the_routines() {
        let failing = |report_errors| HookStats { report_errors, ..HookStats::default() };
        let rows = vec![("a".to_owned(), failing(2)), ("b".to_owned(), failing(1))];
        assert_eq!(report_error_count(&rows), 3);
        assert_eq!(report_error_count(&[("a".to_owned(), stats(5, 5, 0))]), 0);
    }

    #[test]
    fn the_divergence_summary_points_at_the_reports_or_says_they_failed() {
        let dir = Path::new("some-reports");
        let saved = divergence_summary(3, 0, Some(dir));
        assert!(saved.contains("3 divergence(s)"), "{saved}");
        assert!(saved.contains("reports in some-reports"), "{saved}");

        let failed = divergence_summary(3, 2, Some(dir));
        assert!(failed.contains("3 divergence(s)"), "{failed}");
        assert!(failed.contains("2 report(s) could not be saved"), "{failed}");
        assert!(!failed.contains("reports in"), "{failed}");

        let unsaved = divergence_summary(1, 0, None);
        assert!(unsaved.contains("1 divergence(s)") && !unsaved.contains("reports"), "{unsaved}");
    }
}
