//! The logic of the debug panel and its controls, without any drawing: what is open, which tab,
//! the toast, the emulation controls and the actions that change them. The egui code in `ui.rs`
//! only reads this state and pushes [`PanelAction`]s.

use crate::debugger::{DebugState, StepKind};
use crate::hook_setup::HooksArg;
use crate::hotkeys::{Hotkey, Speed};
use crate::replays::ReplaysState;
use crate::screenshot::Kind;
use crate::tab_memory::MemoryState;
use ssd_oracle::{HookMode, HookStats};

/// Width of the panel, in points.
pub const PANEL_WIDTH: f32 = 440.0;
/// How long a toast stays, in seconds.
pub const TOAST_SECONDS: f64 = 8.0;
/// The save-state slots.
pub const SLOTS: [u8; 4] = [1, 2, 3, 4];
/// How long the notice of a screenshot stays, in seconds.
pub const NOTICE_SECONDS: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Routines,
    Memory,
    Debugger,
    Replays,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Routines, Tab::Memory, Tab::Debugger, Tab::Replays];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Tab::Routines => "Routines",
            Tab::Memory => "Memory",
            Tab::Debugger => "Debugger",
            Tab::Replays => "Replays",
        }
    }
}

/// A message over the game while the panel is closed.
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub text: String,
    /// The tab the toast's "Open" shows.
    pub tab: Tab,
    /// When it goes away, in the clock of the UI (seconds).
    pub until: f64,
}

/// What is open and shown: panel, tab, toast, and what the tabs keep.
#[derive(Debug, Default)]
pub struct PanelState {
    pub open: bool,
    pub tab: Tab,
    pub memory: MemoryState,
    toast: Option<Toast>,
    /// The line shown after a screenshot, and when it goes away (the clock of the UI).
    notice: Option<(String, f64)>,
    pub debug: DebugState,
    pub replays: ReplaysState,
}

impl PanelState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.toast = None;
        }
    }

    /// Opens the panel on `tab`.
    pub fn open_tab(&mut self, tab: Tab) {
        self.open = true;
        self.tab = tab;
        self.toast = None;
    }

    /// Shows a toast until `now + TOAST_SECONDS`; nothing while the panel is open (it has the
    /// information itself).
    pub fn push_toast(&mut self, text: String, tab: Tab, now: f64) {
        if !self.open {
            self.toast = Some(Toast { text, tab, until: now + TOAST_SECONDS });
        }
    }

    /// Shows `text` for [`NOTICE_SECONDS`], panel open or not.
    pub fn push_notice(&mut self, text: String, now: f64) {
        self.notice = Some((text, now + NOTICE_SECONDS));
    }

    /// The notice to show at time `now`, if any. One that ran out is dropped.
    pub fn notice(&mut self, now: f64) -> Option<&str> {
        if self.notice.as_ref().is_some_and(|(_, until)| *until <= now) {
            self.notice = None;
        }
        self.notice.as_ref().map(|(text, _)| text.as_str())
    }

    /// The toast to show at time `now`, if any. One that ran out is dropped.
    pub fn toast(&mut self, now: f64) -> Option<&Toast> {
        if self.toast.as_ref().is_some_and(|toast| toast.until <= now) {
            self.toast = None;
        }
        self.toast.as_ref()
    }
}

/// What the panel and the hotkeys can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelAction {
    TogglePanel,
    OpenTab(Tab),
    TogglePause,
    /// One frame, while paused.
    NextFrame,
    SetSpeed(Speed),
    CycleSpeed,
    SelectSlot(u8),
    SaveState,
    LoadState,
    /// Saves the picture of the window as a PNG (no debug panel in it).
    Screenshot,
    /// Opens the Cheats screen over a battle (Enhanced; the host checks the battle).
    BattleCheats,
    /// Saves the PPU's own frame, unscaled, as a PNG.
    ScreenshotFrame,
    SetHooks(HooksArg),
    /// The mode of one routine, by its hook key (its address), applied live.
    SetRoutineMode {
        key: u32,
        mode: HookMode,
    },
    /// Freezes the selection's symbol (or byte) at its current value.
    FreezeSelected,
    /// Turns the freeze at this index of the Memory tab's list on or off.
    ToggleFreeze(usize),
    Debug(DebugAction),
    Replay(ReplayAction),
}

/// What the debugger tab asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugAction {
    /// A step, while paused.
    Step(StepKind),
    ToggleStopAtInterrupts,
    /// Adds the breakpoint that the target field names.
    AddBreakpoint,
    ToggleBreakpoint(usize),
    RemoveBreakpoint(usize),
}

/// What the replays tab asks for; a `usize` is a row of its list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayAction {
    /// Starts a new recording from power-on.
    Record,
    StopRecording,
    /// Rescans the replay files and the divergence reports.
    Refresh,
    /// Ends the replay being played: the machine goes on with the live buttons.
    StopPlayback,
    Play(usize),
    Verify(usize),
    /// Pauses a verification run, or goes on with it.
    TogglePauseVerify,
    StopVerify,
    /// Runs the divergence report of the row again (a routine's report only).
    Reproduce(usize),
    /// Loads the state of the divergence report of the row into the machine.
    OpenState(usize),
}

impl PanelAction {
    /// The action a hotkey stands for; `None` for quitting.
    #[must_use]
    pub fn from_hotkey(hotkey: Hotkey) -> Option<Self> {
        Some(match hotkey {
            Hotkey::SaveState => Self::SaveState,
            Hotkey::LoadState => Self::LoadState,
            Hotkey::Slot(n) => Self::SelectSlot(n),
            Hotkey::CycleSpeed => Self::CycleSpeed,
            Hotkey::Pause => Self::TogglePause,
            Hotkey::Screenshot => Self::Screenshot,
            Hotkey::TogglePanel => Self::TogglePanel,
            Hotkey::BattleCheats => Self::BattleCheats,
            Hotkey::Quit => return None,
        })
    }
}

/// Where a key press goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRoute {
    Quit,
    Action(PanelAction),
    /// The game's joypad.
    Game,
    /// Nobody: a text field of the panel has the keyboard.
    TextField,
}

/// Decides where the press of a key goes. `hotkey` is what the key stands for, if anything;
/// `text_focus` whether a text field has the keyboard. A text field gets every key but F1, and
/// the keys that are neither hotkeys nor a text field's go to the game.
#[must_use]
pub fn route_key(hotkey: Option<Hotkey>, text_focus: bool) -> KeyRoute {
    if text_focus && hotkey != Some(Hotkey::TogglePanel) {
        return KeyRoute::TextField;
    }
    match hotkey {
        None => KeyRoute::Game,
        Some(hotkey) => PanelAction::from_hotkey(hotkey).map_or(KeyRoute::Quit, KeyRoute::Action),
    }
}

/// What the app has to do to the machine for an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Save(u8),
    Load(u8),
    /// A screenshot of this kind, taken after the next frame is drawn.
    Screenshot(Kind),
    /// Run one frame now.
    StepFrame,
    /// Open the Cheats screen over the battle, if the game is in one.
    BattleCheats,
    SetHooks(HooksArg),
    SetRoutineMode {
        key: u32,
        mode: HookMode,
    },
    FreezeSelected,
    /// The freezes that are on changed: the machine takes them again.
    ApplyFreezes,
}

/// Whether the hook mode can be changed while the app runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HooksSwitch {
    Allowed,
    /// Not allowed; the text says why.
    Locked(&'static str),
}

/// The emulation controls: the same state whether they are set by a hotkey or by the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Controls {
    pub paused: bool,
    pub speed: Speed,
    pub slot: u8,
    /// The mode every routine has, while they all have one; after a change of single routines
    /// it is the last mode set for all of them, and `hooks_mixed` tells that they differ.
    pub hooks: HooksArg,
    /// The routines have different modes (set one by one in the Routines tab).
    pub hooks_mixed: bool,
    pub hooks_switch: HooksSwitch,
    /// Loading a state is refused while a replay is recorded or played.
    pub load_allowed: bool,
    /// Freezes can be made and changed: they are refused while a replay is recorded.
    pub freezes_allowed: bool,
    /// Whether the debugger's steps may run: nothing records or verifies, so the steps can
    /// change the machine freely. The app sets it each frame.
    pub can_step: bool,
    /// A verification runs: it has the machines, so the game does not step.
    pub verifying: bool,
}

impl Controls {
    #[must_use]
    pub fn new(
        hooks: HooksArg,
        hooks_switch: HooksSwitch,
        load_allowed: bool,
        freezes_allowed: bool,
    ) -> Self {
        Self {
            paused: false,
            speed: Speed::default(),
            slot: 1,
            hooks,
            hooks_mixed: false,
            hooks_switch,
            load_allowed,
            freezes_allowed,
            can_step: false,
            verifying: false,
        }
    }

    /// Applies an action to the controls and the panel state; returns what is left for the
    /// machine to do. A load while loading is refused, a next frame while running and a hook
    /// mode that cannot be changed or is the current one do nothing.
    pub fn apply(&mut self, action: PanelAction, panel: &mut PanelState) -> Option<Effect> {
        match action {
            PanelAction::TogglePanel => panel.toggle(),
            PanelAction::OpenTab(tab) => panel.open_tab(tab),
            PanelAction::TogglePause => self.paused = !self.paused,
            PanelAction::NextFrame if self.paused && !self.verifying => {
                return Some(Effect::StepFrame);
            }
            PanelAction::SetSpeed(speed) => self.speed = speed,
            PanelAction::CycleSpeed => self.speed = self.speed.next(),
            PanelAction::SelectSlot(slot) if SLOTS.contains(&slot) => self.slot = slot,
            PanelAction::SaveState => return Some(Effect::Save(self.slot)),
            PanelAction::LoadState if self.load_allowed => return Some(Effect::Load(self.slot)),
            PanelAction::Screenshot => return Some(Effect::Screenshot(Kind::Window)),
            PanelAction::BattleCheats => return Some(Effect::BattleCheats),
            PanelAction::ScreenshotFrame => return Some(Effect::Screenshot(Kind::Frame)),
            // Setting every routine to the mode they have is nothing, unless they differ.
            PanelAction::SetHooks(mode)
                if self.hooks_switch == HooksSwitch::Allowed
                    && (mode != self.hooks || self.hooks_mixed) =>
            {
                return Some(Effect::SetHooks(mode));
            }
            PanelAction::SetRoutineMode { key, mode }
                if self.hooks_switch == HooksSwitch::Allowed =>
            {
                return Some(Effect::SetRoutineMode { key, mode });
            }
            PanelAction::FreezeSelected if self.freezes_allowed => {
                return Some(Effect::FreezeSelected);
            }
            PanelAction::ToggleFreeze(index) => {
                if self.freezes_allowed && panel.memory.toggle_freeze(index) {
                    return Some(Effect::ApplyFreezes);
                }
            }
            // A next frame while running, a slot that does not exist, a load that is refused, a
            // hook mode that cannot be changed or is the current one, a routine or freeze that
            // cannot change: nothing to do. The debugger's and replays' actions change the
            // machine too: the app carries them out.
            PanelAction::NextFrame
            | PanelAction::SelectSlot(_)
            | PanelAction::LoadState
            | PanelAction::SetHooks(_)
            | PanelAction::SetRoutineMode { .. }
            | PanelAction::FreezeSelected
            | PanelAction::Debug(_)
            | PanelAction::Replay(_) => {}
        }
        None
    }

    /// Every routine has `mode` now (after [`Effect::SetHooks`] succeeded).
    pub fn set_hooks(&mut self, mode: HooksArg) {
        self.hooks = mode;
        self.hooks_mixed = false;
    }

    /// The routines have different modes now (after [`Effect::SetRoutineMode`] succeeded).
    pub fn set_mixed(&mut self) {
        self.hooks_mixed = true;
    }
}

/// Emulated frames per second over the last half second or so.
#[derive(Debug, Default)]
pub struct FpsCounter {
    start: f64,
    frames: u32,
    fps: f32,
}

impl FpsCounter {
    const WINDOW: f64 = 0.5;

    /// Counts `frames` more frames at time `now` (seconds, increasing); returns the rate of the
    /// last full window (0 until the first one is over).
    pub fn add(&mut self, now: f64, frames: u32) -> f32 {
        self.frames += frames;
        let elapsed = now - self.start;
        if elapsed >= Self::WINDOW {
            self.fps = (f64::from(self.frames) / elapsed) as f32;
            self.start = now;
            self.frames = 0;
        }
        self.fps
    }
}

/// Notices when a routine's divergence count goes up.
#[derive(Debug, Default)]
pub struct DivergenceWatch {
    seen: Vec<(String, u64)>,
}

impl DivergenceWatch {
    /// Takes the counters of the machine; returns the first routine whose divergence count is
    /// higher than at the last call (a first call compares with zero). A count that went down
    /// (the routine was reinstalled, its counters reset) is not an event.
    pub fn update(&mut self, stats: &[(String, HookStats)]) -> Option<String> {
        let mut found = None;
        let mut seen = Vec::with_capacity(stats.len());
        for (name, stat) in stats {
            let before = self.seen.iter().find(|(n, _)| n == name).map_or(0, |&(_, count)| count);
            if stat.divergences > before && found.is_none() {
                found = Some(name.clone());
            }
            seen.push((name.clone(), stat.divergences));
        }
        self.seen = seen;
        found
    }

    /// Takes `stats` as the baseline without reporting anything: after a table change the counts it
    /// brings are old news, so only a divergence from now on is.
    pub fn reseed(&mut self, stats: &[(String, HookStats)]) {
        let _ = self.update(stats);
    }
}

/// The share of the window's instructions that ran as Rust, as the header shows it ("0.7%"); a
/// dash where there is nothing to show yet.
#[must_use]
pub fn rust_share_text(percent: Option<f64>) -> String {
    percent.map_or_else(|| "\u{2014}".to_owned(), |p| format!("{p:.1}%"))
}

/// A number with a space between thousands ("4 812").
#[must_use]
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls() -> (Controls, PanelState) {
        (Controls::new(HooksArg::Original, HooksSwitch::Allowed, true, true), PanelState::default())
    }

    #[test]
    fn the_panel_starts_closed_and_f1_toggles_it() {
        let (mut c, mut p) = controls();
        assert!(!p.open);
        assert_eq!(c.apply(PanelAction::TogglePanel, &mut p), None);
        assert!(p.open);
        c.apply(PanelAction::TogglePanel, &mut p);
        assert!(!p.open);
        assert_eq!(p.tab, Tab::Routines, "the tab is kept");
    }

    #[test]
    fn pause_and_next_frame() {
        let (mut c, mut p) = controls();
        assert_eq!(c.apply(PanelAction::NextFrame, &mut p), None, "running: nothing to step");
        c.apply(PanelAction::TogglePause, &mut p);
        assert!(c.paused);
        assert_eq!(c.apply(PanelAction::NextFrame, &mut p), Some(Effect::StepFrame));
        assert!(c.paused, "stepping keeps the pause");
        c.apply(PanelAction::TogglePause, &mut p);
        assert!(!c.paused);
    }

    #[test]
    fn speed_is_set_by_the_panel_and_cycled_by_the_hotkey() {
        let (mut c, mut p) = controls();
        c.apply(PanelAction::SetSpeed(Speed::ALL[2]), &mut p);
        assert_eq!(c.speed.multiplier(), 4);
        c.apply(PanelAction::CycleSpeed, &mut p);
        assert_eq!(c.speed.multiplier(), 8);
        c.apply(PanelAction::CycleSpeed, &mut p);
        assert_eq!(c.speed.multiplier(), 1);
    }

    #[test]
    fn save_and_load_use_the_selected_slot() {
        let (mut c, mut p) = controls();
        assert_eq!(c.apply(PanelAction::SaveState, &mut p), Some(Effect::Save(1)));
        c.apply(PanelAction::SelectSlot(3), &mut p);
        assert_eq!(c.apply(PanelAction::SaveState, &mut p), Some(Effect::Save(3)));
        assert_eq!(c.apply(PanelAction::LoadState, &mut p), Some(Effect::Load(3)));
        c.apply(PanelAction::SelectSlot(9), &mut p);
        assert_eq!(c.slot, 3, "there is no slot 9");
    }

    #[test]
    fn loading_is_refused_while_a_replay_is_recorded_or_played() {
        let mut c = Controls::new(HooksArg::Original, HooksSwitch::Allowed, false, true);
        let mut p = PanelState::default();
        assert_eq!(c.apply(PanelAction::LoadState, &mut p), None);
        assert_eq!(c.apply(PanelAction::SaveState, &mut p), Some(Effect::Save(1)));
    }

    #[test]
    fn the_hook_mode_changes_only_when_allowed_and_different() {
        let (mut c, mut p) = controls();
        assert_eq!(c.apply(PanelAction::SetHooks(HooksArg::Original), &mut p), None, "same mode");
        assert_eq!(
            c.apply(PanelAction::SetHooks(HooksArg::Rust), &mut p),
            Some(Effect::SetHooks(HooksArg::Rust))
        );
        assert_eq!(c.hooks, HooksArg::Original, "the mode changes when the machine has it");
        c.set_hooks(HooksArg::Rust);
        assert_eq!(c.apply(PanelAction::SetHooks(HooksArg::Rust), &mut p), None);

        let mut locked =
            Controls::new(HooksArg::Original, HooksSwitch::Locked("recording"), true, false);
        assert_eq!(locked.apply(PanelAction::SetHooks(HooksArg::Verify), &mut p), None);
    }

    #[test]
    fn one_routine_changes_its_mode_and_mixed_modes_can_be_set_back_for_all() {
        let (mut c, mut p) = controls();
        let rust = PanelAction::SetRoutineMode { key: 0x8000, mode: HookMode::Rust };
        assert_eq!(
            c.apply(rust, &mut p),
            Some(Effect::SetRoutineMode { key: 0x8000, mode: HookMode::Rust })
        );
        c.set_mixed();
        assert!(c.hooks_mixed);
        // Mixed, the header still sets every routine: even to the mode it names.
        assert_eq!(
            c.apply(PanelAction::SetHooks(HooksArg::Original), &mut p),
            Some(Effect::SetHooks(HooksArg::Original))
        );
        c.set_hooks(HooksArg::Original);
        assert!(!c.hooks_mixed);
        assert_eq!(c.apply(PanelAction::SetHooks(HooksArg::Original), &mut p), None);

        let mut locked =
            Controls::new(HooksArg::Original, HooksSwitch::Locked("recording"), true, false);
        assert_eq!(locked.apply(rust, &mut p), None, "locked while recording");
    }

    #[test]
    fn freezes_are_refused_while_recording_and_toggle_the_list_otherwise() {
        let (mut c, mut p) = controls();
        let freeze = crate::tab_memory::PanelFreeze {
            name: "active_side".to_owned(),
            freeze: ssd_oracle::Freeze { address: 0x7E_14C2, bytes: vec![1] },
            active: true,
        };
        p.memory.add_freeze(freeze);
        assert_eq!(c.apply(PanelAction::ToggleFreeze(0), &mut p), Some(Effect::ApplyFreezes));
        assert!(!p.memory.freezes[0].active);
        assert_eq!(c.apply(PanelAction::ToggleFreeze(7), &mut p), None, "there is no freeze 7");
        assert_eq!(c.apply(PanelAction::FreezeSelected, &mut p), Some(Effect::FreezeSelected));

        let mut recording = Controls::new(HooksArg::Original, HooksSwitch::Allowed, false, false);
        assert_eq!(recording.apply(PanelAction::FreezeSelected, &mut p), None);
        assert_eq!(recording.apply(PanelAction::ToggleFreeze(0), &mut p), None);
        assert!(!p.memory.freezes[0].active, "refused: unchanged");
    }

    #[test]
    fn hotkeys_become_actions() {
        use PanelAction as A;
        assert_eq!(A::from_hotkey(Hotkey::TogglePanel), Some(A::TogglePanel));
        assert_eq!(A::from_hotkey(Hotkey::Slot(2)), Some(A::SelectSlot(2)));
        assert_eq!(A::from_hotkey(Hotkey::Pause), Some(A::TogglePause));
        assert_eq!(A::from_hotkey(Hotkey::Screenshot), Some(A::Screenshot));
        assert_eq!(A::from_hotkey(Hotkey::CycleSpeed), Some(A::CycleSpeed));
        assert_eq!(A::from_hotkey(Hotkey::SaveState), Some(A::SaveState));
        assert_eq!(A::from_hotkey(Hotkey::LoadState), Some(A::LoadState));
        assert_eq!(A::from_hotkey(Hotkey::Quit), None);
    }

    #[test]
    fn keys_go_to_the_text_field_the_hotkeys_or_the_game() {
        let route = |hotkey, text| route_key(hotkey, text);
        assert_eq!(route(None, false), KeyRoute::Game);
        assert_eq!(route(Some(Hotkey::Pause), false), KeyRoute::Action(PanelAction::TogglePause));
        assert_eq!(route(Some(Hotkey::Quit), false), KeyRoute::Quit);
        assert_eq!(
            route(Some(Hotkey::TogglePanel), false),
            KeyRoute::Action(PanelAction::TogglePanel)
        );
        // Typing: nothing but F1 reaches the game or the hotkeys, not even Esc or P.
        assert_eq!(route(None, true), KeyRoute::TextField);
        assert_eq!(route(Some(Hotkey::Pause), true), KeyRoute::TextField);
        assert_eq!(route(Some(Hotkey::Quit), true), KeyRoute::TextField);
        assert_eq!(
            route(Some(Hotkey::TogglePanel), true),
            KeyRoute::Action(PanelAction::TogglePanel)
        );
    }

    #[test]
    fn a_notice_lasts_two_seconds_with_the_panel_open_or_closed() {
        let mut p = PanelState::default();
        p.push_notice("Screenshot saved: a.png".to_owned(), 10.0);
        assert_eq!(p.notice(11.9), Some("Screenshot saved: a.png"));
        assert_eq!(p.notice(12.0), None, "expired");
        p.open = true;
        p.push_notice("again".to_owned(), 20.0);
        assert_eq!(p.notice(20.0), Some("again"), "the open panel shows it too");
    }

    #[test]
    fn the_screenshot_actions_ask_the_app_for_a_picture() {
        let mut panel = PanelState::default();
        let mut controls = Controls::new(HooksArg::Original, HooksSwitch::Allowed, true, true);
        assert_eq!(
            controls.apply(PanelAction::Screenshot, &mut panel),
            Some(Effect::Screenshot(Kind::Window))
        );
        assert_eq!(
            controls.apply(PanelAction::ScreenshotFrame, &mut panel),
            Some(Effect::Screenshot(Kind::Frame))
        );
    }

    #[test]
    fn a_toast_lasts_a_while_and_only_shows_with_the_panel_closed() {
        let (mut c, mut p) = controls();
        p.push_toast("Divergence in x".to_owned(), Tab::Replays, 100.0);
        assert_eq!(p.toast(100.0).map(|t| t.text.as_str()), Some("Divergence in x"));
        assert!(p.toast(100.0 + TOAST_SECONDS - 0.1).is_some());
        assert!(p.toast(100.0 + TOAST_SECONDS).is_none(), "expired");
        p.push_toast("again".to_owned(), Tab::Replays, 200.0);
        c.apply(PanelAction::OpenTab(Tab::Replays), &mut p);
        assert!(p.open && p.tab == Tab::Replays);
        assert!(p.toast(200.0).is_none(), "opening the panel takes the toast away");
        p.push_toast("open".to_owned(), Tab::Replays, 300.0);
        assert!(p.toast(300.0).is_none(), "no toast over an open panel");
    }

    #[test]
    fn fps_is_the_rate_of_the_last_full_window() {
        let mut fps = FpsCounter::default();
        assert!(fps.add(0.1, 6) < 0.001, "no full window yet");
        let value = fps.add(0.5, 24);
        assert!((value - 60.0).abs() < 0.01, "30 frames in 0.5 s: {value}");
        // The next window is x2 speed: 60 frames in 0.5 s.
        assert!((fps.add(0.75, 30) - 60.0).abs() < 0.01, "same value until the window ends");
        assert!((fps.add(1.0, 30) - 120.0).abs() < 0.01);
    }

    #[test]
    fn a_rising_divergence_count_is_reported_once() {
        let stat = |divergences| HookStats { divergences, ..HookStats::default() };
        let mut watch = DivergenceWatch::default();
        let table = |a, b| vec![("rng_a".to_owned(), stat(a)), ("rng_b".to_owned(), stat(b))];
        assert_eq!(watch.update(&table(0, 0)), None);
        assert_eq!(watch.update(&table(0, 2)), Some("rng_b".to_owned()));
        assert_eq!(watch.update(&table(0, 2)), None, "reported once");
        assert_eq!(watch.update(&table(0, 0)), None, "a reset is not an event");
        assert_eq!(watch.update(&table(1, 1)), Some("rng_a".to_owned()));
        assert_eq!(watch.update(&[]), None);
    }

    #[test]
    fn a_reseeded_watch_reports_only_the_divergences_after_it() {
        let stat = |divergences| HookStats { divergences, ..HookStats::default() };
        let table = |a| vec![("rng_a".to_owned(), stat(a))];
        let mut watch = DivergenceWatch::default();
        watch.reseed(&table(2));
        assert_eq!(watch.update(&table(2)), None, "the counts it was given are old news");
        assert_eq!(watch.update(&table(3)), Some("rng_a".to_owned()), "a new one is news");
    }

    #[test]
    fn numbers_are_written_for_the_header() {
        assert_eq!(rust_share_text(Some(0.66)), "0.7%");
        assert_eq!(rust_share_text(None), "\u{2014}");
        assert_eq!(grouped(4812), "4 812");
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(1_210_259), "1 210 259");
        assert_eq!(grouped(999), "999");
    }
}
