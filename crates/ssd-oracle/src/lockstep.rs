//! Lockstep (spec §5.4, "Whole run"): a replay played on two machines side by side, A the pure
//! original and B with hooks (Rust routines), compared at every sync event. This is the whole-run
//! check of the Rust routines: with exact costs and the timing-event window, B stays on A's
//! timeline cycle for cycle, so at each sync event both machines must be in the same state at the
//! same master cycle.
//!
//! # What is compared
//!
//! Both machines run the replay with [`Machine::run_until_sync`] (each tick fed the input of its
//! frame), A first, B up to the same frame. At each sync event (the first arrival at a sync point
//! after an NMI), in this order, the first mismatch being the run's divergence:
//!
//! 1. the event's frame and the NMIs taken since the last comparison:
//!    [`LockstepDivergence::LagDrift`];
//! 2. the event's master cycle, `cycle_drift = master_cycles_b − master_cycles_a`:
//!    [`LockstepDivergence::CycleDrift`] when not 0;
//! 3. the state: the CPU registers (A, X, Y, S, D, DBR, PBR, PC, P, E), WRAM except the dead
//!    stack, SRAM, VRAM, CGRAM and OAM: [`LockstepDivergence::State`] (A's values are the
//!    `original` ones of each [`Difference`], B's the `rust` ones).
//!
//! An event that only one machine reaches within the window is a
//! [`LockstepDivergence::EventCount`].
//!
//! B's table holds routines in `Rust` (or `Original`) mode, and sync points only at the sync keys
//! (see [`run_lockstep`]).
//!
//! # The dead stack
//!
//! A Rust routine does not push what its original pushes, so the stack bytes the original pushed
//! to below S keep other values on B (and original code a Rust routine calls can push deeper on
//! B than on A). At an event, the dead stack is the WRAM from one above the **stack floor** up
//! to S (bank 0, below `$2000`), the floor being the lower of both machines' floors: the lowest
//! S on each machine's current stack, followed after every tick, the ticks of the original code
//! run inside dispatches included, with one rule for stack switches on both machines (a `TCS`
//! that frees or allocates a stack frame stays on the stack, a jump far down or above the
//! stack's base switches; see `StackWatch` in `sync.rs`). Every byte there was pushed to and is
//! free at the event; bytes below the floor are compared.
//!
//! The floor is not reset at events, nor taken at hook hits only: a dead byte one machine pushed
//! to and the other did not keeps its value until both push over it, possibly many events later,
//! and the bytes a call pushes to lie below the S of its hook hit. (On the real game, with every
//! routine in Rust, the events of frames 6051–6062 of the battle replay show three differing dead
//! bytes, `$0FDF–$0FE1`, while the stack reached no lower than `$0FE4` since each previous event.)
//!
//! It heals instead: after each event that compared equal, both machines' floors rise to just
//! below the lowest dead byte that still differs, or to S if none does. So a deep push (at boot,
//! a rare deep call) stops widening the dead stack once both machines agree there again, and a
//! later write there is compared; pushes lower the floor again as they come.
//!
//! # Unsynced windows
//!
//! A runs at most `budget_frames` frames to its next event. When the budget runs out without one
//! (NMI off: the intro, mode transitions), B runs to the same frame, and only the NMIs taken over
//! the window are compared, never the memories: a dispatch that renders a frame returns after its
//! call, so B can stop past the frame boundary where A stops on it. Such a window counts in
//! `unsynced_windows`; the replay's tail after the last event (shorter than the budget) is
//! compared the same way without counting (there B may stand a few frames past the replay's end, inside a
//! Rust routine that began before it: only the NMIs are compared then).
//!
//! A run without any event compared nothing: it is [`Verdict::Unverified`], never green.
//!
//! An arrival of B at a sync point inside a dispatch (original code a Rust routine calls) is no
//! event, even where it would be one at the top level: the dispatch runs to its end. Such
//! arrivals are counted in the sync points' counters ([`LockstepReport::sync_stats`],
//! `sync_inside_dispatch`); lockstep itself is unchanged by them.

use crate::compare::{DifferenceText, compare_except, keep_first, wram_labels};
use crate::verify::dead_stack;
use crate::{
    Difference, HookAction, HookEntry, HookMode, HookStats, HookTable, Joypad, Machine, NullAudio,
    OracleError, Outcome, PackTag, Replay, Symbols, SyncEvent, hook_key,
};
use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

/// Frames A may run without a sync event before the window is compared on its NMIs only (see
/// [`LockstepConfig::budget_frames`]).
pub const DEFAULT_BUDGET_FRAMES: u64 = 600;

/// How a lockstep run is set up.
#[derive(Clone)]
pub struct LockstepConfig {
    /// The hooks of machine B (`None`: none, B runs as A): routines in `Rust` or `Original` mode,
    /// sync points only at `sync_keys` (see [`run_lockstep`]).
    pub table: Option<HookTable>,
    /// Hook keys (24-bit addresses) of the sync points, installed on both machines.
    pub sync_keys: Vec<u32>,
    /// Frames A may run without a sync event: then B runs to the same frame and the window is
    /// compared on its NMIs only (an unsynced window). At least 1 (0 counts as 1).
    pub budget_frames: u64,
    /// The pack set and its image machine B runs on, instead of the ROM (`None`: B runs on the ROM as
    /// A does). B then plays the replay as a replay of that set ([`Replay::packs`] is set to the tag
    /// for it); A stays on the ROM alone.
    pub b_packs: Option<(PackTag, Vec<u8>)>,
}

impl Default for LockstepConfig {
    fn default() -> Self {
        Self {
            table: None,
            sync_keys: Vec::new(),
            budget_frames: DEFAULT_BUDGET_FRAMES,
            b_packs: None,
        }
    }
}

/// What a lockstep run found.
#[derive(Debug, Clone, PartialEq)]
pub struct LockstepReport {
    /// Sync events compared equal (both machines, every check).
    pub events: u64,
    /// Frames A ran: the replay's, or up to the first divergence.
    pub frames: u64,
    /// Windows of `budget_frames` frames without a sync event, compared on their NMIs only.
    pub unsynced_windows: u64,
    /// The cycle drift of largest magnitude among the events compared (0 if none): B's master
    /// cycle at the event minus A's.
    pub max_cycle_drift: i64,
    /// The first divergence; the run stops there.
    pub first_divergence: Option<LockstepDivergence>,
    /// B's dispatch counters of every routine of the table, in key order (sync points left out).
    pub hook_stats: Vec<(String, HookStats)>,
    /// B's counters of the sync points, in key order: an arrival inside a dispatch that would
    /// have been an event ([`HookStats::sync_inside_dispatch`]) was not compared where A's was.
    pub sync_stats: Vec<(String, HookStats)>,
    /// The widest dead stack an event excluded from the comparison, as WRAM offsets (the first
    /// of the widest; empty if none).
    pub widest_dead_stack: Range<u32>,
}

/// The outcome of a lockstep run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every event compared equal, and there was at least one.
    Green,
    /// The machines diverged.
    Diverged,
    /// No divergence, but no sync event either: nothing was compared beyond NMI counts.
    Unverified,
}

impl LockstepReport {
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        if self.first_divergence.is_some() {
            Verdict::Diverged
        } else if self.events == 0 {
            Verdict::Unverified
        } else {
            Verdict::Green
        }
    }
}

/// How machine B (hooked) differs from A (the pure original). `event` is the index of the sync
/// event (0 = the first), the count of the events compared equal before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockstepDivergence {
    /// Both machines are at the event, at the same frame and master cycle, in different states:
    /// the first 256 differences in [`compare`](crate::compare)'s order (A's value as
    /// `original`, B's as `rust`).
    State {
        event: u64,
        frame: u64,
        differences: Vec<Difference>,
        /// Differences found beyond those kept.
        truncated: usize,
        /// The variable each kept WRAM difference falls in, by WRAM offset (see
        /// [`Divergence::labels`](crate::Divergence::labels)).
        labels: BTreeMap<u32, String>,
    },
    /// The event (or, in an unsynced window, the window's end) at different frames, or after a
    /// different number of NMIs since the last comparison (`nmis_a`, `nmis_b`).
    LagDrift { event: u64, frame_a: u64, frame_b: u64, nmis_a: u64, nmis_b: u64 },
    /// The event at the same frame after the same NMIs, `drift` master cycles later on B
    /// (negative: earlier).
    CycleDrift { event: u64, drift: i64 },
    /// Only one machine reached the next event within the window, at `frame`: A had `a` events,
    /// B `b`.
    EventCount { a: u64, b: u64, frame: u64 },
}

/// One line (`event 5 (frame 812): state differs, 2 differences`, then one indented line per
/// kept difference, A's value first, and the count of the others); every line ends with a
/// newline.
impl fmt::Display for LockstepDivergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::State { event, frame, differences, truncated, labels } => {
                let total = differences.len() + truncated;
                let plural = if total == 1 { "" } else { "s" };
                writeln!(
                    f,
                    "event {event} (frame {frame}): state differs, {total} difference{plural}"
                )?;
                for difference in differences {
                    writeln!(f, "  {}", DifferenceText { difference, labels })?;
                }
                if *truncated > 0 {
                    writeln!(f, "  … {truncated} more differences")?;
                }
                Ok(())
            }
            Self::LagDrift { event, frame_a, frame_b, nmis_a, nmis_b } => writeln!(
                f,
                "event {event}: lag drift: the original is at frame {frame_a} after {nmis_a} \
                 NMI(s), the hooked machine at frame {frame_b} after {nmis_b}"
            ),
            Self::CycleDrift { event, drift } => {
                let way = if *drift < 0 { "earlier" } else { "later" };
                writeln!(
                    f,
                    "event {event}: cycle drift: the hooked machine arrives {} master cycles \
                     {way} than the original",
                    drift.unsigned_abs()
                )
            }
            Self::EventCount { a, b, frame } => writeln!(
                f,
                "event count at frame {frame}: the original reached {a} sync events, the hooked \
                 machine {b}"
            ),
        }
    }
}

/// A lockstep run played step by step: the machines of [`run_lockstep`], advanced one sync event
/// or one unsynced window at a time (or in slices of frames), for a debugger that shows the
/// comparison as it goes. It owns its replay and both machines ([`LockstepRun::machine_a`],
/// [`LockstepRun::machine_b`]).
///
/// [`run_lockstep`] is a loop over [`LockstepRun::next`], so a run stopped after any number of
/// steps and resumed ends with the report of the one-shot run, and so does one advanced with
/// [`LockstepRun::next_within`] in slices of any size.
///
/// A run whose step returned an error must be discarded: its machines may be part-way through a
/// window, and calling it again would not give the report of any run.
pub struct LockstepRun {
    a: Machine,
    b: Machine,
    replay: Arc<Replay>,
    symbols: Option<Symbols>,
    /// Names of the table's routines (the others in `hook_stats()` are sync points).
    routines: Vec<String>,
    budget: u64,
    /// [`Machine::nmi_count`] of each machine at the last comparison (0 at power-on).
    nmis_a: u64,
    nmis_b: u64,
    /// The window in progress, A having run part of it.
    window: Option<Window>,
    report: LockstepReport,
}

/// What a machine found in its window: its first sync event, or none before the window's end.
#[derive(Debug, Clone, Copy)]
enum Found {
    Event(SyncEvent),
    NoEvent,
}

/// A window the machines run: each ends at its next sync event or at the frame count `end`.
#[derive(Clone, Copy)]
struct Window {
    /// A's frame count at the window's start.
    start: u64,
    end: u64,
    /// The window is a whole budget (not the replay's tail).
    full: bool,
    /// What each machine found in the window; `None` while it still runs the window.
    a: Option<Found>,
    b: Option<Found>,
}

/// What one step of a [`LockstepRun`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockstepStep {
    /// A sync event compared equal: both machines are at it, in the same state.
    Event {
        /// Index of the event (0 = the first): the events compared equal before.
        index: u64,
        /// The frame of the event.
        frame: u64,
        /// NMIs taken since the last comparison.
        nmis: u64,
        /// B's master cycle at the event minus A's (always 0 here: a drift is a divergence).
        drift: i64,
        /// The dead stack the comparison excluded, as WRAM offsets (empty if none).
        dead_stack: Range<u32>,
    },
    /// A window without a sync event compared equal on its frames and NMI counts: `frames`
    /// frames of A, `nmis` NMIs taken. Every such window is reported, the replay's shorter tail
    /// included (only the windows of a whole budget count in
    /// [`LockstepReport::unsynced_windows`]).
    UnsyncedWindow { frames: u64, nmis: u64 },
    /// [`LockstepRun::next_within`] used up its frames: A is at `frame`, and the event or window
    /// goes on with the next call.
    Running { frame: u64 },
    /// The machines diverged (the run is over: [`LockstepRun::next`] gives `None` from here on).
    Diverged(LockstepDivergence),
}

impl LockstepRun {
    /// Powers both machines on and sets up the comparison as [`run_lockstep`] does.
    ///
    /// # Errors
    ///
    /// As for [`run_lockstep`], except for the runs themselves.
    pub fn new(
        rom: Vec<u8>,
        replay: Arc<Replay>,
        config: &LockstepConfig,
        symbols: Option<&Symbols>,
    ) -> Result<Self, OracleError> {
        let sync_keys: Vec<u32> = config.sync_keys.iter().map(|&key| hook_key(key)).collect();
        for entry in config.table.iter().flat_map(HookTable::entries) {
            let refused = match entry.action {
                HookAction::Routine { .. } if entry.mode == HookMode::Verify => format!(
                    "`{}` is in Verify mode; lockstep runs routines in Rust (a Verify call \
                     spanning a window's end lets the hooked machine take an NMI the original \
                     takes later)",
                    entry.name
                ),
                HookAction::Routine { .. } if sync_keys.contains(&entry.key) => format!(
                    "the sync point at ${:06X} is the entry of `{}` in the hook table",
                    entry.key, entry.name
                ),
                HookAction::SyncPoint if !sync_keys.contains(&entry.key) => format!(
                    "the hook table's sync point `{}` at ${:06X} is not a sync key (only the \
                     hooked machine would have events there)",
                    entry.name, entry.key
                ),
                _ => continue,
            };
            return Err(OracleError::Lockstep(refused));
        }
        let sync_points: Vec<HookEntry> = sync_keys
            .iter()
            .map(|&key| HookEntry::sync_point(sync_point_name(key, symbols), key))
            .collect();
        let mut table_b = config.table.clone().unwrap_or_default();
        for point in &sync_points {
            table_b.insert(point.clone());
        }

        let mut a = replay.power_on(rom.clone())?;
        let mut b = match &config.b_packs {
            None => replay.power_on(rom)?,
            Some((tag, image)) => {
                let mut on_packs = (*replay).clone();
                on_packs.packs = Some(*tag);
                on_packs.power_on_image(image.clone())?
            }
        };
        a.set_hook_table(Some(sync_points.into_iter().collect()));
        b.set_hook_table(Some(table_b));
        a.watch_stack();
        b.watch_stack();
        let routines = config
            .table
            .iter()
            .flat_map(HookTable::entries)
            .filter(|entry| matches!(entry.action, HookAction::Routine { .. }))
            .map(|entry| entry.name.clone())
            .collect();
        Ok(Self {
            a,
            b,
            replay,
            symbols: symbols.cloned(),
            routines,
            budget: config.budget_frames.max(1),
            nmis_a: 0,
            nmis_b: 0,
            window: None,
            report: LockstepReport {
                events: 0,
                frames: 0,
                unsynced_windows: 0,
                max_cycle_drift: 0,
                first_divergence: None,
                hook_stats: Vec::new(),
                sync_stats: Vec::new(),
                widest_dead_stack: 0..0,
            },
        })
    }

    /// Runs to the next sync event, or through the next window without one, and compares the
    /// machines there ([`LockstepStep::Event`], [`LockstepStep::UnsyncedWindow`], or
    /// [`LockstepStep::Diverged`]); `None` once the replay is played or after the divergence.
    ///
    /// # Errors
    ///
    /// The errors of the runs ([`Machine::run_until_sync`]).
    #[allow(clippy::should_implement_trait)] // fallible, and not an iterator over borrowed state
    pub fn next(&mut self) -> Result<Option<LockstepStep>, OracleError> {
        self.next_within(u64::MAX)
    }

    /// [`LockstepRun::next`], but each machine runs at most `frames` frames (at least 1) in this
    /// call: when they are used up before the machine's event or the window ends, the step is
    /// [`LockstepStep::Running`] and the next call goes on, with the accounting of `next`. A and
    /// B run in the same calls, so a window of N frames without events takes ceil(N / `frames`)
    /// calls and never runs more than `frames` frames of either machine in one of them.
    ///
    /// # Errors
    ///
    /// As for [`LockstepRun::next`]; after one, the run must be discarded (see [`LockstepRun`]).
    pub fn next_within(&mut self, frames: u64) -> Result<Option<LockstepStep>, OracleError> {
        if self.report.first_divergence.is_some() {
            return Ok(None);
        }
        let mut window = match self.window {
            Some(window) => window,
            None => {
                let start = self.a.frame_count();
                if start >= self.replay.frames() {
                    return Ok(None);
                }
                let end = (start + self.budget).min(self.replay.frames());
                Window { start, end, full: end - start == self.budget, a: None, b: None }
            }
        };
        let replay = Arc::clone(&self.replay);
        let mut input = |frame| replay.input(frame);
        let frames = frames.max(1);
        if window.a.is_none() {
            window.a = slice(&mut self.a, &mut input, window.end, frames)?;
        }
        if window.b.is_none() {
            window.b = slice(&mut self.b, &mut input, window.end, frames)?;
        }
        let (Some(event_a), Some(event_b)) = (window.a, window.b) else {
            self.window = Some(window);
            return Ok(Some(LockstepStep::Running { frame: self.a.frame_count() }));
        };
        self.window = None;
        let index = self.report.events;
        let result = match (event_a, event_b) {
            (Found::Event(a), Found::Event(b)) => self.compare_event(&a, &b),
            (Found::Event(a), Found::NoEvent) => {
                Err(LockstepDivergence::EventCount { a: index + 1, b: index, frame: a.frame })
            }
            (Found::NoEvent, Found::Event(b)) => {
                Err(LockstepDivergence::EventCount { a: index, b: index + 1, frame: b.frame })
            }
            (Found::NoEvent, Found::NoEvent) => self.compare_window(window),
        };
        Ok(Some(match result {
            Ok(step) => step,
            Err(divergence) => {
                self.report.first_divergence = Some(divergence.clone());
                LockstepStep::Diverged(divergence)
            }
        }))
    }

    /// What the run found so far: the counters, the first divergence, and B's routine and sync
    /// point counters as they are now. `frames` is A's frame count.
    #[must_use]
    pub fn report(&self) -> LockstepReport {
        let mut report = self.report.clone();
        report.frames = self.a.frame_count();
        (report.hook_stats, report.sync_stats) =
            self.b.hook_stats().into_iter().partition(|(name, _)| self.routines.contains(name));
        report
    }

    /// Machine A, the pure original (with the sync points).
    #[must_use]
    pub fn machine_a(&self) -> &Machine {
        &self.a
    }

    /// Machine B, with the hooks.
    #[must_use]
    pub fn machine_b(&self) -> &Machine {
        &self.b
    }

    /// The window ended without an event on either machine: compares frames and NMIs.
    fn compare_window(&mut self, window: Window) -> Result<LockstepStep, LockstepDivergence> {
        let (frame_a, mut frame_b) = (self.a.frame_count(), self.b.frame_count());
        // The replay's last window: a Rust routine that began before the end is atomic, so B can
        // finish it a few frames past the recorded end where A, stopped on the last frame, is
        // still inside the original's run of it (the NMIs are still compared).
        if window.end == self.replay.frames() && frame_b > frame_a {
            frame_b = frame_a;
        }
        let nmis = self.a.nmi_count() - self.nmis_a;
        self.lag_drift(frame_a, frame_b, self.a.nmi_count(), self.b.nmi_count())?;
        if window.full {
            self.report.unsynced_windows += 1;
        }
        Ok(LockstepStep::UnsyncedWindow { frames: frame_a - window.start, nmis })
    }

    /// A [`LockstepDivergence::LagDrift`] if the frames or the NMIs taken since the last
    /// comparison differ; otherwise the NMI counts become the last comparison's.
    fn lag_drift(
        &mut self,
        frame_a: u64,
        frame_b: u64,
        nmi_count_a: u64,
        nmi_count_b: u64,
    ) -> Result<(), LockstepDivergence> {
        let (nmis_a, nmis_b) = (nmi_count_a - self.nmis_a, nmi_count_b - self.nmis_b);
        if frame_a != frame_b || nmis_a != nmis_b {
            let event = self.report.events;
            return Err(LockstepDivergence::LagDrift { event, frame_a, frame_b, nmis_a, nmis_b });
        }
        (self.nmis_a, self.nmis_b) = (nmi_count_a, nmi_count_b);
        Ok(())
    }

    /// Compares both machines at their events `a` and `b` (see the module documentation); after
    /// an equal comparison, heals the stack floor.
    fn compare_event(
        &mut self,
        a: &SyncEvent,
        b: &SyncEvent,
    ) -> Result<LockstepStep, LockstepDivergence> {
        let nmis = a.nmi_count - self.nmis_a;
        self.lag_drift(a.frame, b.frame, a.nmi_count, b.nmi_count)?;
        let event = self.report.events;
        let drift = b.master_cycles.wrapping_sub(a.master_cycles) as i64;
        if drift.unsigned_abs() > self.report.max_cycle_drift.unsigned_abs() {
            self.report.max_cycle_drift = drift;
        }
        if drift != 0 {
            return Err(LockstepDivergence::CycleDrift { event, drift });
        }

        let s = self.a.cpu_registers().s.min(self.b.cpu_registers().s);
        let floor = |m: &Machine| m.stack_floor().unwrap_or(s);
        let dead = dead_stack(floor(&self.a).min(floor(&self.b)), s);
        let mut differences = compare_except(
            &Outcome::of(&self.a, &[]),
            &Outcome::of(&self.b, &[]),
            0xFF,
            false,
            dead.clone(),
            usize::MAX,
        );
        if !differences.is_empty() {
            let truncated = keep_first(&mut differences);
            let labels = wram_labels(&differences, self.symbols.as_ref());
            let frame = a.frame;
            return Err(LockstepDivergence::State { event, frame, differences, truncated, labels });
        }
        let dead_stack = dead.start as u32..dead.end as u32;
        if dead.len() > self.report.widest_dead_stack.len() {
            self.report.widest_dead_stack = dead_stack.clone();
        }
        self.heal_stack_floor(dead, s);
        self.report.events += 1;
        Ok(LockstepStep::Event { index: event, frame: a.frame, nmis, drift, dead_stack })
    }

    /// Raises both machines' stack floors to just below the lowest byte of `dead` (the dead
    /// stack just compared, up to S = `s`) that still differs, or to `s` if none does.
    fn heal_stack_floor(&mut self, dead: Range<usize>, s: u16) {
        let (a, b) = (&self.a.wram()[dead.clone()], &self.b.wram()[dead.clone()]);
        let differing = a.iter().zip(b).position(|(a, b)| a != b);
        let floor = differing.map_or(s, |offset| (dead.start + offset - 1) as u16);
        self.a.raise_stack_floor(floor);
        self.b.raise_stack_floor(floor);
    }
}

/// Plays `replay` on two machines powered on from `rom` (see [`Replay::power_on`]): A with the
/// sync points of `config` only, B with `config.table` and the sync points, and compares them as
/// the module documentation says until the replay's end or the first divergence ([`LockstepRun`]
/// to the end). `symbols` names the sync points (else `sync_point_<key>`) and the WRAM bytes of a
/// state divergence.
///
/// The table's routines run in `Rust` mode (or not at all, in `Original` mode). `Verify` is
/// refused: a `Verify` call runs the original to its return inside its dispatch, so one that
/// spans the end of a window lets B take an NMI there that A, stopped on the frame boundary,
/// takes in the next window (a false lag drift); its divergences are not lockstep's either. A
/// sync point of the table is one of `sync_keys` (B alone would have events at another one).
///
/// # Errors
///
/// [`OracleError::Lockstep`] if the table has a routine in `Verify` mode, a sync point that is
/// not a sync key, or an entry at a sync key that is no sync point; the errors of
/// [`Replay::power_on`] (another ROM, SRAM that does not fit) and of the runs.
pub fn run_lockstep(
    rom: Vec<u8>,
    replay: &Replay,
    config: &LockstepConfig,
    symbols: Option<&Symbols>,
) -> Result<LockstepReport, OracleError> {
    let mut run = LockstepRun::new(rom, Arc::new(replay.clone()), config, symbols)?;
    while run.next()?.is_some() {}
    Ok(run.report())
}

/// Runs `machine` up to `frames` frames of its window, which ends at the frame count `end`: its
/// first sync event there, or [`Found::NoEvent`] once it reached `end` without one.
fn slice(
    machine: &mut Machine,
    input: &mut dyn FnMut(u64) -> Joypad,
    end: u64,
    frames: u64,
) -> Result<Option<Found>, OracleError> {
    if machine.frame_count() >= end {
        return Ok(Some(Found::NoEvent));
    }
    let slice = frames.min(end - machine.frame_count());
    match machine.run_until_sync(input, &mut NullAudio, slice)? {
        Some(event) => Ok(Some(Found::Event(event))),
        None if machine.frame_count() >= end => Ok(Some(Found::NoEvent)),
        None => Ok(None),
    }
}

/// The name of the sync point at `key`: its `[[sync_point]]` in `symbols`, else
/// `sync_point_<key>`.
fn sync_point_name(key: u32, symbols: Option<&Symbols>) -> String {
    let key = hook_key(key);
    symbols
        .and_then(|symbols| symbols.sync_points().iter().find(|point| point.address == key))
        .map_or_else(|| format!("sync_point_{key:06x}"), |point| point.name.clone())
}
