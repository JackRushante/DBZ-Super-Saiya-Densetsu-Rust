//! Touch controls (M9): an overlay of buttons drawn over the game and driven by SDL's finger events. It feeds the
//! same input state as the keyboard and the gamepad (`InputState::touch`), so the game, the HD menus and the
//! shortcuts work unchanged, and a replay (which records the SNES buttons) does not depend on it.
//!
//! This module is the model and the input: the player's choices (`settings`), where the buttons are (`layout`), what
//! each finger holds, the MENU, the options and the edit mode (this file), and the drawing (`draw`, with egui).
//! Nothing here touches the machine: the host takes the [`Out`] events and acts on them.

pub mod draw;
pub mod layout;
pub mod settings;

use egui::{Pos2, Rect, Vec2, pos2, vec2};
use layout::{Btn, Geometry, MIN_DP, Shape};
use settings::{Group, OPACITY_MAX, OPACITY_MIN, Orient, SCALE_MAX, SCALE_MIN, Settings};

/// What the touch layer asks of the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Out {
    /// The speed button: next of x1/x2/x4/x8.
    Speed,
    /// MENU: Save state, Load state.
    Save,
    Load,
    /// Pause (true) or let the game go (false); the host pauses only if the player had not.
    Pause(bool),
    /// A light tick under the finger.
    Haptic,
    /// The choices changed: write them to the settings file.
    Persist,
    /// The GAME page: end the game and start it again at the title screen.
    Restart,
    /// The GAME page: end the game and close the app.
    Quit,
}

/// What the screen shows besides the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Play,
    Menu,
    Options,
    /// The GAME page: restart, quit.
    Game,
    Edit,
}

/// A button of the MENU, the options page or the edit panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PBtn {
    Save,
    Load,
    Options,
    Edit,
    HideControls,
    Game,
    Resume,
    Restart,
    Quit,
    OptLayout,
    OptOpacity,
    OptHaptics,
    OptAuto,
    OptHidePad,
    OptReset,
    OptFps,
    OptPicture,
    Back,
    SizeDown,
    SizeUp,
    OpDown,
    OpUp,
    EditReset,
    Done,
}

#[derive(Debug, Clone)]
pub struct PanelButton {
    pub id: PBtn,
    pub rect: Rect,
    pub label: String,
    /// The main action: orange instead of dark red.
    pub primary: bool,
}

/// The cream window over the game: title, buttons and a line of help.
#[derive(Debug, Clone)]
pub struct Panel {
    pub rect: Rect,
    pub title: &'static str,
    pub buttons: Vec<PanelButton>,
    pub note: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    Dpad,
    Stick,
    Face,
    Hold(Btn),
    Tap(Btn),
    Panel(PBtn),
    /// Edit: moving a group; `off` is the group's centre minus the finger's.
    Drag(Group, Vec2),
    /// Edit: resizing a group by its corner.
    Handle(Group),
    /// The small chip that brings the hidden controls back.
    Show,
    /// A finger that holds nothing.
    None,
}

#[derive(Debug, Clone, Copy)]
struct Finger {
    id: i64,
    target: Target,
    pos: Pos2,
    /// The stick's origin (where the thumb came down).
    origin: Pos2,
}

/// The touch layer's state.
pub struct Touch {
    pub enabled: bool,
    pub settings: Settings,
    pub mode: Mode,
    /// The battle layout (only the d-pad, a big A and B).
    pub minimal: bool,
    /// The player hid the controls from the MENU.
    pub hidden: bool,
    /// A gamepad is connected.
    pub gamepad: bool,
    /// The group the edit mode has selected.
    pub selected: Group,
    /// The speed button's label, the slot of the states and whether the game is in a battle: told by the host.
    pub speed_label: &'static str,
    pub slot: u8,
    in_battle: bool,
    /// The line of the frame rate and the sound buffer, told by the host; drawn while Show FPS is on.
    pub stats: Option<String>,
    geo: Geometry,
    fingers: Vec<Finger>,
    pressed: [bool; 12],
    out: Vec<Out>,
    /// The GAME page's button tapped once: a second tap does it (the progress since the last save is lost).
    armed: Option<PBtn>,
}

static FORCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--touch-ui`: the touch controls on a desktop run too (a UI snapshot, or a touch screen).
pub fn force(on: bool) {
    FORCED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether this run has the touch controls: always on Android, on request elsewhere.
#[must_use]
pub fn wanted() -> bool {
    cfg!(target_os = "android") || FORCED.load(std::sync::atomic::Ordering::Relaxed)
}

/// The command the activity (`GameActivity.java`) answers with a light haptic tick (SDL forwards user commands
/// from 0x8000 to its UI thread; 0x8001 is the first-launch status page).
#[cfg(target_os = "android")]
const HAPTIC_COMMAND: u32 = 0x8002;

/// A light tick under the finger: on Android the activity performs the system's keyboard-tap feedback
/// (no permission needed); elsewhere nothing.
pub fn haptic() {
    #[cfg(target_os = "android")]
    // SAFETY: a plain SDL call; the activity's handler runs it on the UI thread.
    unsafe {
        sdl3::sys::system::SDL_SendAndroidMessage(HAPTIC_COMMAND, 0);
    }
}

/// The index of a SNES button in the arrays of `InputState` (its `ALL`).
fn slot_of(btn: Btn) -> Option<usize> {
    Some(match btn {
        Btn::Up => 0,
        Btn::Down => 1,
        Btn::Left => 2,
        Btn::Right => 3,
        Btn::A => 4,
        Btn::B => 5,
        Btn::X => 6,
        Btn::Y => 7,
        Btn::L => 8,
        Btn::R => 9,
        Btn::Start => 10,
        Btn::Select => 11,
        Btn::Speed | Btn::Menu | Btn::Full | Btn::Min => return None,
    })
}

const SLOP: f32 = 6.0;

impl Touch {
    #[must_use]
    pub fn new(enabled: bool, settings: Settings, screen: Rect) -> Self {
        let geo = layout::compute(screen, false, &settings);
        Self {
            enabled,
            settings,
            mode: Mode::Play,
            minimal: false,
            hidden: false,
            gamepad: false,
            selected: Group::Dpad,
            speed_label: "x1",
            slot: 1,
            in_battle: false,
            stats: None,
            geo,
            fingers: Vec::new(),
            pressed: [false; 12],
            out: Vec::new(),
            armed: None,
        }
    }

    #[must_use]
    pub fn geometry(&self) -> &Geometry {
        &self.geo
    }

    /// The screen changed size (or a choice changed the layout): the buttons are worked out again.
    pub fn set_screen(&mut self, screen: Rect) {
        if screen != self.geo.screen {
            self.geo = layout::compute(screen, self.minimal, &self.settings);
        }
    }

    fn relayout(&mut self) {
        self.geo = layout::compute(self.geo.screen, self.minimal, &self.settings);
    }

    /// The host's news of every frame: the speed label, the slot, a gamepad, a battle.
    pub fn set_status(&mut self, speed_label: &'static str, slot: u8, gamepad: bool, battle: bool) {
        self.speed_label = speed_label;
        self.slot = slot;
        self.gamepad = gamepad;
        if battle != self.in_battle {
            self.in_battle = battle;
            if self.settings.auto_battle && self.mode == Mode::Play && self.minimal != battle {
                self.set_minimal(battle);
            }
        }
    }

    fn set_minimal(&mut self, minimal: bool) {
        if self.minimal != minimal {
            self.minimal = minimal;
            self.relayout();
            self.fingers.clear();
            self.recompute();
        }
    }

    /// Whether the controls are on screen (not hidden by a gamepad).
    #[must_use]
    pub fn visible(&self) -> bool {
        self.enabled && !(self.gamepad && self.settings.hide_pad)
    }

    /// The SNES buttons the fingers hold.
    #[must_use]
    pub fn pressed(&self) -> [bool; 12] {
        self.pressed
    }

    pub fn take_out(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.out)
    }

    fn emit(&mut self, out: Out) {
        if out != Out::Haptic || self.settings.haptics {
            self.out.push(out);
        }
    }

    /// Whether a finger holds this button (for the drawing: a pressed button turns solid cream).
    #[must_use]
    pub fn is_down(&self, btn: Btn) -> bool {
        if let Some(i) = slot_of(btn) {
            return self.pressed[i];
        }
        self.fingers.iter().any(|f| f.target == Target::Tap(btn))
    }

    /// The floating stick on screen: its base and its knob.
    #[must_use]
    pub fn stick(&self) -> Option<(Pos2, Pos2)> {
        self.fingers.iter().find(|f| f.target == Target::Stick).map(|f| (f.origin, f.pos))
    }

    /// The chip that brings the controls back.
    #[must_use]
    pub fn show_chip(&self) -> Rect {
        let s = self.geo.screen;
        Rect::from_min_size(pos2(s.max.x - 64.0, s.min.y + 8.0), vec2(56.0, 44.0))
    }

    // --- the panels ----------------------------------------------------------------------------

    /// The window of the MENU, the options or the edit mode, if one is up.
    #[must_use]
    pub fn panel(&self) -> Option<Panel> {
        let s = self.geo.screen;
        let w = 400.0_f32.min(s.width() - 16.0);
        let col = (w - 36.0) / 2.0;
        let cell = |p: &mut Panel, id, label: String, col_i: f32, row: f32, primary| {
            let r = Rect::from_min_size(
                p.rect.min + vec2(14.0 + col_i * (col + 8.0), 50.0 + row * 56.0),
                vec2(col, MIN_DP),
            );
            p.buttons.push(PanelButton { id, rect: r, label, primary });
        };
        let make = |h: f32, title| {
            let rect = Rect::from_center_size(s.center(), vec2(w, h.min(s.height() - 8.0)));
            Panel { rect, title, buttons: Vec::new(), note: "" }
        };
        match self.mode {
            Mode::Play => None,
            Mode::Menu => {
                let mut p = make(338.0, "PAUSED");
                cell(
                    &mut p,
                    PBtn::Save,
                    format!("Save state (slot {})", self.slot),
                    0.0,
                    0.0,
                    true,
                );
                cell(
                    &mut p,
                    PBtn::Load,
                    format!("Load state (slot {})", self.slot),
                    1.0,
                    0.0,
                    false,
                );
                cell(&mut p, PBtn::Options, "Options".into(), 0.0, 1.0, false);
                cell(&mut p, PBtn::Edit, "Edit layout".into(), 1.0, 1.0, false);
                cell(&mut p, PBtn::HideControls, "Hide controls".into(), 0.0, 2.0, false);
                cell(&mut p, PBtn::Game, "Game: restart, quit".into(), 1.0, 2.0, false);
                cell(&mut p, PBtn::Resume, "Resume".into(), 1.0, 3.0, true);
                p.note = "Menu pauses the game. Slots 1 to 4 as on the PC (F5, F9).";
                Some(p)
            }
            Mode::Game => {
                let mut p = make(338.0, "GAME");
                let armed = |id, idle: &str, again: &str| {
                    if self.armed == Some(id) { again.to_owned() } else { idle.to_owned() }
                };
                let restart = armed(PBtn::Restart, "Restart game", "Tap again: restart");
                cell(&mut p, PBtn::Restart, restart, 0.0, 0.0, false);
                cell(
                    &mut p,
                    PBtn::Quit,
                    armed(PBtn::Quit, "Quit", "Tap again: quit"),
                    1.0,
                    0.0,
                    false,
                );
                cell(&mut p, PBtn::Back, "Back".into(), 0.0, 3.0, true);
                p.note = "Restart and Quit lose what was played since the last save.";
                Some(p)
            }
            Mode::Options => {
                let mut p = make(394.0, "TOUCH OPTIONS");
                let st = &self.settings;
                cell(
                    &mut p,
                    PBtn::OptLayout,
                    format!("Layout: {}", if st.stick { "Stick" } else { "D-pad" }),
                    0.0,
                    0.0,
                    false,
                );
                cell(
                    &mut p,
                    PBtn::OptOpacity,
                    format!("Opacity: {}%", st.opacity),
                    1.0,
                    0.0,
                    false,
                );
                cell(
                    &mut p,
                    PBtn::OptHaptics,
                    format!("Haptics: {}", if st.haptics { "On" } else { "Off" }),
                    0.0,
                    1.0,
                    false,
                );
                cell(
                    &mut p,
                    PBtn::OptAuto,
                    format!("Auto battle: {}", if st.auto_battle { "On" } else { "Off" }),
                    1.0,
                    1.0,
                    false,
                );
                cell(
                    &mut p,
                    PBtn::OptHidePad,
                    format!("Hide w/ pad: {}", if st.hide_pad { "On" } else { "Off" }),
                    0.0,
                    2.0,
                    false,
                );
                cell(&mut p, PBtn::OptReset, "Reset layout".into(), 1.0, 2.0, false);
                cell(
                    &mut p,
                    PBtn::OptFps,
                    format!("Show FPS: {}", if st.show_fps { "On" } else { "Off" }),
                    0.0,
                    3.0,
                    false,
                );
                cell(
                    &mut p,
                    PBtn::OptPicture,
                    format!("Picture: {}", st.picture.name()),
                    1.0,
                    3.0,
                    false,
                );
                cell(&mut p, PBtn::Back, "Back".into(), 0.0, 4.0, true);
                p.note = "Saved on this device only, in ssd.settings. Auto battle switches to the minimal layout in fights.";
                Some(p)
            }
            Mode::Edit => {
                let mut p = make(332.0, "EDIT LAYOUT");
                let row = |p: &mut Panel, minus, plus, y: f32| {
                    p.buttons.push(PanelButton {
                        id: minus,
                        rect: Rect::from_min_size(p.rect.min + vec2(84.0, y), vec2(MIN_DP, MIN_DP)),
                        label: "-".into(),
                        primary: false,
                    });
                    p.buttons.push(PanelButton {
                        id: plus,
                        rect: Rect::from_min_size(
                            p.rect.min + vec2(268.0, y),
                            vec2(MIN_DP, MIN_DP),
                        ),
                        label: "+".into(),
                        primary: false,
                    });
                };
                row(&mut p, PBtn::SizeDown, PBtn::SizeUp, 62.0);
                row(&mut p, PBtn::OpDown, PBtn::OpUp, 118.0);
                cell(&mut p, PBtn::EditReset, "Reset to default".into(), 0.0, 4.0, false);
                cell(&mut p, PBtn::Done, "Done".into(), 1.0, 4.0, true);
                p.note = "Drag a control, or its square handles to resize it. Saved on this device only.";
                Some(p)
            }
        }
    }

    // --- fingers -------------------------------------------------------------------------------

    fn find(&self, id: i64) -> Option<usize> {
        self.fingers.iter().position(|f| f.id == id)
    }

    fn handles(&self) -> Vec<(Group, [Pos2; 4])> {
        match self.geo.group_bounds(self.selected) {
            Some(b) => {
                let b = b.expand(6.0);
                vec![(
                    self.selected,
                    [b.left_top(), b.right_top(), b.left_bottom(), b.right_bottom()],
                )]
            }
            None => Vec::new(),
        }
    }

    /// What a finger coming down at `pos` takes, in play.
    fn play_target(&self, pos: Pos2) -> Target {
        let g = &self.geo;
        for item in &g.items {
            let tap = matches!(item.btn, Btn::Speed | Btn::Menu | Btn::Full | Btn::Min);
            let hold = matches!(item.btn, Btn::L | Btn::R | Btn::Start | Btn::Select);
            if (tap || hold) && item.shape.hit(pos, SLOP) {
                return if tap { Target::Tap(item.btn) } else { Target::Hold(item.btn) };
            }
        }
        let face = |slop: f32| {
            g.items.iter().filter(|i| i.group == Group::Face).any(|i| i.shape.hit(pos, slop))
        };
        if face(SLOP) {
            return Target::Face;
        }
        if let Some(zone) = g.stick_zone {
            if zone.contains(pos) {
                return Target::Stick;
            }
        } else if g.group(Group::Dpad).next().is_some() {
            let (c, half) = g.dpad;
            let r = Rect::from_center_size(c, vec2(2.0 * half, 2.0 * half)).expand(14.0);
            if r.contains(pos) {
                return Target::Dpad;
            }
        }
        if face(24.0) {
            return Target::Face;
        }
        Target::None
    }

    /// What a finger coming down at `pos` takes, in the edit mode.
    fn edit_target(&self, pos: Pos2) -> Target {
        if let Some(panel) = self.panel() {
            if let Some(b) = panel.buttons.iter().find(|b| b.rect.contains(pos)) {
                return Target::Panel(b.id);
            }
            if panel.rect.contains(pos) {
                return Target::None;
            }
        }
        for (g, corners) in self.handles() {
            if corners.iter().any(|c| c.distance(pos) <= 26.0) {
                return Target::Handle(g);
            }
        }
        // The group under the finger (the selected one first), else the nearest within reach.
        let mut best: Option<(f32, Group)> = None;
        for g in Group::ALL {
            let Some(b) = self.geo.group_bounds(g) else { continue };
            let reach = if self.geo.group(g).any(|i| i.shape.hit(pos, SLOP)) || b.contains(pos) {
                0.0
            } else if b.expand(10.0).contains(pos) {
                5.0
            } else {
                continue;
            };
            let bias = if g == self.selected { -1.0 } else { 0.0 };
            let score = reach + bias + b.center().distance(pos) * 0.001;
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, g));
            }
        }
        match best {
            Some((_, g)) => {
                let c = layout::placed(self.geo.orient, g, self.geo.screen, &self.settings).0;
                Target::Drag(g, c - pos)
            }
            None => Target::None,
        }
    }

    pub fn finger_down(&mut self, id: i64, pos: Pos2) {
        if !self.enabled || self.find(id).is_some() {
            return;
        }
        let mut finger = Finger { id, target: Target::None, pos, origin: pos };
        match self.mode {
            Mode::Play => {
                if self.hidden {
                    if self.show_chip().expand(8.0).contains(pos) {
                        finger.target = Target::Show;
                    }
                } else if self.visible() {
                    finger.target = self.play_target(pos);
                    match finger.target {
                        Target::Tap(btn) => {
                            self.emit(Out::Haptic);
                            self.fingers.push(finger);
                            self.tap(btn);
                            return;
                        }
                        Target::Stick => self.emit(Out::Haptic),
                        _ => {}
                    }
                }
            }
            Mode::Menu | Mode::Options | Mode::Game => {
                if let Some(b) =
                    self.panel().and_then(|p| p.buttons.into_iter().find(|b| b.rect.contains(pos)))
                {
                    finger.target = Target::Panel(b.id);
                }
            }
            Mode::Edit => {
                finger.target = self.edit_target(pos);
                match finger.target {
                    Target::Drag(g, _) | Target::Handle(g) => {
                        self.selected = g;
                    }
                    _ => {}
                }
            }
        }
        self.fingers.push(finger);
        self.recompute();
    }

    pub fn finger_motion(&mut self, id: i64, pos: Pos2) {
        let Some(i) = self.find(id) else { return };
        self.fingers[i].pos = pos;
        if self.mode == Mode::Edit {
            match self.fingers[i].target {
                Target::Drag(g, off) => self.drag_to(g, pos + off),
                Target::Handle(g) => self.resize_to(g, pos),
                _ => {}
            }
        }
        self.recompute();
    }

    pub fn finger_up(&mut self, id: i64, cancelled: bool) {
        let Some(i) = self.find(id) else { return };
        let f = self.fingers.remove(i);
        match f.target {
            Target::Panel(b) if !cancelled => {
                let inside = self
                    .panel()
                    .and_then(|p| p.buttons.into_iter().find(|x| x.id == b))
                    .is_some_and(|x| x.rect.expand(8.0).contains(f.pos));
                if inside {
                    self.emit(Out::Haptic);
                    self.panel_press(b);
                }
            }
            Target::Show if !cancelled && self.show_chip().expand(8.0).contains(f.pos) => {
                self.hidden = false;
                self.emit(Out::Pause(false));
            }
            _ => {}
        }
        self.recompute();
    }

    /// Lets go of every finger (the app lost the focus).
    pub fn cancel_all(&mut self) {
        self.fingers.clear();
        self.recompute();
    }

    /// The Android back key: the MENU, or back out of one.
    pub fn back_key(&mut self) {
        if !self.enabled {
            return;
        }
        match self.mode {
            Mode::Play => self.open_menu(),
            Mode::Menu => self.panel_press(PBtn::Resume),
            Mode::Options | Mode::Game => self.panel_press(PBtn::Back),
            Mode::Edit => self.panel_press(PBtn::Done),
        }
    }

    fn open_menu(&mut self) {
        self.mode = Mode::Menu;
        self.fingers.clear();
        self.emit(Out::Pause(true));
        self.recompute();
    }

    fn tap(&mut self, btn: Btn) {
        match btn {
            Btn::Speed => self.emit(Out::Speed),
            Btn::Menu => self.open_menu(),
            Btn::Full => self.set_minimal(false),
            Btn::Min => self.set_minimal(true),
            _ => {}
        }
    }

    fn persist(&mut self) {
        self.relayout();
        self.emit(Out::Persist);
    }

    fn panel_press(&mut self, b: PBtn) {
        // Restart and Quit lose the play since the last save: the first tap only arms the button.
        let confirmed = matches!(b, PBtn::Restart | PBtn::Quit);
        if confirmed && self.armed != Some(b) {
            self.armed = Some(b);
            return;
        }
        self.armed = None;
        match b {
            PBtn::Save => {
                self.emit(Out::Save);
                self.close_menu();
            }
            PBtn::Load => {
                self.emit(Out::Load);
                self.close_menu();
            }
            PBtn::Options => self.mode = Mode::Options,
            PBtn::Edit => {
                self.minimal = false;
                self.mode = Mode::Edit;
                self.relayout();
            }
            PBtn::HideControls => {
                self.hidden = true;
                self.close_menu();
            }
            PBtn::Resume => self.close_menu(),
            PBtn::Game => self.mode = Mode::Game,
            PBtn::Restart | PBtn::Quit => {
                self.close_menu();
                self.emit(if b == PBtn::Restart { Out::Restart } else { Out::Quit });
            }
            PBtn::OptFps => {
                self.settings.show_fps = !self.settings.show_fps;
                self.persist();
            }
            PBtn::OptPicture => {
                self.settings.picture = self.settings.picture.next();
                self.persist();
            }
            PBtn::OptHidePad => {
                self.settings.hide_pad = !self.settings.hide_pad;
                self.persist();
            }
            PBtn::OptLayout => {
                self.settings.stick = !self.settings.stick;
                self.persist();
            }
            PBtn::OptOpacity => {
                let next = self.settings.opacity + 10;
                self.settings.opacity = if next > OPACITY_MAX { OPACITY_MIN } else { next };
                self.persist();
            }
            PBtn::OptHaptics => {
                self.settings.haptics = !self.settings.haptics;
                self.persist();
            }
            PBtn::OptAuto => {
                self.settings.auto_battle = !self.settings.auto_battle;
                self.persist();
            }
            PBtn::OptReset => {
                self.settings.reset_layout();
                self.persist();
            }
            PBtn::Back => self.mode = Mode::Menu,
            PBtn::SizeDown | PBtn::SizeUp => {
                let o = self.geo.orient;
                let g = self.selected;
                let cur = self.settings.place(o, g).scale.unwrap_or(100);
                let next = if b == PBtn::SizeUp { cur + 10 } else { cur.saturating_sub(10) };
                self.set_scale(g, next);
            }
            PBtn::OpDown => {
                self.settings.opacity = self.settings.opacity.saturating_sub(10).max(OPACITY_MIN);
            }
            PBtn::OpUp => {
                self.settings.opacity = (self.settings.opacity + 10).min(OPACITY_MAX);
            }
            PBtn::EditReset => {
                self.settings.reset_layout();
                self.settings.opacity = settings::OPACITY_DEFAULT;
                self.selected = Group::Dpad;
                self.relayout();
            }
            PBtn::Done => {
                self.persist();
                self.close_menu();
            }
        }
    }

    fn close_menu(&mut self) {
        self.armed = None;
        self.mode = Mode::Play;
        self.fingers.clear();
        self.emit(Out::Pause(false));
        self.recompute();
    }

    fn set_scale(&mut self, g: Group, percent: u32) {
        let o = self.geo.orient;
        let p = percent.clamp(SCALE_MIN, SCALE_MAX);
        self.settings.place_mut(o, g).scale = if p == 100 { None } else { Some(p) };
        self.relayout();
    }

    fn drag_to(&mut self, g: Group, centre: Pos2) {
        let s = self.geo.screen;
        let o = self.geo.orient;
        let c = pos2(centre.x.clamp(s.min.x, s.max.x), centre.y.clamp(s.min.y, s.max.y));
        let place = self.settings.place_mut(o, g);
        place.x = Some(((c.x - s.min.x) / s.width() * 1000.0).round() as i32);
        place.y = Some(((c.y - s.min.y) / s.height() * 1000.0).round() as i32);
        self.relayout();
    }

    fn resize_to(&mut self, g: Group, pos: Pos2) {
        let o = self.geo.orient;
        let Some(b) = self.geo.group_bounds(g) else { return };
        let cur = self.settings.place(o, g).scale.unwrap_or(100) as f32;
        let half_diag_now = (b.expand(6.0).size() / 2.0).length();
        let per_percent = half_diag_now / cur;
        let want = pos.distance(b.center()) / per_percent;
        let snapped = ((want / 5.0).round() * 5.0) as u32;
        self.set_scale(g, snapped);
    }

    /// The SNES buttons the fingers hold, worked out again.
    fn recompute(&mut self) {
        let mut p = [false; 12];
        if self.mode == Mode::Play && !self.hidden && self.visible() {
            let g = &self.geo;
            for f in &self.fingers {
                match f.target {
                    Target::Dpad => {
                        let (c, half) = g.dpad;
                        let d = f.pos - c;
                        // Past the cross's reach the finger holds nothing.
                        if d.x.abs().max(d.y.abs()) <= half + 34.0 {
                            let dirs = layout::directions(d, half / 3.0);
                            for (i, on) in dirs.into_iter().enumerate() {
                                p[i] |= on;
                            }
                        }
                    }
                    Target::Stick => {
                        let d = f.pos - f.origin;
                        let dirs = layout::directions(d, 16.0);
                        // Up, down, left, right sit at 0 to 3 of the array.
                        for (i, on) in dirs.into_iter().enumerate() {
                            p[i] |= on;
                        }
                    }
                    Target::Face => {
                        let near = g
                            .items
                            .iter()
                            .filter(|i| i.group == Group::Face)
                            .filter_map(|i| match i.shape {
                                Shape::Circle(c, r) => Some((c.distance(f.pos) / r, i.btn)),
                                _ => None,
                            })
                            .filter(|(k, _)| *k <= 1.6)
                            .min_by(|a, b| a.0.total_cmp(&b.0));
                        if let Some((_, btn)) = near
                            && let Some(i) = slot_of(btn)
                        {
                            p[i] = true;
                        }
                    }
                    Target::Hold(btn) => {
                        if let Some(item) = g.item(btn)
                            && item.shape.hit(f.pos, 24.0)
                            && let Some(i) = slot_of(btn)
                        {
                            p[i] = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        if (0..12).any(|i| p[i] && !self.pressed[i]) {
            self.emit(Out::Haptic);
        }
        self.pressed = p;
    }

    /// Whether a finger holds this button of a panel (for the drawing).
    #[must_use]
    pub fn panel_down(&self, id: PBtn) -> bool {
        self.fingers.iter().any(|f| f.target == Target::Panel(id))
    }

    /// The choices changed elsewhere (the Options screen): the layout is worked out again.
    pub fn apply_settings(&mut self, settings: Settings) {
        if self.settings != settings {
            self.settings = settings;
            self.relayout();
        }
    }

    /// A `touch` line of a UI script: puts the layer in a state a snapshot shows (no real finger is involved).
    ///
    /// # Errors
    ///
    /// The line is not one of the words below.
    pub fn script(&mut self, line: &str) -> Result<(), String> {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["play"] => {
                self.mode = Mode::Play;
                self.hidden = false;
            }
            ["menu"] => self.mode = Mode::Menu,
            ["options"] => self.mode = Mode::Options,
            ["game"] => self.mode = Mode::Game,
            ["edit"] => {
                self.minimal = false;
                self.mode = Mode::Edit;
            }
            ["hidden"] => self.hidden = true,
            ["minimal"] => self.minimal = true,
            ["full"] => self.minimal = false,
            ["stick"] => self.settings.stick = true,
            ["dpad"] => self.settings.stick = false,
            ["gamepad"] => self.gamepad = true,
            ["nogamepad"] => self.gamepad = false,
            ["opacity", percent] => {
                self.settings.opacity = percent
                    .parse::<u32>()
                    .map_err(|_| format!("`{percent}` is not a percentage"))?
                    .clamp(OPACITY_MIN, OPACITY_MAX);
            }
            ["select", name] => {
                self.selected = Group::ALL
                    .into_iter()
                    .find(|g| g.id() == *name)
                    .ok_or_else(|| format!("unknown group `{name}`"))?;
            }
            ["release"] => self.cancel_all(),
            ["press", names] => {
                self.mode = Mode::Play;
                self.relayout();
                for (n, name) in names.split(',').enumerate() {
                    let btn = match name {
                        "up" => Btn::Up,
                        "down" => Btn::Down,
                        "left" => Btn::Left,
                        "right" => Btn::Right,
                        "a" => Btn::A,
                        "b" => Btn::B,
                        "x" => Btn::X,
                        "y" => Btn::Y,
                        "l" => Btn::L,
                        "r" => Btn::R,
                        "start" => Btn::Start,
                        "select" => Btn::Select,
                        "speed" => Btn::Speed,
                        _ => return Err(format!("unknown button `{name}`")),
                    };
                    let at = match self.geo.item(btn) {
                        Some(item) => item.shape.bounds().center(),
                        None => self.geo.stick_zone.map_or(Pos2::ZERO, |z| z.center()),
                    };
                    self.finger_down(900 + n as i64, at);
                }
                return Ok(());
            }
            _ => return Err(format!("unknown touch line `{line}`")),
        }
        self.relayout();
        self.fingers.clear();
        self.recompute();
        Ok(())
    }

    /// A band for the picture: the picture area of the layout, if the controls are on screen.
    #[must_use]
    pub fn picture_area(&self) -> Option<Rect> {
        self.visible().then_some(self.geo.picture)
    }

    /// The orientation of the layout.
    #[must_use]
    pub fn orient(&self) -> Orient {
        self.geo.orient
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch() -> Touch {
        Touch::new(true, Settings::default(), Rect::from_min_size(Pos2::ZERO, vec2(932.0, 430.0)))
    }

    fn centre(t: &Touch, btn: Btn) -> Pos2 {
        t.geometry().item(btn).unwrap().shape.bounds().center()
    }

    #[test]
    fn a_finger_on_a_d_pad_cell_holds_that_direction_and_leaving_releases() {
        let mut t = touch();
        let up = centre(&t, Btn::Up);
        t.finger_down(1, up);
        assert!(t.pressed()[0] && !t.pressed()[1]);
        // Slide to the right cell.
        t.finger_motion(1, centre(&t, Btn::Right));
        let p = t.pressed();
        assert!(p[3] && !p[0]);
        // Far away: nothing held, the finger is still the d-pad's.
        t.finger_motion(1, pos2(500.0, 100.0));
        assert_eq!(t.pressed(), [false; 12]);
        t.finger_motion(1, centre(&t, Btn::Down));
        assert!(t.pressed()[1]);
        t.finger_up(1, false);
        assert_eq!(t.pressed(), [false; 12]);
    }

    #[test]
    fn the_d_pad_and_a_work_together_with_two_fingers() {
        let mut t = touch();
        t.finger_down(1, centre(&t, Btn::Left));
        t.finger_down(2, centre(&t, Btn::A));
        let p = t.pressed();
        assert!(p[2] && p[4], "left and A: {p:?}");
        t.finger_up(2, false);
        assert!(t.pressed()[2] && !t.pressed()[4]);
    }

    #[test]
    fn a_finger_slides_between_face_buttons() {
        let mut t = touch();
        t.finger_down(1, centre(&t, Btn::A));
        assert!(t.pressed()[4]);
        t.finger_motion(1, centre(&t, Btn::B));
        assert!(t.pressed()[5] && !t.pressed()[4]);
    }

    #[test]
    fn menu_pauses_and_resume_lets_go_and_the_speed_button_cycles() {
        let mut t = touch();
        t.finger_down(1, centre(&t, Btn::Speed));
        t.finger_up(1, false);
        assert!(t.take_out().contains(&Out::Speed));
        t.finger_down(1, centre(&t, Btn::Menu));
        t.finger_up(1, false);
        assert_eq!(t.mode, Mode::Menu);
        assert!(t.take_out().contains(&Out::Pause(true)));
        // Play buttons are dead behind the menu.
        t.finger_down(2, centre(&t, Btn::A));
        assert_eq!(t.pressed(), [false; 12]);
        t.finger_up(2, false);
        let resume =
            t.panel().unwrap().buttons.iter().find(|b| b.id == PBtn::Resume).unwrap().rect.center();
        t.finger_down(3, resume);
        t.finger_up(3, false);
        assert_eq!(t.mode, Mode::Play);
        assert!(t.take_out().contains(&Out::Pause(false)));
    }

    #[test]
    fn dragging_in_the_edit_mode_moves_a_group_and_done_asks_to_save() {
        let mut t = touch();
        t.mode = Mode::Edit;
        let a = centre(&t, Btn::A);
        t.finger_down(1, a);
        assert_eq!(t.selected, Group::Face);
        t.finger_motion(1, a - vec2(100.0, 50.0));
        t.finger_up(1, false);
        let moved = t.geometry().group_bounds(Group::Face).unwrap().center();
        let before = layout::compute(t.geometry().screen, false, &Settings::default())
            .group_bounds(Group::Face)
            .unwrap()
            .center();
        assert!(
            (before.x - moved.x - 100.0).abs() < 6.0 && (before.y - moved.y - 50.0).abs() < 6.0
        );
        t.take_out();
        let done =
            t.panel().unwrap().buttons.iter().find(|b| b.id == PBtn::Done).unwrap().rect.center();
        t.finger_down(2, done);
        t.finger_up(2, false);
        let out = t.take_out();
        assert!(out.contains(&Out::Persist) && out.contains(&Out::Pause(false)));
        assert_eq!(t.mode, Mode::Play);
        assert!(t.settings.land[Group::Face.index()].x.is_some());
    }

    #[test]
    fn size_buttons_stay_within_100_and_180_percent() {
        let mut t = touch();
        t.mode = Mode::Edit;
        t.selected = Group::L;
        for _ in 0..12 {
            t.panel_press(PBtn::SizeUp);
        }
        assert_eq!(t.settings.land[Group::L.index()].scale, Some(180));
        for _ in 0..12 {
            t.panel_press(PBtn::SizeDown);
        }
        assert_eq!(t.settings.land[Group::L.index()].scale, None, "100% is the default");
        let b = t.geometry().group_bounds(Group::L).unwrap();
        assert!(b.width() >= MIN_DP && b.height() >= MIN_DP);
    }

    #[test]
    fn the_floating_stick_anchors_under_the_thumb_and_follows_it() {
        let mut t = touch();
        t.settings.stick = true;
        t.relayout();
        let zone = t.geometry().stick_zone.unwrap();
        let at = zone.center();
        t.finger_down(1, at);
        assert_eq!(t.pressed(), [false; 12], "no direction at the origin");
        t.finger_motion(1, at + vec2(40.0, 0.0));
        assert!(t.pressed()[3]);
        t.finger_motion(1, at + vec2(0.0, -40.0));
        assert!(t.pressed()[0] && !t.pressed()[3]);
        assert_eq!(t.stick().unwrap().0, at);
    }

    #[test]
    fn auto_battle_switches_the_layout_and_the_minimal_one_has_only_the_battle_controls() {
        let mut t = touch();
        t.settings.auto_battle = true;
        t.set_status("x1", 1, false, true);
        assert!(t.minimal);
        let btns: Vec<Btn> = t.geometry().items.iter().map(|i| i.btn).collect();
        assert!(btns.contains(&Btn::A) && btns.contains(&Btn::B) && btns.contains(&Btn::Full));
        assert!(!btns.contains(&Btn::X) && !btns.contains(&Btn::L));
        t.set_status("x1", 1, false, false);
        assert!(!t.minimal);
    }

    #[test]
    fn the_game_page_asks_a_second_tap_before_it_restarts_or_quits() {
        let mut t = touch();
        let tap = |t: &mut Touch, id: PBtn| {
            let at = t.panel().unwrap().buttons.iter().find(|b| b.id == id).unwrap().rect.center();
            t.finger_down(7, at);
            t.finger_up(7, false);
        };
        t.back_key();
        tap(&mut t, PBtn::Game);
        assert_eq!(t.mode, Mode::Game);
        t.take_out();
        tap(&mut t, PBtn::Quit);
        assert_eq!(t.mode, Mode::Game, "one tap only arms it");
        assert!(!t.take_out().contains(&Out::Quit));
        tap(&mut t, PBtn::Restart);
        tap(&mut t, PBtn::Quit);
        assert!(!t.take_out().contains(&Out::Quit), "another button disarmed it");
        tap(&mut t, PBtn::Quit);
        assert!(t.take_out().contains(&Out::Quit));
        assert_eq!(t.mode, Mode::Play);
        t.back_key();
        tap(&mut t, PBtn::Game);
        t.back_key();
        assert_eq!(t.mode, Mode::Menu, "back leaves the page for the MENU");
    }

    #[test]
    fn a_gamepad_hides_the_controls_unless_the_choice_is_off() {
        let mut t = touch();
        t.set_status("x1", 1, true, false);
        assert!(!t.visible());
        t.finger_down(1, centre(&t, Btn::A));
        assert_eq!(t.pressed(), [false; 12]);
        t.settings.hide_pad = false;
        assert!(t.visible());
    }
}
