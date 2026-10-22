//! The replays tab's state and logic, without drawing (`tab_replays.rs` draws it): the replay
//! files of `.work/replays`, the progress and outcome of a verification run, the divergence
//! reports of `.work/divergences`, the refusals of a recording, and the report of a lockstep
//! divergence.

use ssd_oracle::report::ReproHeader;
use ssd_oracle::{
    CORE_VERSION, LockstepDivergence, LockstepReport, Machine, Replay, Verdict as LockstepVerdict,
};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Where the replays are, relative to the working directory (as `.work/divergences` is).
pub const REPLAYS_DIR: &str = ".work/replays";
/// Divergence reports listed, the newest first.
pub const REPORT_LIMIT: usize = 20;

/// How a verification run ended, or how far it got when it was stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Every check compared equal; `checks` sync events in all.
    Green { checks: u64 },
    /// A check failed at sync event `event`.
    Diverged { event: u64 },
    /// No sync event at all: nothing was compared.
    Unverified,
    /// Stopped by hand (or by an error) at `frame`, before the end.
    Stopped { frame: u64 },
}

impl Outcome {
    /// The line the list shows for an outcome.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Outcome::Green { checks } => format!("green, {checks} checks"),
            Outcome::Diverged { event } => format!("diverged at event {event}"),
            Outcome::Unverified => "unverified: no sync event".to_owned(),
            Outcome::Stopped { frame } => format!("stopped at frame {frame}"),
        }
    }
}

/// A replay file of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayEntry {
    pub name: String,
    pub path: PathBuf,
    /// Its frames, or why it cannot be played on this ROM and build.
    pub frames: Result<u64, String>,
    /// How its last verification in this session ended.
    pub outcome: Option<Outcome>,
}

/// A verification run of a replay: its report so far, and its outcome once it is over.
#[derive(Debug, Clone, PartialEq)]
pub struct Verification {
    pub name: String,
    pub frames_total: u64,
    pub report: LockstepReport,
    pub paused: bool,
    /// How the run ended; `None` while it runs.
    pub outcome: Option<Outcome>,
}

impl Verification {
    /// A run just started, with its first report.
    #[must_use]
    pub fn new(name: String, frames_total: u64, report: LockstepReport) -> Self {
        Self { name, frames_total, report, paused: false, outcome: None }
    }

    /// The share of the replay's frames played, from 0 to 1.
    #[must_use]
    pub fn progress(&self) -> f32 {
        if self.frames_total == 0 {
            return 1.0;
        }
        (self.report.frames as f32 / self.frames_total as f32).clamp(0.0, 1.0)
    }

    /// The differences of the first divergence (0 while there is none).
    #[must_use]
    pub fn differences(&self) -> u64 {
        match &self.report.first_divergence {
            None => 0,
            Some(LockstepDivergence::State { differences, truncated, .. }) => {
                (differences.len() + truncated) as u64
            }
            Some(_) => 1,
        }
    }

    /// Whether the run goes on (it may be paused).
    #[must_use]
    pub fn running(&self) -> bool {
        self.outcome.is_none()
    }
}

/// The outcome a run has, from its report.
#[must_use]
pub fn outcome_of(report: &LockstepReport) -> Outcome {
    match report.verdict() {
        LockstepVerdict::Green => Outcome::Green { checks: report.events },
        LockstepVerdict::Diverged => Outcome::Diverged { event: report.events },
        LockstepVerdict::Unverified => Outcome::Unverified,
    }
}

/// A divergence report of `.work/divergences`: a routine's, which Reproduce runs again, or a
/// replay's lockstep check, which has no routine to run again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportEntry {
    /// The text file; its state is the `.ssdstate` beside it.
    pub text: PathBuf,
    pub state: PathBuf,
    /// The routine's header, for a report that can be run again.
    pub header: Option<ReproHeader>,
    pub title: String,
    /// The lines under the title: the summary, and the first difference.
    pub lines: Vec<String>,
    /// What the last Reproduce said.
    pub verdict: Option<String>,
}

/// The reason a recording cannot start now, if it cannot.
#[must_use]
pub fn refuse_recording(recording: bool, verifying: bool, frozen: bool) -> Option<&'static str> {
    if recording {
        Some("A recording is already running: stop it first.")
    } else if verifying {
        Some("Stop the verification first.")
    } else if frozen {
        Some("A recording is refused while WRAM freezes are set: a replay does not hold them.")
    } else {
        None
    }
}

/// The replay files of `dir`, by name, each with its frames or why it cannot be played on the ROM
/// with CRC32 `rom_crc32`. A directory that does not exist has none.
#[must_use]
pub fn scan_replays(dir: &Path, rom_crc32: u32) -> Vec<ReplayEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut replays: Vec<ReplayEntry> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("ssdreplay"))
        .map(|path| {
            let name =
                path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
            let frames = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| {
                Replay::from_bytes(&bytes, rom_crc32, CORE_VERSION)
                    .map(|replay| replay.frames())
                    .map_err(|e| e.to_string())
            });
            ReplayEntry { name, path, frames, outcome: None }
        })
        .collect();
    replays.sort_by(|a, b| a.name.cmp(&b.name));
    replays
}

/// The divergence reports in `dir` (its text files), the newest first, at most [`REPORT_LIMIT`].
#[must_use]
pub fn scan_reports(dir: &Path) -> Vec<ReportEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut texts: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("txt"))
        .map(|path| (modified(&path), path))
        .collect();
    texts.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    texts.iter().take(REPORT_LIMIT).filter_map(|(_, path)| read_report(path)).collect()
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path).and_then(|meta| meta.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// The report in the text file `text`: a routine's (its first line is a [`ReproHeader`]) or a
/// lockstep check's (`ssd-lockstep: replay=<name> event=<n>`). `None` for any other file.
#[must_use]
pub fn read_report(text: &Path) -> Option<ReportEntry> {
    let contents = std::fs::read_to_string(text).ok()?;
    let mut lines = contents.lines();
    let first = lines.next()?;
    let state = text.with_extension("ssdstate");
    if let Some(header) = ReproHeader::parse(first) {
        let routine = header.routine.clone();
        let summary = after(lines.next(), &format!("{routine}: "));
        let mut shown = vec![summary];
        if let Some(line) = lines.next() {
            shown.push(after(Some(line), &format!("{routine} \u{2014} ")));
        }
        return Some(ReportEntry {
            text: text.to_path_buf(),
            state,
            title: routine,
            header: Some(header),
            lines: shown,
            verdict: None,
        });
    }
    let rest = first.strip_prefix("ssd-lockstep: ")?;
    let field = |name: &str| {
        rest.split_whitespace().find_map(|word| word.strip_prefix(name)).unwrap_or("?").to_owned()
    };
    let mut shown = vec![lines.next().unwrap_or_default().to_owned()];
    if let Some(line) = lines.next() {
        shown.push(line.trim().to_owned());
    }
    Some(ReportEntry {
        text: text.to_path_buf(),
        state,
        header: None,
        title: format!("replay {}, event {}", field("replay="), field("event=")),
        lines: shown,
        verdict: None,
    })
}

/// `line` without `prefix`, if it has it.
fn after(line: Option<&str>, prefix: &str) -> String {
    let line = line.unwrap_or_default();
    line.strip_prefix(prefix).unwrap_or(line).to_owned()
}

/// Saves a lockstep divergence next to the reports of the routines: `lockstep-<replay>-<event>.txt`
/// (its first line names the replay and the event, then the comparison) and the state of the
/// hooked machine at the event, `.ssdstate`, which Open state loads. Returns the text file.
///
/// # Errors
///
/// The directory or a file cannot be written.
pub fn save_lockstep_report(
    dir: &Path,
    replay: &str,
    event: u64,
    divergence: &LockstepDivergence,
    hooked: &Machine,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let file: String = replay
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let stem = format!("lockstep-{file}-{event}");
    std::fs::write(dir.join(format!("{stem}.ssdstate")), hooked.save_state())?;
    let text = dir.join(format!("{stem}.txt"));
    std::fs::write(&text, format!("ssd-lockstep: replay={file} event={event}\n{divergence}"))?;
    Ok(text)
}

/// What the replays tab shows and remembers.
#[derive(Debug, Default)]
pub struct ReplaysState {
    pub replays: Vec<ReplayEntry>,
    pub reports: Vec<ReportEntry>,
    pub verification: Option<Verification>,
    /// The name of the recording in progress, if one is.
    pub recording: Option<String>,
    /// The name of the replay being played, if one is (until it is stopped, even once it has ended).
    pub playing: Option<String>,
    /// What the last action said, when it did not do what was asked.
    pub notice: Option<String>,
}

impl ReplaysState {
    /// Rescans the replay files and the reports in `reports_dir`, keeping this session's outcomes.
    pub fn refresh(&mut self, rom_crc32: u32, reports_dir: &Path) {
        let mut replays = scan_replays(Path::new(REPLAYS_DIR), rom_crc32);
        for replay in &mut replays {
            replay.outcome = self
                .replays
                .iter()
                .find(|known| known.name == replay.name)
                .and_then(|known| known.outcome.clone());
        }
        self.replays = replays;
        self.reports = scan_reports(reports_dir);
    }

    /// Notes how the verification run ended, on it and on its replay's entry.
    pub fn finished(&mut self, outcome: Outcome) {
        let Some(run) = self.verification.as_mut() else { return };
        run.outcome = Some(outcome.clone());
        if let Some(replay) = self.replays.iter_mut().find(|r| r.name == run.name) {
            replay.outcome = Some(outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::sram_counter_machine;
    use ssd_oracle::Joypad;
    use std::collections::BTreeMap;

    fn report(events: u64, frames: u64, first: Option<LockstepDivergence>) -> LockstepReport {
        LockstepReport {
            events,
            frames,
            unsynced_windows: 0,
            max_cycle_drift: 0,
            first_divergence: first,
            hook_stats: Vec::new(),
            sync_stats: Vec::new(),
            widest_dead_stack: 0..0,
        }
    }

    #[test]
    fn the_outcome_reads_as_the_list_shows_it() {
        assert_eq!(Outcome::Green { checks: 3880 }.text(), "green, 3880 checks");
        assert_eq!(Outcome::Diverged { event: 5 }.text(), "diverged at event 5");
        assert_eq!(Outcome::Stopped { frame: 900 }.text(), "stopped at frame 900");
        assert!(Outcome::Unverified.text().starts_with("unverified"));
    }

    #[test]
    fn a_run_ends_green_diverged_or_unverified() {
        assert_eq!(outcome_of(&report(3880, 7200, None)), Outcome::Green { checks: 3880 });
        assert_eq!(outcome_of(&report(0, 60, None)), Outcome::Unverified);
        let cycle = LockstepDivergence::CycleDrift { event: 5, drift: -4 };
        assert_eq!(outcome_of(&report(5, 900, Some(cycle))), Outcome::Diverged { event: 5 });
    }

    #[test]
    fn the_progress_and_the_differences_of_a_run() {
        let state = LockstepDivergence::State {
            event: 2,
            frame: 80,
            differences: Vec::new(),
            truncated: 3,
            labels: BTreeMap::new(),
        };
        let run = Verification::new("battle".to_owned(), 200, report(2, 50, Some(state)));
        assert!((run.progress() - 0.25).abs() < 1e-6);
        assert_eq!(run.differences(), 3);
        assert!(run.running());
        let quiet = Verification::new("wander".to_owned(), 0, report(0, 0, None));
        assert_eq!((quiet.progress(), quiet.differences()), (1.0, 0));
    }

    #[test]
    fn a_recording_is_refused_while_another_runs_verifies_or_freezes() {
        assert_eq!(refuse_recording(false, false, false), None);
        assert!(refuse_recording(true, false, false).unwrap().contains("already"));
        assert!(refuse_recording(false, true, false).unwrap().contains("verification"));
        assert!(refuse_recording(false, false, true).unwrap().contains("freezes"));
        // The first reason wins.
        assert!(refuse_recording(true, true, true).unwrap().contains("already"));
    }

    #[test]
    fn a_routine_report_and_a_lockstep_report_are_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let routine = dir.path().join("rng_next-812-3.txt");
        std::fs::write(
            &routine,
            "ssd-repro: key=$008040 routine=rng_next\n\
             rng_next: call 3 at frame 812, 2 differences\n\
             rng_next \u{2014} WRAM $7E0010 (seed): original $12, rust $13\n",
        )
        .unwrap();
        let entry = read_report(&routine).unwrap();
        assert_eq!(entry.title, "rng_next");
        assert_eq!(entry.lines[0], "call 3 at frame 812, 2 differences");
        assert!(entry.lines[1].starts_with("WRAM $7E0010"), "{:?}", entry.lines);
        assert_eq!(entry.header.as_ref().map(|h| h.key), Some(0x008040));
        assert_eq!(entry.state, dir.path().join("rng_next-812-3.ssdstate"));

        let lockstep = dir.path().join("lockstep-battle-5.txt");
        std::fs::write(
            &lockstep,
            "ssd-lockstep: replay=battle event=5\n\
             event 5 (frame 812): state differs, 1 difference\n  $7E0010 (seed): 12 vs 13\n",
        )
        .unwrap();
        let entry = read_report(&lockstep).unwrap();
        assert_eq!(entry.title, "replay battle, event 5");
        assert_eq!(entry.header, None, "a lockstep check has no routine to run again");
        assert_eq!(
            entry.lines,
            ["event 5 (frame 812): state differs, 1 difference", "$7E0010 (seed): 12 vs 13"]
        );

        std::fs::write(dir.path().join("notes.txt"), "not a report\n").unwrap();
        assert_eq!(read_report(&dir.path().join("notes.txt")), None);
    }

    #[test]
    fn the_reports_are_listed_up_to_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..(REPORT_LIMIT + 3) {
            let text = format!("ssd-lockstep: replay=wander event={n}\nevent {n}: x\n");
            std::fs::write(dir.path().join(format!("lockstep-wander-{n}.txt")), text).unwrap();
        }
        assert_eq!(scan_reports(dir.path()).len(), REPORT_LIMIT);
        assert_eq!(scan_reports(&dir.path().join("missing")).len(), 0);
    }

    #[test]
    fn replays_are_listed_with_their_frames_or_their_error() {
        let dir = tempfile::tempdir().unwrap();
        let replay = Replay {
            rom_crc32: 7,
            wram_seed: 1,
            initial_sram: None,
            mode: 0,
            events: Vec::new(),
            packs: None,
            inputs: vec![Joypad::default(); 3],
        };
        replay.write_to(&dir.path().join("battle.ssdreplay"), CORE_VERSION).unwrap();
        std::fs::write(dir.path().join("broken.ssdreplay"), b"not a replay").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"ignored").unwrap();

        let replays = scan_replays(dir.path(), 7);
        let names: Vec<&str> = replays.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["battle", "broken"]);
        assert_eq!(replays[0].frames, Ok(3));
        assert!(replays[1].frames.as_ref().unwrap_err().contains("SSDRPLAY"));
        // Another ROM: every replay is refused with the reason.
        assert!(scan_replays(dir.path(), 8)[0].frames.as_ref().unwrap_err().contains("ROM"));
    }

    #[test]
    fn a_lockstep_divergence_is_saved_with_the_hooked_state() {
        let dir = tempfile::tempdir().unwrap();
        let machine = sram_counter_machine().0;
        let divergence = LockstepDivergence::EventCount { a: 5, b: 4, frame: 9 };
        let text =
            save_lockstep_report(dir.path(), "battle run", 4, &divergence, &machine).unwrap();
        assert_eq!(text, dir.path().join("lockstep-battle_run-4.txt"));
        assert!(text.with_extension("ssdstate").exists());
        let entry = read_report(&text).unwrap();
        assert_eq!(entry.title, "replay battle_run, event 4");
        assert!(entry.lines[0].contains("event count at frame 9"), "{:?}", entry.lines);
    }
}
