//! The HD menu driver: the curtain, the HD cursors, the bag, and the plans that carry out what the
//! player chose. It is what the windowed app and the headless tests drive, one frame at a time:
//! [`Driver::key`] gives the HD a key, [`Driver::frame`] says which joypad the next frame runs with and
//! which host events come before it, and [`Driver::scene`] says what the HD draws.
//!
//! The curtain is up (`Up`) while the HD draws the menu, and then the keys belong to the HD and the
//! game gets no buttons. The curtain lifts (`Lifted`) when the original draws the next thing itself
//! (a message after Talk or Look, Fly or Land, Status, Order, Text, Save), and the game gets the
//! keys again; it comes down (`Down`) once the field is free. The original's menu sits on its first
//! screen while the HD cursor moves: a choice is a plan of presses from the original's own cursor.
//! A use or a drop of the bag stages its stack in the original's list and then presses the original's
//! own keys, one unit at a time, so the original takes the unit and the bag follows the list.

use super::cards;
use super::keeper::{BIND_TAG, Keeper, SPAWN_TAG, good_checksum, label_now};
use super::order;
use super::presses::{Button, Next, Outcome, Player, Step, Until};
use super::{MenuProbe, PORUNGA, SAVE_DONE_STEP, SAVE_LIST_STEP, SAVE_SCREEN, status};
use super::{save, text};
use crate::cheats::{CHEATS_TAG, block_events, frame_events, hand_events, settings_events};
use crate::data as d;
use crate::gameplay::{self, AutoRun};
use crate::mods::state::{MODS_TAG, apply_events, capture, clear_events};
use ssd_menus::bag::{Bag, Rules, SavedBag, Sidecar};
use ssd_menus::cheats::{CHARACTERS, CheatSidecar, Cheats, SavedCheats};
use ssd_menus::mods_state::{ModState, ModsSidecar};
use ssd_menus::nav::{
    Action, Entry, ItemRow, Key, ModsLine, Nav, ORIGINAL_MENU_ROWS, SPAWNER_OBJECTS, SPAWNER_PAGE,
    Spawner, command_rows,
};
use ssd_menus::options::{
    Category, Effect, Opens, Options, SlotOptions, Values, controls, difficulty, video,
};
use ssd_menus::packs::{Catalog, Listed, PackInfo, Request};
use ssd_menus::screens::OrderNav;
use ssd_menus::view::{
    BagView, ChapterLine, CheatsView, ControlsView, Labels, MenuView, ModsView, ObjectLabel,
    OptionsView, PackLine, PackState, RefusedKey, Screen, Selected, SpawnerRow, SpawnerView,
    StackView,
};
use ssd_menus::{DrawList, screens};
use ssd_oracle::{HostEvent, Joypad};

/// Where the curtain is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curtain {
    /// The field: no menu is open.
    Down,
    /// The HD draws the menu, and the keys go to it.
    Up,
    /// The original draws the menu or a message, and the keys go to the game.
    Lifted,
}

/// Where the curtain goes when a plan has reached its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrival {
    /// The HD shows this screen.
    Show(Screen),
    /// The HD shows this screen again, with the cursors it had (the Menu list after Status).
    Resume(Screen),
    /// The original draws what comes next.
    Lifted,
    /// The menu has closed.
    Closed,
    /// The original saved to this slot (0 to 2) and shows its "Completed!": the HD says so and waits for A.
    Saved(u8),
}

/// A plan running, and where it leaves the curtain.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Running {
    player: Player,
    arrival: Arrival,
}

/// How the bag is bound: by the driver itself, from the save and the sidecar, or by a replay's
/// recorded bind event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// The driver binds the bag when the game has a save or a new game is in the field.
    Auto,
    /// The bag is bound only by the replay's recorded bind (see [`Driver::replay_event`]).
    Recorded,
}

/// What a frame of the menu needs from the host: the joypad of the frame, the host events to make
/// before it, the bag of a save that changed (to be kept in the sidecar file), and the cheats and the mod state
/// of the save a Save wrote (kept in `saves/ssd.cheats` and `saves/ssd.mods`).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub pad: Joypad,
    pub events: Vec<HostEvent>,
    pub saved: Option<SavedBag>,
    pub saved_cheats: Option<SavedCheats>,
    /// The slot and checksum word of the save a Save wrote, for its mod state's file (`saves/ssd.mods`). Every save
    /// reports it, as it reports the cheats, so a save without a mod state drops the entry it had.
    pub saved_mods: Option<(u8, u16)>,
    /// The options of the save slot a Save wrote (kept in `saves/ssd.options`): the file is to be written.
    pub saved_options: bool,
    /// The options kept for this PC, when one changed this frame (kept in `saves/ssd.settings`).
    pub settings: Option<Values>,
    /// The video options to apply now (they changed this frame, or a question about them was answered).
    pub video: Option<Values>,
}

/// Context the menu needs of the game at each frame: its labels and its rules.
/// Work RAM values that the probe reads are read fresh each time they are needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Context {
    /// The party flies: the command window has Land.
    flying: bool,
    /// The party is in a town: the command window has no Fly.
    town: bool,
}

/// The state of the HD menu: everything but the labels, which the probe reads alongside it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    curtain: Curtain,
    nav: Nav,
    running: Option<Running>,
    last: Option<Outcome>,
    keeper: Keeper,
    sidecar: Sidecar,
    binding: Binding,
    /// The line the bag's screen shows at its foot, until the next key.
    notice: Option<&'static str>,
    /// The cheats the run has: the choices of the Cheats screen, which the cheat block carries.
    cheats: Cheats,
    /// The cheats of the save slots (`saves/ssd.cheats`).
    cheat_sidecar: CheatSidecar,
    /// Whether the cheat wrappers are installed in this run: the cheat block's frame events are made then.
    cheats_installed: bool,
    /// The save the cheats are bound to (the bag's save), so that a new save binds them again.
    cheat_save: Option<(u8, u16)>,
    /// The host events of the cheats' changes, made before the next frame.
    cheat_events: Vec<HostEvent>,
    /// The mod state of the save slots (`saves/ssd.mods`): the extended flags and the high parts of the wide BP.
    mods_sidecar: ModsSidecar,
    /// The save the mod state is bound to (the bag's save), so that a new save binds it again.
    mods_save: Option<(u8, u16)>,
    /// The slot the Save list's cursor is on, kept from the list's step until the Save completes.
    save_slot: Option<u8>,
    /// Whether the Save was at its completion step at the last frame (a completion is seen once).
    save_done: bool,
    /// The three save slots as the cartridge had them at the last frame, while the HD Save screen is up.
    save_slots: [ssd_menus::screens::SlotState; 3],
    /// The slots saved with a cheat on, as of the same frame.
    save_marks: [bool; 3],
    /// The Cheats screen is open over a battle ([`Driver::open_battle_cheats`]): Back closes it, and a change
    /// writes the settings only, so the runtime bytes the wrappers keep through the battle stay.
    battle_cheats: bool,
    /// The Item list of a battle is open and the HD's Items screen is over it: Back goes to the battle's command
    /// window, and the rows are the stacks a battle can use ([`usable_in_battle`]).
    battle_items: bool,
    /// The options of the run: the values saved with the game (the bound save's) and the ones of this PC.
    options: Options,
    /// The options of the save slots (`saves/ssd.options`).
    option_slots: SlotOptions,
    /// The PC options changed: the frame hands them to the host to be written.
    settings_changed: bool,
    /// A button of the Game page asked for the setup: the host takes it, and restarts the game into it.
    setup_request: Option<Opens>,
    /// While "Apply video changes?" asks: the PC options as they were before the change, which a Revert restores.
    video_backup: Option<Values>,
    /// The frames left before the question reverts the change.
    video_frames: u32,
    /// The video options changed: the frame hands them to the host to be applied.
    video_apply: bool,
    /// The message auto-run: the wait of a finished message and the press (the Gameplay page).
    auto_run: AutoRun,
    /// The data packs the Data packs screen lists, as the run found them (kept across a state load).
    packs: Catalog,
    /// The list as the player leaves it: the packs in their order, and whether each is on. Made from `packs` when the
    /// screen opens.
    packs_edit: Vec<Listed>,
    /// A line the Data packs screen shows until the next key: why the host could not do what the screen asked.
    mods_alert: Option<String>,
    /// The request the Data packs screen made, once: the host applies the set, or starts a chapter.
    mods_request: Option<Request>,
    /// The Order screen's cursor and the order it holds, until "Apply order" has the original take it.
    order: OrderNav,
}

/// The frames a second the video question counts its ten seconds in (the console's rate, which the host paces to).
const VIDEO_FPS: u32 = 60;
/// The line of the Data packs screen when a chapter or Main game is chosen while the list differs from the set in use.
const APPLY_FIRST: &str = "Apply the packs first: the game runs with another set.";
/// The line of the Data packs screen when Apply is chosen for the set the game already runs with.
const UNCHANGED: &str = "The game already runs with this set.";

/// The HD menu of a run: the curtain, the HD cursors, the bag and the plan that is running, if one is.
pub struct Driver {
    labels: Labels,
    /// The rules the bag is kept with (kept for [`Driver::reset`]).
    rules: Rules,
    state: State,
}

impl Driver {
    /// A driver for the menus with the names of `labels`, with the field free and the bag unbound.
    #[must_use]
    pub fn new(labels: Labels) -> Self {
        Self::with_bag(labels, Rules::default(), Sidecar::empty())
    }

    /// A driver whose bag has `rules` and binds with the sidecar `sidecar` (the bags of the saves).
    #[must_use]
    pub fn with_bag(labels: Labels, rules: Rules, sidecar: Sidecar) -> Self {
        Self {
            labels,
            rules,
            state: State {
                curtain: Curtain::Down,
                nav: Nav::default(),
                running: None,
                last: None,
                keeper: Keeper::new(rules),
                sidecar,
                binding: Binding::Auto,
                notice: None,
                cheats: Cheats::neutral(),
                cheat_sidecar: CheatSidecar::empty(),
                cheats_installed: false,
                cheat_save: None,
                cheat_events: Vec::new(),
                mods_sidecar: ModsSidecar::empty(),
                mods_save: None,
                save_slot: None,
                save_done: false,
                save_slots: std::array::from_fn(|_| screens::SlotState::Used(None)),
                save_marks: [false; 3],
                battle_cheats: false,
                battle_items: false,
                options: Options::default(),
                option_slots: SlotOptions::empty(),
                settings_changed: false,
                setup_request: None,
                video_backup: None,
                video_frames: 0,
                video_apply: false,
                auto_run: AutoRun::default(),
                packs: Catalog::default(),
                packs_edit: Vec::new(),
                mods_alert: None,
                mods_request: None,
                order: OrderNav::default(),
            },
        }
    }

    /// The run installs the cheats (its wrappers are in the machine): the cheat block's frame events are
    /// made, and the cheats of the save slots are the ones of `sidecar` (`saves/ssd.cheats`).
    #[must_use]
    pub fn with_cheats(mut self, sidecar: CheatSidecar) -> Self {
        self.state.cheats_installed = true;
        self.state.cheat_sidecar = sidecar;
        self
    }

    /// The run keeps the mod state of the save slots of `sidecar` (`saves/ssd.mods`), bound to the bag's save as the
    /// cheats are.
    #[must_use]
    pub fn with_mods(mut self, sidecar: ModsSidecar) -> Self {
        self.state.mods_sidecar = sidecar;
        self
    }

    /// The run keeps the options of the save slots of `sidecar` (`saves/ssd.options`) and the ones of this PC
    /// (`saves/ssd.settings`).
    #[must_use]
    pub fn with_options(mut self, sidecar: SlotOptions, pc: Values) -> Self {
        self.state.option_slots = sidecar;
        self.state.options.pc = pc;
        self
    }

    /// The names and texts the menus show from now on (those of another language): only the words may differ, the
    /// ids and flags of `labels` are those of the labels the driver was made with.
    pub fn set_labels(&mut self, labels: Labels) {
        self.labels = labels;
    }

    /// The options of the run.
    #[must_use]
    pub fn options(&self) -> &Options {
        &self.state.options
    }

    /// The setup a button of the Game page asked for, once (the host restarts the game into it).
    pub fn take_setup_request(&mut self) -> Option<Opens> {
        self.state.setup_request.take()
    }

    /// The data packs the Data packs screen lists (see [`Catalog`]): the host gives them when the run starts.
    #[must_use]
    pub fn with_packs(mut self, packs: Catalog) -> Self {
        self.state.packs = packs;
        self
    }

    /// The request the Data packs screen made, once: the host applies the set, starts a chapter or goes back to the main
    /// game (see [`Request`]).
    pub fn take_mods_request(&mut self) -> Option<Request> {
        self.state.mods_request.take()
    }

    /// Whether Y is held this frame: on the Data packs screen Up and Down then move the pack under the cursor.
    pub fn set_moving(&mut self, held: bool) {
        self.state.nav.set_mods_moving(held);
    }

    /// A line the Data packs screen shows until the next key: why the host could not do what the screen asked.
    pub fn set_mods_alert(&mut self, text: String) {
        self.state.mods_alert = Some(text);
    }

    /// What a save state of the window keeps of the HD menus beside the machine's state, which does not hold it: the bag,
    /// the cheats and the save they belong to. `None` while no bag is bound. [`Driver::restore_session`] reads it back.
    #[must_use]
    pub fn session_bytes(&self) -> Option<Vec<u8>> {
        let state = &self.state;
        let bag = state.keeper.bag()?;
        let mut out = SESSION_MAGIC.to_vec();
        out.push(SESSION_VERSION);
        match state.keeper.bound_save() {
            Some((slot, checksum)) => {
                out.extend([1, slot]);
                out.extend(checksum.to_le_bytes());
            }
            None => out.extend([0; 4]),
        }
        for part in [bag.to_bytes(), state.cheats.to_bytes()] {
            out.extend(u32::try_from(part.len()).ok()?.to_le_bytes());
            out.extend(part);
        }
        Some(out)
    }

    /// Restores what [`Driver::session_bytes`] kept, after the machine's state is loaded and the driver reset: the bag is
    /// bound again as it was, to the same save, with the cheats of that moment and the game's options of that save. The
    /// events write the list and the cheat block into work RAM. Without it a state loaded where no save matches the game
    /// (a battle, say) had no bag until the field, and the cheats went back to the save's.
    ///
    /// # Errors
    ///
    /// The bytes are not a session of this version, or its bag or cheats cannot be read.
    pub fn restore_session(&mut self, bytes: &[u8], wram: &[u8]) -> Result<Vec<HostEvent>, String> {
        let not = || "not a session of the HD menus".to_owned();
        let body = bytes.strip_prefix(SESSION_MAGIC.as_slice()).ok_or_else(not)?;
        let (&version, body) = body.split_first().ok_or_else(not)?;
        if version != SESSION_VERSION {
            return Err(format!("a session of version {version}, not {SESSION_VERSION}"));
        }
        let (head, mut rest) = body.split_at_checked(4).ok_or_else(not)?;
        let save = (head[0] == 1).then(|| (head[1], u16::from_le_bytes([head[2], head[3]])));
        let mut parts = Vec::new();
        for _ in 0..2 {
            let (len, tail) = rest.split_at_checked(4).ok_or_else(not)?;
            let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
            let (part, tail) = tail.split_at_checked(len).ok_or_else(not)?;
            parts.push(part);
            rest = tail;
        }
        let bag = Bag::from_bytes(parts[0])?;
        let cheats = Cheats::from_bytes(parts[1])?;
        let state = &mut self.state;
        let mut events = state.keeper.restore(bag, save, wram, &self.labels);
        // The cheats and the mod state are the save's already: no bind of them follows on the next frame.
        state.cheat_save = state.keeper.bound_save();
        state.mods_save = state.cheat_save;
        state.options.game = state
            .cheat_save
            .and_then(|(slot, checksum)| state.option_slots.entry_for(slot, checksum))
            .cloned()
            .unwrap_or_default();
        state.cheats = cheats;
        difficulty::restore(&state.options.game, &mut state.cheats);
        events.extend(block_events(&state.cheats));
        events.extend(hand_events(&state.cheats, wram));
        events.push(state.cheats_app_event());
        Ok(events)
    }

    /// A pack the host installed while the run goes on ("Install a pack..."): it joins the list, off, or takes the place of
    /// the pack with its id (a new version; its line keeps its place and whether it is on).
    pub fn add_pack(&mut self, info: PackInfo) {
        let state = &mut self.state;
        if let Some(known) = state.packs.packs.iter_mut().find(|known| known.id == info.id) {
            *known = PackInfo { on: known.on, ..info };
            return;
        }
        state.packs.packs.push(PackInfo { on: false, ..info });
        state.packs_edit.push(Listed { pack: state.packs.packs.len() - 1, on: false });
    }

    /// The options of this PC, to change from outside the menus: the touch controls write their layout here, so the
    /// next time the Options screen saves the file it keeps them.
    pub fn pc_options_mut(&mut self) -> &mut Values {
        &mut self.state.options.pc
    }

    /// The machine frames to run for one frame of the screen now (the Gameplay page's battle animation speed in the
    /// attack animation, else 1): see [`gameplay::animation_factor`]. `wram` is the work RAM after the last frame.
    #[must_use]
    pub fn animation_factor(&self, wram: &[u8]) -> u32 {
        gameplay::animation_factor(wram, &self.state.options.game)
    }

    /// The options of the save slots, as the slot file keeps them.
    #[must_use]
    pub fn option_slots(&self) -> &SlotOptions {
        &self.state.option_slots
    }

    /// The machine's state was replaced under the menu (a state loaded, not a frame run): the curtain
    /// comes down, no plan runs, and the bag binds again to what the game now holds (the sidecar's
    /// bag of the save it matches, or a rebuild from the list). The sidecars, the rules, the labels
    /// and the binding mode are kept; the cheats bind again with the bag.
    pub fn reset(&mut self) {
        // A video change still being asked about is reverted: the question goes with the state.
        let mut pc = self.state.options.pc.clone();
        let reverted = self
            .state
            .video_backup
            .take()
            .inspect(|before| video::restore(&mut pc, before))
            .is_some();
        self.state = State {
            curtain: Curtain::Down,
            nav: Nav::default(),
            running: None,
            last: None,
            keeper: Keeper::new(self.rules),
            sidecar: self.state.sidecar.clone(),
            binding: self.state.binding,
            notice: None,
            cheats: Cheats::neutral(),
            cheat_sidecar: self.state.cheat_sidecar.clone(),
            cheats_installed: self.state.cheats_installed,
            cheat_save: None,
            cheat_events: Vec::new(),
            mods_sidecar: self.state.mods_sidecar.clone(),
            mods_save: None,
            save_slot: None,
            save_done: false,
            save_slots: std::array::from_fn(|_| screens::SlotState::Used(None)),
            save_marks: [false; 3],
            battle_cheats: false,
            battle_items: false,
            options: Options { game: Values::new(), pc },
            option_slots: self.state.option_slots.clone(),
            settings_changed: reverted,
            setup_request: None,
            video_backup: None,
            video_frames: 0,
            video_apply: reverted,
            auto_run: AutoRun::default(),
            packs: self.state.packs.clone(),
            packs_edit: Vec::new(),
            mods_alert: None,
            mods_request: None,
            order: OrderNav::default(),
        };
    }

    /// The cheats of the run: the choices the Cheats screen shows.
    #[must_use]
    pub fn cheats(&self) -> &Cheats {
        &self.state.cheats
    }

    /// The cheats of the save slots, as the sidecar file keeps them.
    #[must_use]
    pub fn cheat_sidecar(&self) -> &CheatSidecar {
        &self.state.cheat_sidecar
    }

    /// The mod state of the save slots, as the sidecar file `saves/ssd.mods` keeps it.
    #[must_use]
    pub fn mods_sidecar(&self) -> &ModsSidecar {
        &self.state.mods_sidecar
    }

    /// Which save slots carry the CHEATS badge: a good save whose cheats were saved with one on, by its
    /// checksum word (a slot changed by another way has no entry).
    #[must_use]
    pub fn marked_slots(&self, sram: &[u8]) -> [bool; 3] {
        self.state.marked_slots(sram)
    }

    /// How the bag is bound (see [`Binding`]).
    pub fn set_binding(&mut self, binding: Binding) {
        self.state.binding = binding;
    }

    /// The sidecar: the bags of the saves this run knows.
    #[must_use]
    pub fn sidecar(&self) -> &Sidecar {
        &self.state.sidecar
    }

    /// The four sidecar files of the save slots, to change from outside the game: the save editor re-keys the entries of a slot
    /// it wrote ([`ssd_menus::saveedit::Commit::rekey`]).
    pub fn sidecars_mut(&mut self) -> ssd_menus::saveedit::Sidecars<'_> {
        ssd_menus::saveedit::Sidecars {
            bags: &mut self.state.sidecar,
            cheats: &mut self.state.cheat_sidecar,
            mods: &mut self.state.mods_sidecar,
            options: &mut self.state.option_slots,
        }
    }

    /// The bag, once it is bound.
    #[must_use]
    pub fn bag(&self) -> Option<&Bag> {
        self.state.keeper.bag()
    }

    /// Where the curtain is.
    #[must_use]
    pub fn curtain(&self) -> Curtain {
        self.state.curtain
    }

    /// Opens the Cheats screen over a battle (the host asks for it only while the game is in one): the
    /// curtain comes up on the screen, the game gets no buttons until Back closes it. Refused while a menu or a
    /// plan has the curtain, or when the run has no cheats.
    pub fn open_battle_cheats(&mut self) -> bool {
        let state = &mut self.state;
        if state.curtain != Curtain::Down || state.running.is_some() || !state.cheats_installed {
            return false;
        }
        state.nav = Nav::default();
        state.nav.enter(Screen::Cheats);
        state.curtain = Curtain::Up;
        state.battle_cheats = true;
        true
    }

    /// Whether the Cheats screen is open over a battle.
    #[must_use]
    pub fn battle_cheats(&self) -> bool {
        self.state.battle_cheats
    }

    /// The HD cursors.
    #[must_use]
    pub fn nav(&self) -> Nav {
        self.state.nav
    }

    /// The Order screen's cursor and the order it holds.
    #[must_use]
    pub fn order(&self) -> &OrderNav {
        &self.state.order
    }

    /// Whether a plan is running: the menu is busy with a choice, and the keys wait.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.state.running.is_some()
    }

    /// What the last plan that ended reached, if one has ended.
    #[must_use]
    pub fn last_outcome(&self) -> Option<Outcome> {
        self.state.last
    }

    /// The line the bag's screen shows at its foot, if there is one.
    #[must_use]
    pub fn notice(&self) -> Option<&'static str> {
        self.state.notice
    }

    /// The player opens the menu on the field: a press of A, as a plan. Nothing happens unless the
    /// field is free and no plan runs.
    pub fn open(&mut self) {
        self.state.open();
    }

    /// A key of the HD on the menu of `wram`: the cursor moves, or a choice starts its plan. Ignored
    /// while the curtain is not up, or while a plan runs.
    pub fn key(&mut self, key: Key, wram: &[u8]) {
        let probe = MenuProbe::new(wram, &self.labels);
        self.state.key(key, &probe, &self.labels, wram);
    }

    /// Start on the HD's Cards or Status screen closes the menu: on the Cards, the original's own A, once it has dealt the
    /// hand; on the Status, its own B presses back out of the page; on the Save list (nothing asked, nothing saved) the
    /// original's own B presses, and nothing is saved. Ignored while the curtain is not up, while a plan runs,
    /// and on the other screens.
    pub fn close_menu(&mut self) {
        self.state.close_menu();
    }

    /// Whether the Controls page waits for the next key or button, and for which device: the host then gives
    /// it the raw press through [`Driver::capture`] and keeps it from the menu's keys.
    #[must_use]
    pub fn listening(&self) -> Option<controls::Device> {
        (self.state.curtain == Curtain::Up && self.state.nav.options_listening())
            .then(|| self.state.nav.controls_device())
    }

    /// A raw key or button pressed while the Controls page waits for one: see [`Driver::listening`].
    pub fn capture(&mut self, capture: controls::Capture) {
        self.state.capture(capture);
    }

    /// The frame of the menu on the game of `wram` (its work RAM after the frame before it, and the
    /// battery save `sram`): the bag is bound and kept in step with the list, a plan's next press is
    /// chosen, and the live joypad is given to the game when the curtain is not up.
    pub fn frame(&mut self, wram: &[u8], sram: Option<&[u8]>, live: Joypad) -> Frame {
        let probe = MenuProbe::new(wram, &self.labels);
        self.state.frame(&probe, &self.labels, wram, sram, live)
    }

    /// A replay's recorded application event: the bag's bind, applied as it was recorded, the run's cheats
    /// ([`CHEATS_TAG`]) for the bookkeeping of the saves, or the writes that put the run's mod state into work RAM
    /// ([`MODS_TAG`]). The sidecar files are not written by a replay (see [`Frame`]), so only the run's state changes
    /// here. Other tags are not the menus'.
    ///
    /// # Errors
    ///
    /// The event's payload is not a bag, a cheats file or a mod state.
    pub fn replay_event(
        &mut self,
        tag: u8,
        payload: &[u8],
        wram: &[u8],
    ) -> Result<Vec<HostEvent>, String> {
        match tag {
            BIND_TAG => self.state.keeper.bind_recorded(payload, wram, &self.labels),
            CHEATS_TAG => {
                self.state.cheats = Cheats::from_bytes(payload)?;
                Ok(Vec::new())
            }
            MODS_TAG => Ok(apply_events(&ModState::from_bytes(payload)?)),
            SPAWN_TAG => {
                let &[object, count] = payload else {
                    return Err(format!("a spawn event of {} bytes", payload.len()));
                };
                Ok(self.state.keeper.spawn(wram, &self.labels, object, count).1)
            }
            _ => Ok(Vec::new()),
        }
    }

    /// The bag's Item screen shows `tab` from its first stack (the shoulder buttons do the same).
    pub fn set_tab(&mut self, tab: ssd_menus::view::Tab) {
        self.state.nav.set_tab(tab);
    }

    /// The units of the stack the shop's Sell list has its cursor on, while the list takes presses (`None` anywhere
    /// else, or with no bag). The original draws the list one row a stack, with no count; the host draws this one
    /// beside the price box of the object under the cursor.
    #[must_use]
    pub fn sell_count(&self, wram: &[u8]) -> Option<u16> {
        // The shop is the original's own screen: the curtain is lifted for it (Talk), and never up.
        if self.state.curtain == Curtain::Up {
            return None;
        }
        let row = MenuProbe::new(wram, &self.labels).sell_row()?;
        self.state.keeper.count_at(wram, row)
    }

    /// The screen the HD shows while the curtain is up (`None`: the game's own picture).
    #[must_use]
    pub fn screen(&self) -> Option<Screen> {
        (self.state.curtain == Curtain::Up).then(|| self.state.nav.screen())
    }

    /// What the HD draws now: the screen the HD shows, with its cursors, while the curtain is up.
    #[must_use]
    pub fn scene(&self, wram: &[u8]) -> Option<DrawList> {
        if self.state.curtain != Curtain::Up {
            return None;
        }
        let probe = MenuProbe::new(wram, &self.labels);
        self.state.scene(&probe, &self.labels)
    }
}

impl State {
    /// The player's own press opened the menu: see [`Driver::open`].
    fn open(&mut self) {
        if self.curtain != Curtain::Down || self.running.is_some() {
            return;
        }
        // The command window is up once the menu is drawn: its screen (13 in a town) is set a
        // couple of frames after the press.
        let steps = vec![
            Step::Press { button: Button::A, until: Until::Open },
            Step::Wait { until: Until::Ready },
            Step::Pause { frames: SETTLE_WINDOW },
        ];
        self.start(steps, Arrival::Show(Screen::Command));
    }

    /// See [`Driver::close_menu`].
    fn close_menu(&mut self) {
        if self.curtain != Curtain::Up || self.running.is_some() {
            return;
        }
        match self.nav.screen() {
            Screen::Cards => self.start(cards::close_steps(), Arrival::Closed),
            Screen::Status => self.start(status::close_steps(), Arrival::Closed),
            // The Save list with nothing asked and nothing saved: the original's own B back to its command window and B
            // again close the menu, and nothing is saved.
            Screen::Save if self.nav.save().is_browsing() => {
                self.start(save::leave_steps(), Arrival::Closed);
            }
            _ => {}
        }
    }

    /// The Controls page was waiting for a key or button and got `capture`: Esc stops, Backspace clears a
    /// shortcut, a code is taken for the action under the cursor unless another action already holds it.
    fn capture(&mut self, capture: controls::Capture) {
        if self.curtain != Curtain::Up || !self.nav.options_listening() {
            return;
        }
        let device = self.nav.controls_device();
        let Some(action) = self.nav.controls_action() else {
            self.nav.stop_listening();
            return;
        };
        match capture {
            controls::Capture::Cancel => self.nav.stop_listening(),
            controls::Capture::Clear => {
                if controls::clear(&mut self.options.pc, device, action) {
                    self.settings_changed = true;
                    self.nav.stop_listening();
                }
            }
            controls::Capture::Code(code) => {
                match controls::assign(&mut self.options.pc, device, action, code) {
                    Ok(()) => {
                        self.settings_changed = true;
                        self.nav.stop_listening();
                    }
                    Err(by) => self.nav.refuse(by),
                }
            }
        }
    }

    /// The device the Controls page shows is the stored one.
    fn sync_device(&mut self) {
        self.nav.set_controls_page(controls::page(&self.options.pc));
    }

    /// A key of the HD: see [`Driver::key`]. The Controls page shows the stored device before and after it.
    fn key(&mut self, key: Key, probe: &MenuProbe<'_>, labels: &Labels, wram: &[u8]) {
        self.sync_device();
        self.key_screen(key, probe, labels, wram);
        self.sync_device();
    }

    /// A key of the HD: see [`Driver::key`].
    fn key_screen(&mut self, key: Key, probe: &MenuProbe<'_>, labels: &Labels, wram: &[u8]) {
        if self.curtain != Curtain::Up || self.running.is_some() {
            return;
        }
        self.notice = None;
        self.mods_alert = None;
        let screen = self.nav.screen();
        let Context { flying, town } = context(probe);
        let action = match screen {
            Screen::Items => {
                let rows = self.rows(labels, probe.encounter_group());
                self.nav.key_bag(key, &rows)
            }
            Screen::Cheats if self.nav.spawner().is_some() => {
                self.nav.key_spawner(key, &|object| Keeper::story(labels, object))
            }
            Screen::Mods => {
                let lines = self.mods_lines();
                self.nav.key_mods(key, &lines)
            }
            Screen::Order => self.order.key(key),
            Screen::Status => self.nav.key_status(key, probe.status_count()),
            _ => self.nav.key(key, flying, town, probe.item_count()),
        };
        match action {
            Action::None => {}
            Action::Notice(text) => self.notice = Some(text),
            // The Cheats screen is the HD's own: nothing is sent to the original, which keeps its Menu list.
            Action::Choose(Entry::Cheats) => self.nav.enter(Screen::Cheats),
            // The Options screen is the HD's own too: the original keeps its Menu list.
            Action::Choose(Entry::Options) => self.nav.enter(Screen::Options),
            // The Text screen is the HD's own too: it holds the speed until A, then the original's Text screen takes it.
            Action::Choose(Entry::Text) => self.nav.open_text(probe.text_speed()),
            Action::TextKeep(speed) => self.start(text::keep_steps(speed), Arrival::Closed),
            // The Save screen is the HD's own: the original keeps its Menu list until a slot is chosen, and then its own
            // Save takes the presses (it writes the slot).
            Action::Choose(Entry::Save) => self.nav.open_save(),
            Action::SaveSlot(slot) => {
                self.nav.save_saving(slot);
                self.start(save::save_steps(slot), Arrival::Saved(slot));
            }
            Action::SaveClose => self.start(save::close_steps(), Arrival::Closed),
            // The Order screen is the HD's own as well: it holds the order until Apply, then the original's own Order
            // screen takes it pick by pick. A party the original's records do not describe keeps the original's screen.
            Action::Choose(Entry::Order) if probe.order_rows().is_some() => {
                if let Some(rows) = probe.order_rows() {
                    self.nav.enter(Screen::Order);
                    self.order.open(rows);
                }
            }
            Action::OrderApply => {
                self.start(order::apply_steps(self.order.order()), Arrival::Closed);
            }
            Action::Choose(entry) => {
                if let Some((steps, arrival)) = choice(screen, entry, flying, town) {
                    self.start(steps, arrival);
                }
            }
            Action::Adjust { row, by } => {
                row.step(&mut self.cheats, by);
                self.cheats_changed(wram);
            }
            Action::OptionChange { category, row, by } => {
                let speed = probe.text_speed();
                let before = (category == Category::Video).then(|| self.options.pc.clone());
                let effect = match category.rows().get(row) {
                    Some(def) => {
                        let State { options, cheats, .. } = self;
                        def.step(options.values_mut(category), cheats, speed, by)
                    }
                    None => Effect::None,
                };
                self.option_effect(category, effect, wram);
                if let Some(before) = before {
                    self.video_changed(&before);
                }
            }
            Action::OptionsResetPage(category) => self.reset_options(&[category], probe, wram),
            Action::OptionsResetAll => self.reset_options(Category::shown(), probe, wram),
            Action::OptionsOpen(Opens::Cheats) => self.nav.enter_cheats_from_options(),
            Action::OptionsOpen(Opens::ResetTouchLayout) => {
                controls::reset_touch_layout(&mut self.options.pc);
                self.settings_changed = true;
            }
            Action::OptionsOpen(opens @ (Opens::Mode | Opens::Pack)) => {
                self.setup_request = Some(opens);
            }
            Action::OptionsVideo { keep } => self.video_answer(keep),
            Action::OptionsOpen(Opens::Packs) => self.open_packs(),
            Action::PackToggle(at) => {
                if let Some(listed) = self.packs_edit.get_mut(at) {
                    listed.on = !listed.on;
                }
            }
            Action::PackMove { at, by } => {
                if let Some(to) = at.checked_add_signed(isize::from(by))
                    && to < self.packs_edit.len()
                {
                    self.packs_edit.swap(at, to);
                }
            }
            Action::PacksApply => self.apply_packs(),
            Action::PacksInstall => self.mods_request = Some(Request::Install),
            Action::ChapterStart(line) => self.start_chapter(line),
            Action::ChapterMain => self.main_game(),
            Action::ResetAll => {
                // The spawner's mark is history, not a choice: Reset all keeps it.
                let spawned = self.cheats.spawned();
                self.cheats = Cheats::neutral();
                self.cheats.set_spawned(spawned);
                self.cheats_changed(wram);
            }
            Action::Spawn { object, count } => {
                let (added, writes) = self.keeper.spawn(wram, labels, object, count);
                if added > 0 {
                    self.cheat_events.extend(writes);
                    self.cheat_events
                        .push(HostEvent::App { tag: SPAWN_TAG, payload: vec![object, added] });
                    if !self.cheats.spawned() {
                        self.cheats.set_spawned(true);
                        self.cheats_changed(wram);
                    }
                }
            }
            // What the use asks after its Use is the player's, on the original's screens.
            Action::Use { stack } => self.start(use_steps(stack), Arrival::Lifted),
            Action::Drop { stack } => {
                let steps = drop_steps(stack);
                self.start(steps, Arrival::Show(Screen::Items));
            }
            Action::Back => match screen {
                Screen::Command => self.start(
                    vec![Step::Press { button: Button::B, until: Until::Closed }],
                    Arrival::Closed,
                ),
                // Over a battle the Item list is the battle's: B from it goes back to the battle's command window.
                Screen::Items if self.battle_items => self.start(
                    vec![Step::Press { button: Button::B, until: Until::BattleCommand }],
                    Arrival::Closed,
                ),
                Screen::Menu | Screen::Items => self.start(
                    vec![Step::Press { button: Button::B, until: Until::Command }],
                    Arrival::Show(Screen::Command),
                ),
                // Status is the HD's two pages in one: the original sits on its page 1, and B takes it back to its Menu list.
                Screen::Status => self.start(status::back_steps(), Arrival::Resume(Screen::Menu)),
                // Over a battle there is no menu under it: Back gives the battle its keys again.
                Screen::Cheats if self.battle_cheats => {
                    self.battle_cheats = false;
                    self.nav = Nav::default();
                    self.curtain = Curtain::Down;
                }
                // Back to the Menu list, which the original never left: its cursor stays on Cheats.
                Screen::Cheats => self.nav.resume(self.nav.cheats_back()),
                // The Text screen goes back the same way: its speed was only held, and the old one stays.
                Screen::Options | Screen::Text | Screen::Order | Screen::Save => {
                    self.nav.resume(Screen::Menu);
                }
                // Back to the Options, where the Data packs button was.
                Screen::Mods => self.nav.resume(Screen::Options),
                // The original's Cards screen takes only A, which closes the menu: back is a close and a reopening.
                Screen::Cards => {
                    self.start(cards::back_steps(flying, town), Arrival::Show(Screen::Menu));
                }
            },
        }
    }

    /// What a change of a row of `category` asks for: the cheat block written, the text speed written to the game,
    /// or the file of the category marked to be written.
    fn option_effect(&mut self, category: Category, effect: Effect, wram: &[u8]) {
        match effect {
            Effect::None | Effect::Open(_) => {}
            Effect::Stored => {
                if !category.per_game() {
                    self.settings_changed = true;
                }
            }
            Effect::Cheats => self.cheats_changed(wram),
            Effect::TextSpeed(speed) => {
                self.cheat_events
                    .push(HostEvent::Wram { address: d::TEXT_SPEED, bytes: vec![speed] });
            }
        }
    }

    /// Reset this page (one category) or Reset all (every category): the stored values are forgotten, and the
    /// views (the cheats' encounter rate, the text speed) go back to their defaults, each through its own effect.
    fn reset_options(&mut self, categories: &[Category], probe: &MenuProbe<'_>, wram: &[u8]) {
        let (speed, cheats) = (probe.text_speed(), self.cheats);
        let before = categories.contains(&Category::Video).then(|| self.options.pc.clone());
        for &category in categories {
            self.options.values_mut(category).reset_category(category);
            if !category.per_game() {
                self.settings_changed = true;
            }
            for def in category.rows() {
                let State { options, cheats, .. } = self;
                match def.reset(options.values_mut(category), cheats) {
                    Effect::TextSpeed(now) if now == speed => {}
                    Effect::Cheats => {}
                    effect => self.option_effect(category, effect, wram),
                }
            }
        }
        if self.cheats != cheats {
            self.cheats_changed(wram);
        }
        if let Some(before) = before {
            self.video_changed(&before);
        }
    }

    /// The Video page changed from `before`: the host applies it now, and a change of display, scale or aspect
    /// is asked about. The change is kept in the file only when it is kept.
    fn video_changed(&mut self, before: &Values) {
        if self.options.pc == *before {
            return;
        }
        self.video_apply = true;
        if self.video_backup.is_none() && video::mode(before) != video::mode(&self.options.pc) {
            self.video_backup = Some(before.clone());
            self.video_frames = video::CONFIRM_SECONDS * VIDEO_FPS;
            self.nav.open_video_confirm();
        }
    }

    /// "Apply video changes?" was answered (or timed out): Keep writes the options, Revert puts the video back.
    fn video_answer(&mut self, keep: bool) {
        self.nav.close_video_confirm();
        if let Some(before) = self.video_backup.take() {
            if !keep {
                video::restore(&mut self.options.pc, &before);
                self.video_apply = true;
            }
            self.settings_changed = true;
        }
    }

    /// One frame of the question's countdown; it reverts at the end, and when the Options screen is gone.
    fn video_tick(&mut self) {
        if self.video_backup.is_none() {
            return;
        }
        self.video_frames = self.video_frames.saturating_sub(1);
        if self.video_frames == 0 || self.nav.screen() != Screen::Options {
            self.video_answer(false);
        }
    }

    /// The cheats changed on the screen: the cheat block is written whole (the choices and the cheats'
    /// runtime bytes, which are zero on the map), and the cards of the hand get the Z corners the choice
    /// asks for. The writes are made before the next frame, with the application event that carries the
    /// choices to a replay.
    /// Over a battle only the settings are written: the runtime bytes (the scripted-loss gate) are the battle's.
    fn cheats_changed(&mut self, wram: &[u8]) {
        if self.battle_cheats {
            self.cheat_events.extend(settings_events(&self.cheats, wram));
            self.cheat_events.push(self.cheats_app_event());
            return;
        }
        self.cheat_events.extend(block_events(&self.cheats));
        self.cheat_events.extend(hand_events(&self.cheats, wram));
        self.cheat_events.push(self.cheats_app_event());
    }

    /// The application event that carries the run's cheats to a replay (see [`CHEATS_TAG`]).
    fn cheats_app_event(&self) -> HostEvent {
        HostEvent::App { tag: CHEATS_TAG, payload: self.cheats.to_bytes() }
    }

    /// The cheats of the save the bag is bound to: the settings the sidecar keeps for that save, or all off
    /// for a save without an entry (a new game, or a save changed by another way). The block is written with
    /// them, with the Z corners of the hand.
    fn bind_cheats(&mut self, wram: &[u8]) -> Vec<HostEvent> {
        self.cheat_save = self.keeper.bound_save();
        self.cheats = self
            .cheat_save
            .and_then(|(slot, checksum)| self.cheat_sidecar.entry_for(slot, checksum))
            .map_or_else(Cheats::neutral, |saved| saved.cheats);
        self.options.game = self
            .cheat_save
            .and_then(|(slot, checksum)| self.option_slots.entry_for(slot, checksum))
            .cloned()
            .unwrap_or_default();
        // The money multiplier is kept with the game's options, not in the cheats' sidecar.
        difficulty::restore(&self.options.game, &mut self.cheats);
        let mut events = block_events(&self.cheats);
        events.extend(hand_events(&self.cheats, wram));
        events.push(self.cheats_app_event());
        events
    }

    /// The mod state of the save the bag is bound to. The markers are cleared first, so that a part the save does not
    /// keep reads neutral: a save without an entry (a new game, or a save changed by another way) and a part missing
    /// from the save's state keep none of the flags or high parts the run had. The save's state follows, with its
    /// markers, when the sidecar has an entry for the save.
    fn bind_mods(&mut self, wram: &[u8]) -> Vec<HostEvent> {
        self.mods_save = self.keeper.bound_save();
        let mut events = clear_events(wram);
        let entry =
            self.mods_save.and_then(|(slot, checksum)| self.mods_sidecar.entry_for(slot, checksum));
        if let Some(state) = entry {
            events.push(HostEvent::App { tag: MODS_TAG, payload: state.to_bytes() });
            events.extend(apply_events(state));
        }
        events
    }

    /// The bind, the list's keeping and the plan's next frame: see [`Driver::frame`].
    fn frame(
        &mut self,
        probe: &MenuProbe<'_>,
        labels: &Labels,
        wram: &[u8],
        sram: Option<&[u8]>,
        live: Joypad,
    ) -> Frame {
        let mut events = Vec::new();
        let bound_now = !self.keeper.bound() && self.binding == Binding::Auto;
        if bound_now {
            let fresh = probe.field_free() && probe.field_mode();
            events = self.keeper.bind(wram, labels, sram, &self.sidecar, fresh);
        }
        // A bind writes the list as the bag projects it: the list is kept from the next frame on.
        // A sale of the shop leaves the list in transit until its message has run: the bag waits for it.
        if self.keeper.bound() && !bound_now {
            let pending = probe.sale_pending();
            events.extend(self.keeper.sync_shop(wram, labels, sram, &self.sidecar, pending));
        }
        // The cheats follow the save the bag is bound to: a bind (a new game has none on), or a save copied
        // into the game, gives the cheats of that save.
        if self.keeper.bound() && (bound_now || self.keeper.bound_save() != self.cheat_save) {
            events.extend(self.bind_cheats(wram));
        }
        if self.keeper.bound() && (bound_now || self.keeper.bound_save() != self.mods_save) {
            events.extend(self.bind_mods(wram));
        }
        events.append(&mut self.cheat_events);
        if self.cheats_installed {
            events.extend(frame_events(wram));
        }
        // The ending of the game (`mods::chapter`): the marker its wrapper set is reported once.
        events.extend(crate::mods::chapter::completion_events(wram));
        let pad = self.pad(probe, labels, wram, live, &mut events);
        let completed = self.save_completed(probe, sram);
        let bag = self.keeper.saved(sram, wram);
        // The cheats of a Save are written for the slot it wrote, also when the slot's checksum word is the same.
        let written =
            completed.or_else(|| bag.as_ref().map(|&(slot, checksum, _)| (slot, checksum)));
        let saved_cheats = self.store_cheats(written);
        let saved_mods = self.store_mods(written, wram);
        let saved = bag.map(|(slot, checksum, bag)| {
            self.sidecar.store(slot, checksum, &bag);
            SavedBag { slot, checksum, bag }
        });
        let saved_options = saved_cheats.is_some();
        if self.nav.screen() == Screen::Save {
            self.refresh_save_slots(labels, sram);
        }
        self.video_tick();
        self.nav.tick();
        self.sync_device();
        // A change waiting to be kept is not written: a crash in those ten seconds leaves the file as it was.
        let settings = (self.video_backup.is_none() && std::mem::take(&mut self.settings_changed))
            .then(|| self.options.pc.clone());
        let video = std::mem::take(&mut self.video_apply).then(|| self.options.pc.clone());
        Frame { pad, events, saved, saved_cheats, saved_mods, saved_options, settings, video }
    }

    /// The HD Save screen's slots, as the cartridge `sram` has them now (a Save that just wrote one shows at once), and
    /// which of them hold a game, for the keys.
    fn refresh_save_slots(&mut self, labels: &Labels, sram: Option<&[u8]>) {
        self.save_slots = save::slots(sram, labels, &self.sidecar);
        self.save_marks = sram.map_or([false; 3], |sram| self.marked_slots(sram));
        self.nav.set_save_used(std::array::from_fn(|slot| self.save_slots[slot].used()));
    }

    /// Which save slots carry the CHEATS badge: a good save whose cheats were saved with one on, by its checksum word (a
    /// slot changed by another way has no entry).
    fn marked_slots(&self, sram: &[u8]) -> [bool; 3] {
        let mut marks = [false; 3];
        for (slot, mark) in (0..3_u8).zip(marks.iter_mut()) {
            if let Some(checksum) = good_checksum(sram, slot) {
                *mark = self.cheat_sidecar.marked(slot, checksum);
            }
        }
        marks
    }

    /// The Save of the original, followed on work RAM at every frame. While the Save's list shows, its cursor (the
    /// slot, 0 to 2) is kept; the Save completes when the screen reaches its completion step, and then the slot
    /// the list chose is the one written. Returns that slot and its checksum word, once per Save, when `sram`
    /// holds the slot as a good save.
    fn save_completed(&mut self, probe: &MenuProbe<'_>, sram: Option<&[u8]>) -> Option<(u8, u16)> {
        let on_save = probe.screen() == Some(SAVE_SCREEN);
        if on_save && probe.step() == Some(SAVE_LIST_STEP) {
            self.save_slot =
                probe.cursor().filter(|&slot| usize::from(slot) < ssd_save::SLOT_COUNT);
        }
        let done = on_save && probe.step() == Some(SAVE_DONE_STEP);
        let completed = done && !self.save_done;
        self.save_done = done;
        if !completed {
            return None;
        }
        let slot = self.save_slot.take()?;
        Some((slot, good_checksum(sram?, slot)?))
    }

    /// Stores the run's cheats for the save written at `written` (its slot and checksum word), marked when a cheat
    /// is on, and returns that entry. The run keeps the save its bag is bound to: a Save to another slot leaves the
    /// binding (and so the run's cheats) where it was, and only that slot's entry is written.
    fn store_cheats(&mut self, written: Option<(u8, u16)>) -> Option<SavedCheats> {
        let (slot, checksum) = written?;
        self.cheat_sidecar.store(slot, checksum, self.cheats);
        difficulty::store(&self.cheats, &mut self.options.game);
        self.option_slots.store(slot, checksum, self.options.game.clone());
        if self.keeper.bound_save() == Some((slot, checksum)) {
            self.cheat_save = Some((slot, checksum));
        }
        self.cheat_sidecar.entry_for(slot, checksum).copied()
    }

    /// Stores the run's mod state for the save written at `written` (its slot and checksum word): the markers and their
    /// bytes as `wram` has them now. A state with no markers leaves the slot no entry. The save is reported either way
    /// (see [`Frame::saved_mods`]), so the file drops an entry the slot had.
    fn store_mods(&mut self, written: Option<(u8, u16)>, wram: &[u8]) -> Option<(u8, u16)> {
        let (slot, checksum) = written?;
        self.mods_sidecar.store(slot, checksum, capture(wram));
        if self.keeper.bound_save() == Some((slot, checksum)) {
            self.mods_save = Some((slot, checksum));
        }
        Some((slot, checksum))
    }

    /// The joypad of the next frame of the plan (the live joypad when no plan runs and the curtain
    /// is not up), collecting the host events the plan's stages make.
    fn pad(
        &mut self,
        probe: &MenuProbe<'_>,
        labels: &Labels,
        wram: &[u8],
        live: Joypad,
        events: &mut Vec<HostEvent>,
    ) -> Joypad {
        if self.running.is_none() {
            self.observe(probe);
        }
        if let Some(running) = self.running.as_mut() {
            let arrival = running.arrival;
            loop {
                match running.player.next(probe) {
                    Next::Frame(pad) => return pad,
                    Next::Stage(id) => {
                        events.extend(self.keeper.stage(wram, labels, id));
                    }
                    // The original's own screen comes up from here on: the player sees what it shows.
                    Next::Lift => self.curtain = Curtain::Lifted,
                    Next::Done(outcome) => {
                        self.running = None;
                        self.last = Some(outcome);
                        self.arrive(arrival, outcome);
                        break;
                    }
                }
            }
        }
        match self.curtain {
            Curtain::Up => {
                self.auto_run = AutoRun::default();
                Joypad::default()
            }
            Curtain::Down | Curtain::Lifted => {
                // A finished message that waits for A: the host presses it after the Gameplay page's wait.
                let mut pad = live;
                self.auto_run.apply(wram, &self.options.game, &mut pad);
                pad
            }
        }
    }

    /// What the HD draws: see [`Driver::scene`], in the style of this PC's Accessibility page.
    fn scene(&self, probe: &MenuProbe<'_>, labels: &Labels) -> Option<DrawList> {
        let style = ssd_menus::options::accessibility::style(&self.options.pc);
        self.scene_plain(probe, labels).map(|list| style.apply(list))
    }

    /// The screen in the design as drawn.
    fn scene_plain(&self, probe: &MenuProbe<'_>, labels: &Labels) -> Option<DrawList> {
        if self.nav.screen() == Screen::Cheats {
            return Some(screens::cheats_screen(&self.cheats_view(labels)));
        }
        if self.nav.screen() == Screen::Options {
            return Some(screens::options_screen(&self.options_view(probe)));
        }
        if self.nav.screen() == Screen::Mods {
            return Some(screens::mods_screen(&self.mods_view()));
        }
        if self.nav.screen() == Screen::Status {
            return Some(screens::status_screen(
                &probe.status_view(usize::from(self.nav.cursor())),
            ));
        }
        if self.nav.screen() == Screen::Cards {
            let cursor = usize::from(self.nav.cursor());
            return Some(screens::cards_screen(&probe.cards_view(cursor)));
        }
        if self.nav.screen() == Screen::Order {
            return Some(screens::order_screen(&order::view(&self.order, labels)));
        }
        if self.nav.screen() == Screen::Text {
            let text = self.nav.text();
            return Some(screens::text_screen(&screens::TextView {
                speed: text.speed(),
                frames: text.frames(),
            }));
        }
        if self.nav.screen() == Screen::Save {
            let save = self.nav.save();
            return Some(screens::save_screen(&screens::SaveView {
                now: probe.save_now(self.keeper.bag().map(ssd_menus::bag::Bag::units)),
                slots: self.save_slots.clone(),
                badges: self.save_marks,
                cursor: save.cursor(),
                ask: save.ask(),
                phase: save.phase(),
            }));
        }
        let Context { flying, town } = context(probe);
        let mut view: MenuView = probe.view_as(
            self.nav.screen(),
            town,
            flying && !town,
            self.nav.cursor(),
            self.nav.page(),
        )?;
        if view.screen == Screen::Items {
            view.bag = self.bag_view(labels, probe.encounter_group());
            view.selected = self.selected(labels, probe.encounter_group());
        }
        let unbound = view.screen == Screen::Items && view.bag.is_none();
        let mut list = screens::scene(&view);
        // A bag not bound yet shows the original's list, which has no foot for a line: the notice goes on top.
        if let Some(text) = self.notice.filter(|_| unbound) {
            list.append(screens::notice(text));
        }
        Some(list)
    }

    /// The Options screen as it shows: the category and row under the cursor and the value of each row.
    fn options_view(&self, probe: &MenuProbe<'_>) -> OptionsView {
        let category = self.nav.options_category();
        OptionsView {
            category,
            row: self.nav.options_row(),
            values: self.options.indices(category, &self.cheats, probe.text_speed()),
            confirm: self.nav.options_confirm(),
            controls: (category == Category::Controls).then(|| {
                let device = self.nav.controls_device();
                ControlsView {
                    listening: self.nav.options_listening(),
                    refused: self.nav.options_refusal().map(|refusal| RefusedKey {
                        position: refusal.position,
                        message: controls::refusal(device, refusal.by),
                        blink: refusal.flash > 0 && (refusal.flash / 8) % 2 == 0,
                    }),
                }
            }),
            video: self.nav.options_video_confirm().map(|choice| ssd_menus::view::VideoAsk {
                choice,
                tenths: (self.video_frames * 100)
                    .div_ceil(video::CONFIRM_SECONDS * VIDEO_FPS)
                    .min(100),
                summary: video::summary(&self.options.pc),
            }),
        }
    }

    /// The Cheats screen as it shows: the run's choices, the row under the cursor and the characters' names.
    fn cheats_view(&self, labels: &Labels) -> CheatsView {
        CheatsView {
            cheats: self.cheats,
            row: self.nav.cheats_row(),
            names: labels
                .characters
                .iter()
                .take(usize::from(CHARACTERS))
                .map(|character| character.name.clone())
                .collect(),
            spawner: self.nav.spawner().map(|spawner| self.spawner_view(spawner, labels)),
        }
    }

    /// The item spawner's page as it shows: the objects of the page the cursor is on, with the units the bag holds.
    fn spawner_view(&self, spawner: Spawner, labels: &Labels) -> SpawnerView {
        let name = |object: u8| {
            labels
                .objects
                .get(usize::from(object))
                .map_or_else(|| format!("#{object}"), |l| l.name.clone())
        };
        let in_bag = |object: u8| self.keeper.bag().map_or(0, |bag| bag.units_of(object));
        let first = spawner.object() / SPAWNER_PAGE * SPAWNER_PAGE;
        let rows = (first..(first + SPAWNER_PAGE).min(SPAWNER_OBJECTS))
            .map(|object| SpawnerRow {
                object,
                name: name(object),
                story: Keeper::story(labels, object),
                in_bag: in_bag(object),
            })
            .collect();
        let object = spawner.object();
        SpawnerView {
            rows,
            object,
            name: name(object),
            help: labels
                .objects
                .get(usize::from(object))
                .map(|l| l.help.clone())
                .unwrap_or_default(),
            story: Keeper::story(labels, object),
            count: spawner.count(),
            confirm: spawner.confirm(),
        }
    }

    /// Whether a stack of `object` can be used at the Item screen now: the map's rule, or over a battle the battle's.
    fn usable(&self, object: u8, label: Option<&ObjectLabel>) -> bool {
        if self.battle_items { usable_in_battle(label) } else { usable_here(object, label) }
    }

    /// Whether a stack can be dropped at the Item screen now: never over a battle, where the HD offers no drop.
    fn can_drop(&self, label: Option<&ObjectLabel>, key: bool) -> bool {
        !self.battle_items && droppable(label, key)
    }

    /// The bag's rows of the tab the Item screen shows, as the navigation reads them; `group` is the encounter group
    /// of the battle, which an object's attribute may depend on ([`label_now`]).
    fn rows(&self, labels: &Labels, group: u8) -> Vec<ItemRow> {
        let Some(bag) = self.keeper.bag() else { return Vec::new() };
        bag.stacks()
            .iter()
            .filter(|stack| self.nav.tab().shows(ally(labels, stack.object)))
            .map(|stack| {
                let label = label_now(labels, stack.object, group);
                let label = label.as_deref();
                ItemRow {
                    id: stack.id,
                    usable: self.usable(stack.object, label),
                    droppable: self.can_drop(label, stack.class.key),
                }
            })
            .collect()
    }

    /// The bag as the Item screen shows it, once the bag is bound; `group` as for [`Self::rows`].
    fn bag_view(&self, labels: &Labels, group: u8) -> Option<BagView> {
        let bag = self.keeper.bag()?;
        let tab = self.nav.tab();
        let stacks: Vec<StackView> = bag
            .stacks()
            .iter()
            .filter(|stack| tab.shows(ally(labels, stack.object)))
            .map(|stack| {
                let label = label_now(labels, stack.object, group);
                let label = label.as_deref();
                StackView {
                    id: stack.id,
                    object: stack.object,
                    name: label.map_or_else(|| "?".to_owned(), |label| label.name.clone()),
                    count: stack.count,
                    usable: self.usable(stack.object, label),
                    droppable: self.can_drop(label, stack.class.key),
                    ally: ally(labels, stack.object),
                }
            })
            .collect();
        let under = stacks.get(self.nav.bag_cursor());
        Some(BagView {
            tab,
            used: bag.stacks().len(),
            slots: usize::from(bag.rules().slots),
            full: bag.is_full(),
            usable_here: under.is_some_and(|stack| stack.usable),
            droppable_here: under.is_some_and(|stack| stack.droppable),
            sheet: self.nav.sheet(),
            choice: self.nav.choice(),
            notice: self.notice,
            stacks,
        })
    }

    /// The object under the Item screen's cursor: its picture, price and help.
    fn selected(&self, labels: &Labels, group: u8) -> Option<Selected> {
        let bag = self.keeper.bag()?;
        let tab = self.nav.tab();
        let stack = bag
            .stacks()
            .iter()
            .filter(|stack| tab.shows(ally(labels, stack.object)))
            .nth(self.nav.bag_cursor())?;
        let label = label_now(labels, stack.object, group)?;
        Some(Selected {
            object: stack.object,
            name: label.name.clone(),
            // Porunga's box shows the wishes he has left (the second byte of his state word)
            price: if stack.object == PORUNGA {
                Some((stack.extra >> 8) as u8)
            } else {
                label.price
            },
            help: label.help.clone(),
            battle: label.battle,
            field: label.field,
        })
    }

    /// Starts a plan that leaves the curtain at `arrival`.
    /// The Data packs screen opens from the Options' button: the list is the packs as the set the run uses has them, the
    /// cursor on the first line.
    fn open_packs(&mut self) {
        self.packs_edit = self
            .packs
            .packs
            .iter()
            .enumerate()
            .map(|(pack, info)| Listed { pack, on: info.on })
            .collect();
        self.nav.enter(Screen::Mods);
    }

    /// The chapters the Data packs screen offers, in its order, each with its id (none for "Main game", which a chapter
    /// session offers first). The packs that are on offer theirs; a chapter that waits is locked until the game is
    /// completed or the cheat "Unlock chapters" is on.
    fn offered(&self) -> Vec<(Option<String>, ChapterLine)> {
        let mut offered = Vec::new();
        if let Some(name) = &self.packs.playing {
            offered.push((
                None,
                ChapterLine {
                    name: "Main game".to_owned(),
                    description: format!("Leave {name}: the game goes to the main saves."),
                    locked: false,
                    main: true,
                },
            ));
        }
        for listed in self.packs_edit.iter().filter(|listed| listed.on) {
            let pack = &self.packs.packs[listed.pack];
            if pack.refusal.is_some() {
                continue;
            }
            for chapter in &pack.chapters {
                let locked = chapter.waits && !self.packs.completed && !self.cheats.chapters();
                offered.push((
                    Some(chapter.id.clone()),
                    ChapterLine {
                        name: chapter.name.clone(),
                        description: chapter.description.clone(),
                        locked,
                        main: false,
                    },
                ));
            }
        }
        offered
    }

    /// The lines of the Data packs screen's list: the packs in their order, "Install a pack...", then the chapters offered (see
    /// [`State::offered`]).
    fn mods_lines(&self) -> Vec<ModsLine> {
        let mut lines: Vec<ModsLine> = self
            .packs_edit
            .iter()
            .map(|listed| ModsLine::Pack {
                refused: self.packs.packs[listed.pack].refusal.is_some(),
            })
            .collect();
        lines.push(ModsLine::Install);
        lines.extend(self.offered().iter().map(|(_, chapter)| {
            if chapter.main { ModsLine::Main } else { ModsLine::Chapter { locked: chapter.locked } }
        }));
        lines
    }

    /// Whether the list as the player left it is not the set the run uses (the same packs, in the same order).
    fn edit_differs(&self) -> bool {
        let pending: Vec<usize> =
            self.packs_edit.iter().filter(|listed| listed.on).map(|listed| listed.pack).collect();
        let applied: Vec<usize> = self
            .packs
            .packs
            .iter()
            .enumerate()
            .filter(|(_, info)| info.on)
            .map(|(pack, _)| pack)
            .collect();
        pending != applied
    }

    /// Apply: the set the list gives, in its order, is asked of the host (when it is not the set the run uses already).
    fn apply_packs(&mut self) {
        if !self.edit_differs() {
            self.notice = Some(UNCHANGED);
            return;
        }
        let ids = self
            .packs_edit
            .iter()
            .filter(|listed| listed.on)
            .map(|listed| self.packs.packs[listed.pack].id.clone())
            .collect();
        self.mods_request = Some(Request::Apply(ids));
    }

    /// Start a chapter (the line `line` of the list): the game restarts into its saves, when the list is the set the run uses.
    fn start_chapter(&mut self, line: usize) {
        // The packs, then "Install a pack...", then the chapters
        let Some(index) = line.checked_sub(self.packs_edit.len() + 1) else { return };
        let offered = self.offered();
        let Some((Some(id), _)) = offered.get(index) else { return };
        if self.edit_differs() {
            self.notice = Some(APPLY_FIRST);
            return;
        }
        self.mods_request = Some(Request::Chapter(id.clone()));
    }

    /// Back to the main game: the game restarts into the main saves, when the list is the set the run uses.
    fn main_game(&mut self) {
        if self.edit_differs() {
            self.notice = Some(APPLY_FIRST);
            return;
        }
        self.mods_request = Some(Request::Main);
    }

    /// What the Data packs screen shows now: the list, the chapters, the cursor, the question and the notice.
    fn mods_view(&self) -> ModsView {
        let packs = self
            .packs_edit
            .iter()
            .map(|listed| {
                let info = &self.packs.packs[listed.pack];
                let state = if info.refusal.is_some() {
                    PackState::Refused
                } else if listed.on {
                    PackState::On
                } else {
                    PackState::Off
                };
                PackLine {
                    name: info.name.clone(),
                    version: info.version.clone(),
                    author: info.author.clone(),
                    description: info.description.clone(),
                    needs: info.needs.clone(),
                    size: info.size,
                    state,
                    reason: info.refusal.clone(),
                }
            })
            .collect();
        ModsView {
            packs,
            chapters: self.offered().into_iter().map(|(_, chapter)| chapter).collect(),
            cursor: self.nav.mods_cursor(),
            moving: self.nav.mods_moving(),
            ask: self.nav.mods_ask(),
            notice: self
                .notice
                .map(str::to_owned)
                .or_else(|| self.mods_alert.clone())
                .or_else(|| self.packs.notice.clone()),
        }
    }

    fn start(&mut self, steps: Vec<Step>, arrival: Arrival) {
        self.running = Some(Running { player: Player::new(steps), arrival });
    }

    /// The curtain follows the menu the player opens or closes, when no plan runs.
    fn observe(&mut self, probe: &MenuProbe<'_>) {
        // A use is over once the original is back on its item list, on the battle's command window or on the field: the
        // list may follow the bag's order again.
        if self.keeper.staged()
            && (probe.field_free()
                || probe.battle_command()
                || (probe.screen() == Some(2) && probe.step() == Some(1)))
        {
            self.keeper.release();
        }
        // The Item list of a battle is the HD's Items screen: the screen goes over the battle, and the game gets no
        // buttons until Back or a use. Only the list itself (step 1): the steps of a use (the fighter it is for, its
        // message) are the original's, and the player answers them there.
        if self.curtain != Curtain::Up
            && probe.battle_list()
            && probe.step() == Some(1)
            && self.keeper.bound()
        {
            self.nav = Nav::default();
            self.nav.enter(Screen::Items);
            self.curtain = Curtain::Up;
            self.battle_items = true;
            return;
        }
        // After a use the player answered on the original's screens: its item list on the map is the HD's again.
        if self.curtain == Curtain::Lifted
            && !probe.in_battle()
            && probe.menu_open()
            && probe.screen() == Some(2)
            && probe.step() == Some(1)
            && !probe.busy()
            && self.keeper.bound()
        {
            self.nav.enter(Screen::Items);
            self.curtain = Curtain::Up;
            return;
        }
        // The battle took its command back after a use: the battle is the game's.
        if self.curtain == Curtain::Lifted && probe.battle_command() {
            self.nav = Nav::default();
            self.curtain = Curtain::Down;
            self.battle_cheats = false;
            self.battle_items = false;
            return;
        }
        self.curtain = match self.curtain {
            // A menu that opens always opens on its command window; the screen byte is not set yet
            // then, and it may still hold the last screen of the menu before.
            Curtain::Down if probe.opening() => {
                self.nav.open();
                Curtain::Up
            }
            Curtain::Down => match probe.screen() {
                Some(0 | 13) if probe.menu_open() => {
                    self.nav.open();
                    Curtain::Up
                }
                _ if probe.menu_open() => Curtain::Lifted,
                _ => Curtain::Down,
            },
            Curtain::Up | Curtain::Lifted if probe.field_free() => {
                self.nav = Nav::default();
                Curtain::Down
            }
            Curtain::Up | Curtain::Lifted => self.curtain,
        };
        // The battle the Cheats were open over has ended: the next change is the field's again.
        if self.curtain == Curtain::Down {
            self.battle_cheats = false;
            self.battle_items = false;
        }
    }

    /// Where a plan that ended leaves the curtain.
    fn arrive(&mut self, arrival: Arrival, outcome: Outcome) {
        self.curtain = match (outcome, arrival) {
            // The original wrote the slot and shows "Completed!": the HD's own message waits for A.
            (Outcome::Reached, Arrival::Saved(slot)) => {
                self.nav.save_done(slot);
                Curtain::Up
            }
            (Outcome::Reached, Arrival::Show(screen)) => {
                self.nav.enter(screen);
                Curtain::Up
            }
            (Outcome::Reached, Arrival::Resume(screen)) => {
                self.nav.resume(screen);
                Curtain::Up
            }
            // A plan that could not close the menu leaves the original's menu to the player, with the
            // keys; one that aborted has closed it.
            (Outcome::Reached, Arrival::Lifted) | (Outcome::Stuck { .. }, _) => Curtain::Lifted,
            (Outcome::Reached, Arrival::Closed) | (Outcome::Aborted { .. }, _) => Curtain::Down,
        };
        if self.curtain == Curtain::Down {
            self.nav = Nav::default();
            self.battle_cheats = false;
            self.battle_items = false;
        }
    }
}

/// What the menu of `probe` has: the command window's rows (the town has no Fly, the flying party has
/// Land).
fn context(probe: &MenuProbe<'_>) -> Context {
    let town = probe.town();
    Context { flying: probe.flying() && !town, town }
}

/// The label of an object, if the labels have it.
fn label(labels: &Labels, object: u8) -> Option<&ObjectLabel> {
    labels.objects.get(usize::from(object))
}

/// Whether a stack of `object` is listed under Allies (a person of the story).
fn ally(labels: &Labels, object: u8) -> bool {
    label(labels, object).is_some_and(|label| label.ally)
}

/// Whether an object can be used on the map: EXT, not a story helper, and not refused by the original (bit 0 of
/// its attribute is its "can't use" answer: Saiba's object).
fn usable_here(object: u8, label: Option<&ObjectLabel>) -> bool {
    label.is_some_and(|label| label.field && !label.story && label.attribute & 0x01 == 0)
        && !BATTLE_EFFECTS.contains(&object)
}

/// The story objects whose effects are for battles (`docs/re/data/menus.md`, the effect table): on the map the
/// original takes no use of them, so the HD does not offer one (Gohan's object, 31, is the one with the map bit).
const BATTLE_EFFECTS: [u8; 5] = [27, 28, 29, 30, 31];

/// Whether an object can be used over a battle: its battle bit (INT) is set and it is not a story object, which the
/// menus never use. The original's list of a battle shows every entry: an object without the battle bit gets bit 0
/// ("can't use") from `01:C535`, so the HD greys it out and refuses it the same way (`docs/re/data/menus.md`).
fn usable_in_battle(label: Option<&ObjectLabel>) -> bool {
    label.is_some_and(|label| label.battle && !label.story)
}

/// Whether an object can be dropped: not a key item (a Dragon Ball or a story object), and not fixed.
fn droppable(label: Option<&ObjectLabel>, key: bool) -> bool {
    !key && label.is_some_and(|label| !label.fixed)
}

/// The steps and the arrival of choosing `entry` on `screen`, or `None` for an entry with nothing
/// to do (greyed out, or not on this screen). The row of an entry is its row in the original's list.
fn choice(screen: Screen, entry: Entry, flying: bool, town: bool) -> Option<(Vec<Step>, Arrival)> {
    let (row, rows, until, arrival) = match screen {
        Screen::Command => {
            let rows = command_rows(flying, town);
            let row = rows.iter().position(|&e| e == entry)?;
            let (until, arrival) = match entry {
                Entry::Talk | Entry::Look => (Until::Taken, Arrival::Lifted),
                Entry::Fly => (Until::Screen(1), Arrival::Lifted),
                Entry::Land => (Until::Screen(5), Arrival::Lifted),
                Entry::Item => (Until::ItemList, Arrival::Show(Screen::Items)),
                Entry::Menu => (Until::Screen(4), Arrival::Show(Screen::Menu)),
                _ => return None,
            };
            (row, rows.len(), until, arrival)
        }
        Screen::Menu => {
            let (row, until) = match entry {
                Entry::Cards => (0, 6),
                Entry::Status => (1, 7),
                Entry::Order => (2, 8),
                Entry::Text => (3, 9),
                Entry::Save => (4, 10),
                _ => return None,
            };
            // Cards and Status are the HD's: the original deals the hand or sits on its page 1, and the HD draws it
            let arrival = match entry {
                Entry::Cards => Arrival::Show(Screen::Cards),
                Entry::Status => Arrival::Show(Screen::Status),
                _ => Arrival::Lifted,
            };
            (row, ORIGINAL_MENU_ROWS, Until::Screen(until), arrival)
        }
        Screen::Items
        | Screen::Cheats
        | Screen::Options
        | Screen::Mods
        | Screen::Cards
        | Screen::Text
        | Screen::Order
        | Screen::Status
        | Screen::Save => {
            return None;
        }
    };
    // The original draws what comes next, so the curtain lifts as the last press goes: the HD cursor moved on the
    // rows above, and the HD window does not stay up (with rows the original's own window may not have) while
    // the original runs its message or its scene.
    let mut steps =
        vec![Step::Row { row: u8::try_from(row).ok()?, rows: u8::try_from(rows).ok()? }];
    if arrival == Arrival::Lifted {
        steps.push(Step::Lift);
    }
    steps.push(Step::Press { button: Button::A, until });
    Some((steps, arrival))
}

/// The first bytes of [`Driver::session_bytes`], and its layout's version.
const SESSION_MAGIC: &[u8; 4] = b"SSDH";
const SESSION_VERSION: u8 = 1;

/// Frames the original runs after a window is drawn or a menu closes, before it takes the next press
/// (a press in the first frames after a draw is lost).
const SETTLE_WINDOW: u32 = 120;

/// Frames the original's Use or Drop window of the Item screen runs once it is drawn and idle, before it takes the press
/// that chooses Use or Drop.
const SETTLE_CHOICE: u32 = 8;

/// The steps of using one unit of the stack `stack`, from the Item list the HD shows: the unit is staged first on the
/// original's list and the original's own Use is pressed. From there the original is the player's: the curtain lifts on
/// its use window, and whatever the use asks next (the fighter it is for, a card to change, a wish, the lines of a
/// message, the battle's target) the player answers on the original's own screens. The HD comes back when the original
/// is on its item list again (see `State::observe`), or goes when its menu closes or the battle takes its command back.
fn use_steps(stack: u32) -> Vec<Step> {
    vec![
        Step::Stage { id: stack },
        Step::ListTop,
        Step::Press { button: Button::A, until: Until::ItemChoice },
        Step::Lift,
        Step::Wait { until: Until::Ready },
        Step::Pause { frames: SETTLE_CHOICE },
        Step::Row { row: 0, rows: 2 },
        Step::Press { button: Button::A, until: Until::UseTaken },
    ]
}

/// The steps of dropping one unit of the stack `stack`: the unit is staged on the list, its Drop is chosen and
/// confirmed with Yes, and the list is back. The HD's own Yes or No asked already, so the original's window
/// stays under the curtain.
fn drop_steps(stack: u32) -> Vec<Step> {
    vec![
        Step::Stage { id: stack },
        Step::ListTop,
        Step::Press { button: Button::A, until: Until::ItemChoice },
        Step::Wait { until: Until::Ready },
        Step::Pause { frames: SETTLE_CHOICE },
        Step::Row { row: 1, rows: 2 },
        Step::Press { button: Button::A, until: Until::DropAsk },
        Step::Row { row: 0, rows: 2 },
        Step::Press { button: Button::A, until: Until::ItemList },
    ]
}

#[cfg(test)]
mod tests {
    use super::super::testing::{FakeMenu, held};
    use super::*;
    use crate::mods::state::{BLOCK, FLAGS_MARKER_AT, MARKER_AT};
    use ssd_menus::mods_state::{FLAG_BYTES, HIGH_BYTES};

    /// One frame: the driver gives the joypad (given the live one), and the fake menu runs it.
    fn frame(driver: &mut Driver, menu: &mut FakeMenu, live: Joypad) -> Joypad {
        let wram = menu.wram();
        let pad = driver.frame(&wram, None, live).pad;
        menu.tick(pad);
        pad
    }

    /// Frames until the driver has no plan running.
    fn settle(driver: &mut Driver, menu: &mut FakeMenu) {
        for _ in 0..2_000 {
            frame(driver, menu, Joypad::default());
            if !driver.busy() {
                return;
            }
        }
        panic!("the plan did not end");
    }

    /// A key of the HD, then the frames of its plan.
    fn choose(driver: &mut Driver, menu: &mut FakeMenu, key: Key) {
        driver.key(key, &menu.wram());
        settle(driver, menu);
    }

    /// The player opens the menu on the field with a live press of A.
    fn open_by_player(driver: &mut Driver, menu: &mut FakeMenu) {
        let mut a = Joypad { a: true, ..Joypad::default() };
        for _ in 0..40 {
            frame(driver, menu, a);
            a = Joypad::default();
        }
        assert_eq!(driver.curtain(), Curtain::Up, "the curtain is up on the command window");
    }

    #[test]
    fn the_players_press_opens_the_menu_under_the_curtain_and_the_keys_are_the_hds() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        let a = Joypad { a: true, ..Joypad::default() };
        frame(&mut driver, &mut menu, a);
        assert_eq!(driver.curtain(), Curtain::Down, "the press opens the menu on the next frame");
        frame(&mut driver, &mut menu, Joypad::default());
        assert_eq!(driver.curtain(), Curtain::Up);
        assert_eq!(frame(&mut driver, &mut menu, a), Joypad::default(), "live keys go to the HD");
        assert_eq!(driver.nav().screen(), Screen::Command);
    }

    #[test]
    fn a_reset_brings_the_curtain_down_and_unbinds_the_bag() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        driver.key(Key::Down, &menu.wram());
        driver.reset();
        assert_eq!(driver.curtain(), Curtain::Down);
        assert!(driver.bag().is_none());
        assert!(!driver.busy());
    }

    #[test]
    fn a_menu_that_opens_over_a_stale_screen_byte_still_gets_the_curtain() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        // The last menu closed from the Item screen (the Dragon Radar does), so `menu_screen` is 2
        // while the next menu opens.
        menu.stale = true;
        open_by_player(&mut driver, &mut menu);
        assert_eq!(driver.nav().screen(), Screen::Command);
    }

    #[test]
    fn an_item_is_chosen_with_the_hd_cursor_and_cancel_brings_back_its_row() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        for _ in 0..3 {
            driver.key(Key::Down, &menu.wram());
        }
        assert_eq!(driver.nav().cursor(), 3, "the HD cursor is on Item");
        assert_eq!(menu.cursor, 0, "the original has not moved");
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Items));
        assert_eq!(menu.screen, 2, "the original is on its Item list");
        choose(&mut driver, &mut menu, Key::Cancel);
        assert_eq!(driver.curtain(), Curtain::Up);
        assert_eq!((driver.nav().screen(), driver.nav().cursor()), (Screen::Command, 3));
        assert_eq!(menu.screen, 0);
        choose(&mut driver, &mut menu, Key::Cancel);
        assert_eq!(driver.curtain(), Curtain::Down, "cancel at the command window closes the menu");
        assert_eq!(menu.field, 0);
        assert_eq!(driver.last_outcome(), Some(Outcome::Reached));
    }

    /// The labels of the battle tests: Bulma (battle and map), Scouter (map only), Yajirobe (battle only), and a
    /// Dragon Ball (a story object, never used from the menu).
    fn battle_labels() -> Labels {
        let mut objects = vec![ObjectLabel::default(); 40];
        objects[2] = ObjectLabel {
            name: "Bulma".to_owned(),
            battle: true,
            field: true,
            ..ObjectLabel::default()
        };
        objects[16] =
            ObjectLabel { name: "Scouter".to_owned(), field: true, ..ObjectLabel::default() };
        objects[18] =
            ObjectLabel { name: "Yajirobe".to_owned(), battle: true, ..ObjectLabel::default() };
        objects[33] = ObjectLabel {
            name: "Dragon Ball".to_owned(),
            battle: true,
            story: true,
            ..ObjectLabel::default()
        };
        Labels { objects, ..Labels::default() }
    }

    /// A driver bound to a bag of two Bulmas, a Scouter and a Yajirobe, and the fake menu in a battle, on its command
    /// window (the bag is bound from a recorded bind, and the fake menu keeps the list the keeper writes).
    fn battle_with_bag() -> (Driver, FakeMenu) {
        let mut bag = Bag::new(Rules::default());
        for object in [2, 2, 16, 18] {
            bag.add(object, 0, ssd_menus::bag::Class::of(object, false)).unwrap();
        }
        let mut driver = Driver::new(battle_labels());
        let mut menu = FakeMenu::default();
        menu.battle = true;
        menu.field = 2;
        menu.screen = 17;
        let events = driver.replay_event(BIND_TAG, &bag.to_bytes(), &menu.wram()).unwrap();
        menu.absorb(&events);
        (driver, menu)
    }

    /// The player's presses on the battle's command window: Down to Item and A. The battle's Item list is drawn over
    /// the next frames, and the HD's Items screen goes over it.
    fn open_battle_list(driver: &mut Driver, menu: &mut FakeMenu) {
        assert_eq!(driver.curtain(), Curtain::Down, "the battle's command window is the game's");
        frame(driver, menu, Joypad { down: true, ..Joypad::default() });
        frame(driver, menu, Joypad { a: true, ..Joypad::default() });
        for _ in 0..10 {
            frame(driver, menu, Joypad::default());
        }
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Items));
    }

    #[test]
    fn the_battle_item_list_opens_the_items_screen_over_the_battle() {
        let (mut driver, mut menu) = battle_with_bag();
        open_battle_list(&mut driver, &mut menu);
        assert_eq!(menu.screen, 2, "the original is on its Item list, under the HD's screen");
        assert!(driver.state.battle_items, "the screen is the battle's");
        assert_eq!(
            driver.bag().map(Bag::units),
            Some(4),
            "the bag with its counts is the one shown"
        );
    }

    #[test]
    fn a_pick_on_the_battle_list_stages_its_stack_and_presses_the_originals_use() {
        let (mut driver, mut menu) = battle_with_bag();
        open_battle_list(&mut driver, &mut menu);
        assert_eq!(menu.list.first().map(|entry| entry.0), Some(2), "Bulma is the first row");
        // The HD cursor on the third row, Yajirobe: A chooses it (Use or Drop), and A on Use is the pick.
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Down, &menu.wram());
        assert_eq!(driver.nav().cursor(), 2);
        driver.key(Key::Confirm, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert!(driver.busy(), "the use runs as a plan");
        for _ in 0..200 {
            let frame = driver.frame(&menu.wram(), None, Joypad::default());
            menu.tick(frame.pad);
            menu.absorb(&frame.events);
            if menu.step == 2 {
                break;
            }
        }
        assert_eq!(menu.step, 2, "the original's A on its row is the Use or Drop choice");
        assert_eq!(
            menu.list.first().map(|entry| entry.0),
            Some(18),
            "the picked stack is staged first"
        );
        assert_eq!(menu.list.len(), 3, "the staging keeps every stack");
    }

    /// The frames until the driver has no plan running, with the keeper's list writes taken by the fake menu (as the
    /// game takes them), so the original's list follows the staging.
    fn battle_settle(driver: &mut Driver, menu: &mut FakeMenu) {
        for _ in 0..2_000 {
            let frame = driver.frame(&menu.wram(), None, Joypad::default());
            menu.tick(frame.pad);
            menu.absorb(&frame.events);
            if !driver.busy() {
                return;
            }
        }
        panic!("the plan did not end");
    }

    #[test]
    fn a_use_on_the_battle_list_takes_the_unit_and_gives_the_battle_its_command_back() {
        let (mut driver, mut menu) = battle_with_bag();
        open_battle_list(&mut driver, &mut menu);
        // Yajirobe, as in the pick test: the staged row is used by the original's own Use; the effect's A is the player's.
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        battle_settle(&mut driver, &mut menu);
        assert_eq!(driver.last_outcome(), Some(Outcome::Reached));
        assert_eq!(
            (driver.curtain(), menu.screen),
            (Curtain::Lifted, 21),
            "the effect waits for the player's A on the original's screen"
        );
        frame(&mut driver, &mut menu, Joypad { a: true, ..Joypad::default() });
        frame(&mut driver, &mut menu, Joypad { a: true, ..Joypad::default() });
        for _ in 0..30 {
            frame(&mut driver, &mut menu, Joypad::default());
        }
        assert_eq!((driver.curtain(), menu.screen), (Curtain::Down, 17));
        assert_eq!(driver.bag().map(|bag| bag.units_of(18)), Some(0), "the used unit is gone");
        assert_eq!(driver.bag().map(|bag| bag.units_of(2)), Some(2), "the other stacks stay");
    }

    #[test]
    fn a_use_from_a_list_of_stacks_does_not_wait_on_presses_the_original_ignored() {
        // The original ignores a press within three frames of the one before: the plan's Up after its Down, and its A after
        // the Up, must come a few frames apart, or each lost press costs a long wait for nothing.
        let (mut driver, mut menu) = battle_with_bag();
        open_battle_list(&mut driver, &mut menu);
        menu.strict = true;
        // Yajirobe, the third row.
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        let mut frames = 0;
        while driver.busy() {
            let frame = driver.frame(&menu.wram(), None, Joypad::default());
            menu.tick(frame.pad);
            menu.absorb(&frame.events);
            frames += 1;
            assert!(frames < 2_000, "the plan did not end");
        }
        assert_eq!(driver.last_outcome(), Some(Outcome::Reached));
        assert!(frames < 120, "the use took {frames} frames: a press was lost and waited for");
    }

    #[test]
    fn back_from_the_battle_list_gives_the_battle_its_keys_back_on_its_command_window() {
        let (mut driver, mut menu) = battle_with_bag();
        open_battle_list(&mut driver, &mut menu);
        choose(&mut driver, &mut menu, Key::Cancel);
        assert_eq!(driver.curtain(), Curtain::Down, "the battle has its keys again");
        assert_eq!(menu.screen, 17, "the original is on the battle's command window");
        assert!(!driver.state.battle_items);
        assert_eq!(driver.last_outcome(), Some(Outcome::Reached));
    }

    #[test]
    fn an_object_a_battle_cannot_use_is_refused_on_the_battle_list() {
        let (mut driver, mut menu) = battle_with_bag();
        open_battle_list(&mut driver, &mut menu);
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.notice(), Some("This cannot be used or dropped here."));
        assert!(!driver.busy(), "nothing is staged for a refused object");
        assert_eq!(menu.step, 1, "the original's list is not touched");
    }

    /// The battle labels with the story's Gohan (26) as the catalogue has him: attribute `$25`, a story object that
    /// cannot be dropped.
    fn gohan_labels() -> Labels {
        let mut labels = battle_labels();
        labels.objects[26] = ObjectLabel {
            name: "Gohan".to_owned(),
            attribute: 0x25,
            story: true,
            fixed: true,
            ..ObjectLabel::default()
        };
        labels
    }

    /// A driver bound to a bag of Gohan and a Bulma, and the fake menu in a battle of the encounter group `group`, on
    /// its command window, with the battle's Item list open under the HD's Items screen. The HD cursor is on Gohan.
    fn gohan_on_the_battle_list(group: u8) -> (Driver, FakeMenu) {
        let mut bag = Bag::new(Rules::default());
        bag.add(26, 0, ssd_menus::bag::Class::of(26, true)).unwrap();
        bag.add(2, 0, ssd_menus::bag::Class::of(2, false)).unwrap();
        let mut driver = Driver::new(gohan_labels());
        let mut menu = FakeMenu::default();
        menu.battle = true;
        menu.field = 2;
        menu.screen = 17;
        menu.group = group;
        let events = driver.replay_event(BIND_TAG, &bag.to_bytes(), &menu.wram()).unwrap();
        menu.absorb(&events);
        open_battle_list(&mut driver, &mut menu);
        let stacks = driver.state.bag_view(&gohan_labels(), group).unwrap().stacks;
        let row = stacks.iter().position(|stack| stack.object == 26).expect("Gohan is listed");
        for _ in 0..row {
            driver.key(Key::Down, &menu.wram());
        }
        (driver, menu)
    }

    /// Whether the Items screen shows Gohan as usable in a battle of the encounter group `group`.
    fn gohan_shown_usable(driver: &Driver, group: u8) -> bool {
        let view = driver.state.bag_view(&gohan_labels(), group).unwrap();
        view.stacks.iter().find(|stack| stack.object == 26).unwrap().usable
    }

    #[test]
    fn gohan_is_usable_on_the_list_of_the_first_raditz_battle() {
        let (mut driver, mut menu) = gohan_on_the_battle_list(5);
        assert!(gohan_shown_usable(&driver, 5), "01:C535 gives him INT in encounter group 5");
        assert!(
            driver.state.selected(&gohan_labels(), 5).is_some_and(|selected| selected.battle),
            "his box shows the battle arrow"
        );
        driver.key(Key::Confirm, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.notice(), None, "the use is not refused");
        assert!(driver.busy(), "the use runs as a plan");
        battle_settle(&mut driver, &mut menu);
        assert_eq!(driver.last_outcome(), Some(Outcome::Reached));
        assert_eq!(
            (driver.curtain(), menu.screen),
            (Curtain::Lifted, 21),
            "the original's own Use took him: its effect waits on the battle's action screen"
        );
    }

    #[test]
    fn gohan_stays_greyed_out_on_the_list_of_another_battle() {
        for group in [0, 4, 6] {
            let (mut driver, menu) = gohan_on_the_battle_list(group);
            assert!(!gohan_shown_usable(&driver, group), "group {group}: a story object");
            driver.key(Key::Confirm, &menu.wram());
            assert_eq!(
                driver.notice(),
                Some("This cannot be used or dropped here."),
                "group {group}"
            );
            assert!(!driver.busy(), "group {group}: nothing is staged");
        }
    }

    #[test]
    fn a_battle_uses_the_objects_with_its_bit_and_no_story_object() {
        let labels = battle_labels();
        assert!(usable_in_battle(label(&labels, 2)), "Bulma works in a battle");
        assert!(!usable_in_battle(label(&labels, 16)), "the Scouter is for the map");
        assert!(usable_in_battle(label(&labels, 18)), "Yajirobe works in a battle");
        assert!(
            !usable_in_battle(label(&labels, 33)),
            "a story object is never used from the menu"
        );
    }

    #[test]
    fn talk_lifts_the_curtain_and_the_live_keys_reach_the_game_until_the_field_is_back() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.curtain(), Curtain::Lifted, "the original draws its message");
        assert!(driver.scene(&menu.wram()).is_none(), "the HD draws nothing while lifted");
        let a = Joypad { a: true, ..Joypad::default() };
        assert_eq!(frame(&mut driver, &mut menu, a), a, "the live A dismisses the message");
        frame(&mut driver, &mut menu, Joypad::default());
        assert_eq!(driver.curtain(), Curtain::Down, "the field is free again");
    }

    #[test]
    fn a_talk_whose_message_holds_the_field_is_taken_at_once_and_the_hd_window_goes_down_with_the_press()
     {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        menu.hold = true;
        open_by_player(&mut driver, &mut menu);
        driver.key(Key::Confirm, &menu.wram());
        // The press goes under a lifted curtain: the HD window is not drawn while the plan runs.
        for frames in 1..=12 {
            frame(&mut driver, &mut menu, Joypad::default());
            assert!(driver.scene(&menu.wram()).is_none(), "no HD window at frame {frames}");
            if !driver.busy() {
                break;
            }
        }
        assert!(
            !driver.busy(),
            "the plan ends within a few frames, as the original takes the press"
        );
        assert_eq!(menu.field, 5, "the message holds the field");
        assert_eq!(driver.curtain(), Curtain::Lifted);
        assert_eq!(driver.last_outcome(), Some(Outcome::Reached));
    }

    #[test]
    fn fly_hides_the_hd_window_once_the_cursor_is_on_its_row() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        let mut shown = 0;
        while driver.busy() {
            frame(&mut driver, &mut menu, Joypad::default());
            // The last Down is held for three frames under the HD window; then the curtain lifts.
            if menu.cursor == 2 && driver.scene(&menu.wram()).is_some() {
                shown += 1;
            }
        }
        assert!(shown <= 3, "the window is down once the Down press is over ({shown} frames)");
        assert_eq!(driver.curtain(), Curtain::Lifted);
    }

    #[test]
    fn the_menu_list_opens_the_cheats_and_the_options_without_a_press_to_the_original() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Menu));
        assert_eq!(menu.screen, 4, "the original's Menu list");
        for _ in 0..5 {
            driver.key(Key::Down, &menu.wram());
        }
        assert_eq!(driver.nav().cursor(), 5, "Cheats");
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Cheats));
        assert_eq!(menu.screen, 4, "nothing was sent to the original: its Menu list stays under");
        assert!(!driver.busy(), "no plan for Cheats");
        choose(&mut driver, &mut menu, Key::Cancel);
        let at = (driver.curtain(), driver.nav().screen(), driver.nav().cursor());
        assert_eq!(at, (Curtain::Up, Screen::Menu, 5), "back on the Menu list, on Cheats");
        assert_eq!(menu.screen, 4);
        driver.key(Key::Down, &menu.wram());
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Options));
        assert_eq!(menu.screen, 4, "nor for Options: the original's Menu list stays under");
        assert!(!driver.busy(), "no plan for Options");
    }

    #[test]
    fn the_cheats_screen_writes_its_choices_into_the_cheat_block_before_the_next_frame() {
        use ssd_menus::cheats::{BLOCK_LEN, Multiplier, Setting};
        let mut driver = Driver::new(Labels::default()).with_cheats(CheatSidecar::empty());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        for _ in 0..5 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.nav().screen(), Screen::Cheats);
        driver.key(Key::Right, &menu.wram());
        assert_eq!(driver.cheats().multiplier(Setting::Damage), Multiplier::new(2, 1));
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        let block = frame.events.iter().find_map(|event| match event {
            HostEvent::Wram { address, bytes } if *address == crate::cheats::BLOCK => {
                Some(bytes.clone())
            }
            _ => None,
        });
        let block = block.expect("the block is written for the change");
        assert_eq!(block.len(), BLOCK_LEN);
        assert_eq!(&block[..2], &[2, 1], "damage x2 is the first pair of the block");
        driver.key(Key::Up, &menu.wram());
        assert_eq!(driver.nav().cheats_row(), ssd_menus::cheats::Row::ResetAll, "the rows wrap");
        driver.key(Key::Confirm, &menu.wram());
        assert!(driver.cheats().is_neutral(), "Reset all turns every cheat off");
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert!(frame.events.iter().any(|event| matches!(
            event,
            HostEvent::Wram { address, bytes } if *address == crate::cheats::BLOCK && bytes[..2] == [1, 1]
        )), "the reset block has the neutral pairs");
    }

    #[test]
    fn cards_keeps_the_curtain_up_for_the_hd_hand_over_the_original_screen() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Cards));
        assert_eq!(menu.screen, 6, "the original's Cards");
    }

    /// Work RAM of the fake menu with a party of `count` members (character ids 0 up) in the records and the order.
    fn with_party(menu: &FakeMenu, count: u8) -> Vec<u8> {
        let mut ram = menu.wram();
        let at = |address: u32| (address - 0x7E_0000) as usize;
        for slot in 0..count {
            let start = at(d::PARTY_RECORDS) + 32 * usize::from(slot);
            ram[start] = 0xC0;
            ram[start + 1] = slot;
            ram[start + 2] = 10 + slot;
            ram[start + 0x1C] = slot;
            ram[at(d::PARTY_ORDER) + usize::from(slot)] = slot;
        }
        ram[at(d::PARTY_ORDER) + usize::from(count)] = 0xFF;
        ram
    }

    /// The Menu list's Status on the fake menu: the HD screen opens over the original's page, Up and Down move the member
    /// without a press of the original, B goes back to the Menu list on Status, and Start closes the menu.
    #[test]
    fn status_is_the_hds_own_screen_over_the_originals_page_and_b_and_start_leave_it() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        driver.key(Key::Down, &menu.wram());
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Status));
        assert_eq!(menu.screen, 7, "the original runs its Status page 1 under the HD screen");
        // The party of three: Down moves over it with no press of the original, and wraps.
        let ram = with_party(&menu, 3);
        for want in [1, 2, 0] {
            driver.key(Key::Down, &ram);
            assert_eq!(driver.nav().cursor(), want);
            assert!(!driver.busy(), "moving the member is the HD's alone");
        }
        assert_eq!(menu.screen, 7);
        let scene = driver.scene(&ram).expect("the HD draws Status");
        assert!(scene.texts().any(|(_, _, text)| text == "3/9"), "the header counts the party");
        // B goes back to the Menu list, with its cursor on Status.
        choose(&mut driver, &mut menu, Key::Cancel);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Menu));
        assert_eq!(driver.nav().cursor(), 1, "the Menu list keeps its cursor on Status");
        assert_eq!(menu.screen, 4, "the original is back on its Menu list");
        // Again, and Start closes the menu from it with the original's own B presses.
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.nav().screen(), Screen::Status);
        assert_eq!(driver.nav().cursor(), 0, "Status opens on the first member");
        driver.close_menu();
        settle(&mut driver, &mut menu);
        assert_eq!(driver.curtain(), Curtain::Down, "Start closes the menu");
        assert_eq!(menu.field, 0, "the field is free");
        assert_eq!(driver.last_outcome(), Some(Outcome::Reached));
    }

    /// Start does nothing on the other HD screens: the original's menu is not closed by a stray Start.
    #[test]
    fn start_does_nothing_off_the_status_screen() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        driver.close_menu();
        settle(&mut driver, &mut menu);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Command));
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        driver.close_menu();
        settle(&mut driver, &mut menu);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Menu));
        assert_eq!(menu.field, 1, "the menu is still open");
    }

    #[test]
    fn fly_takes_off_and_the_take_off_closes_the_menu() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Down, &menu.wram());
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.curtain(), Curtain::Lifted, "the take-off is the original's");
        for _ in 0..200 {
            frame(&mut driver, &mut menu, Joypad::default());
        }
        assert!(menu.flying, "the party flies");
        assert_eq!(driver.curtain(), Curtain::Down);
    }

    #[test]
    fn without_a_save_or_a_list_the_bag_stays_unbound_and_the_list_is_the_originals() {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_by_player(&mut driver, &mut menu);
        let frame_out = driver.frame(&menu.wram(), None, Joypad::default());
        assert!(frame_out.events.is_empty() && frame_out.saved.is_none());
        assert!(driver.bag().is_none());
    }

    #[test]
    fn held_buttons_are_the_joypad_of_a_press() {
        assert!(held(Button::A).a);
        assert!(!held(Button::B).down);
    }

    #[test]
    fn the_use_plan_stages_one_unit_presses_use_and_leaves_the_rest_to_the_player() {
        let steps = use_steps(7);
        let stages = steps.iter().filter(|s| matches!(s, Step::Stage { id: 7 })).count();
        assert_eq!(stages, 1, "one unit is staged: the use takes one unit, no quantity");
        let choice =
            steps.iter().position(|s| matches!(s, Step::Press { until: Until::ItemChoice, .. }));
        let lift = steps.iter().position(|s| matches!(s, Step::Lift));
        assert!(
            choice.is_some() && lift.is_some() && choice < lift,
            "the curtain lifts on the Use or Drop window"
        );
        assert!(
            matches!(steps.last(), Some(Step::Press { button: Button::A, until: Until::UseTaken })),
            "the Use is the plan's last press"
        );
        assert!(
            !steps.iter().any(|s| matches!(s, Step::MemberIf { .. } | Step::Answer)),
            "the fighter, the card, the messages are the player's"
        );
    }

    #[test]
    fn the_drop_plan_drops_one_unit_and_keeps_the_list_open() {
        let steps = drop_steps(3);
        let stages = steps.iter().filter(|s| matches!(s, Step::Stage { id: 3 })).count();
        assert_eq!(stages, 1, "one unit is staged");
        assert!(!steps.iter().any(|s| matches!(s, Step::Press { until: Until::Closed, .. })));
        assert!(
            !steps.iter().any(|s| matches!(s, Step::Lift)),
            "the HD's own Yes or No asked already"
        );
    }

    /// A battery save whose `slots` are good saves of the list the fake menu shows (three entries of object 0,
    /// no zeni), each with its mark and its checksum word.
    fn saved_slots(slots: &[usize]) -> Vec<u8> {
        let mut sram = vec![0_u8; ssd_save::SRAM_SIZE];
        for &slot in slots {
            let base = ssd_save::SLOT_BASES[slot];
            let count = base + ssd_save::IMAGE_LEN + (crate::data::ITEM_COUNT - 0x7E_0100) as usize;
            sram[count] = 3;
            sram[base + ssd_save::MARK_OFFSET..][..ssd_save::MARK.len()]
                .copy_from_slice(&ssd_save::MARK);
            let bytes: [u8; ssd_save::SLOT_LEN] =
                sram[base..base + ssd_save::SLOT_LEN].try_into().expect("a slot");
            let at = base + ssd_save::CHECKSUM_OFFSET;
            sram[at..at + 2].copy_from_slice(&ssd_save::checksum(&bytes).to_le_bytes());
        }
        sram
    }

    /// The checksum word of `slot` in `sram`.
    fn checksum_of_slot(sram: &[u8], slot: usize) -> u16 {
        let at = ssd_save::SLOT_BASES[slot] + ssd_save::CHECKSUM_OFFSET;
        u16::from_le_bytes([sram[at], sram[at + 1]])
    }

    /// The game's Save into `slot` (0 to 2) from its list: the list shows with its cursor on the slot, then the
    /// completion, where the cursor is back at 0. Returns the frame of the completion.
    fn save_into(driver: &mut Driver, menu: &mut FakeMenu, sram: &[u8], slot: u8) -> Frame {
        menu.screen = SAVE_SCREEN;
        menu.step = SAVE_LIST_STEP;
        menu.cursor = slot;
        driver.frame(&menu.wram(), Some(sram), Joypad::default());
        menu.step = SAVE_DONE_STEP;
        menu.cursor = 0;
        driver.frame(&menu.wram(), Some(sram), Joypad::default())
    }

    #[test]
    fn the_cheats_open_over_a_battle_write_the_settings_only_and_back_gives_the_battle_its_keys() {
        let sram = saved_slots(&[0]);
        let mut driver = Driver::new(Labels::default()).with_cheats(CheatSidecar::empty());
        // A battle: the field is not free (`field_state` 3, as the game sets it for one).
        let mut menu = FakeMenu::default();
        menu.field = 3;
        driver.frame(&menu.wram(), Some(&sram), Joypad::default());
        assert!(driver.open_battle_cheats());
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Cheats));
        assert!(!driver.open_battle_cheats(), "once is enough");
        driver.key(Key::Right, &menu.wram());
        let frame = driver.frame(&menu.wram(), Some(&sram), Joypad::default());
        let block: Vec<usize> = frame
            .events
            .iter()
            .filter_map(|event| match event {
                HostEvent::Wram { address, bytes } if *address == crate::cheats::BLOCK => {
                    Some(bytes.len())
                }
                _ => None,
            })
            .collect();
        assert_eq!(block, [ssd_menus::cheats::SETTINGS_LEN], "the runtime bytes are the battle's");
        driver.key(Key::Cancel, &menu.wram());
        assert_eq!(driver.curtain(), Curtain::Down);
        assert!(!driver.battle_cheats());
        // Open again, and the battle ends under it (the field is free): the curtain and the flag go.
        assert!(driver.open_battle_cheats());
        menu.field = 0;
        driver.frame(&menu.wram(), Some(&sram), Joypad::default());
        assert_eq!(driver.curtain(), Curtain::Down);
        assert!(!driver.battle_cheats(), "a change on the map writes the whole block again");
    }

    #[test]
    fn the_item_spawner_puts_units_in_the_bag_marks_the_game_and_replays() {
        let sram = saved_slots(&[0]);
        let (mut driver, menu) = driver_on_cheats(&sram);
        assert!(driver.state.keeper.bag().is_some(), "the bag is bound");
        while driver.nav().cheats_row() != ssd_menus::cheats::Row::ItemSpawner {
            driver.key(Key::Down, &menu.wram());
        }
        driver.key(Key::Confirm, &menu.wram());
        assert!(driver.nav().spawner().is_some());
        // Object 3, two units.
        for _ in 0..3 {
            driver.key(Key::Down, &menu.wram());
        }
        driver.key(Key::Right, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        let units = |driver: &Driver| driver.state.keeper.bag().unwrap().units_of(3);
        assert_eq!(units(&driver), 2);
        assert!(driver.cheats().spawned(), "the game is marked");
        let frame = driver.frame(&menu.wram(), Some(&sram), Joypad::default());
        let spawn = frame.events.iter().find_map(|event| match event {
            HostEvent::App { tag, payload } if *tag == SPAWN_TAG => Some(payload.clone()),
            _ => None,
        });
        assert_eq!(spawn, Some(vec![3, 2]), "the replay gets the object and the count");
        // Reset all keeps the mark.
        driver.key(Key::Cancel, &menu.wram());
        while driver.nav().cheats_row() != ssd_menus::cheats::Row::ResetAll {
            driver.key(Key::Down, &menu.wram());
        }
        driver.key(Key::Confirm, &menu.wram());
        assert!(driver.cheats().is_neutral() && driver.cheats().spawned());
        // A replay of the event adds the same units.
        let before = units(&driver);
        driver.replay_event(SPAWN_TAG, &[3, 2], &menu.wram()).unwrap();
        assert_eq!(units(&driver), before + 2);
    }

    #[test]
    fn the_cheats_do_not_open_over_a_battle_without_the_cheats_installed() {
        let mut driver = Driver::new(Labels::default());
        assert!(!driver.open_battle_cheats());
        assert_eq!(driver.curtain(), Curtain::Down);
    }

    /// A driver with the cheats installed, on the Cheats screen of an open menu (the bag and the cheats are bound
    /// to the battery save `sram` by its first frame).
    fn driver_on_cheats(sram: &[u8]) -> (Driver, FakeMenu) {
        let mut driver = Driver::new(Labels::default()).with_cheats(CheatSidecar::empty());
        let mut menu = FakeMenu::default();
        menu.field = 1;
        menu.screen = 4;
        driver.state.curtain = Curtain::Up;
        driver.state.nav.enter(Screen::Cheats);
        driver.frame(&menu.wram(), Some(sram), Joypad::default());
        (driver, menu)
    }

    #[test]
    fn a_save_of_the_same_data_writes_the_cheats_chosen_since_the_last_save() {
        use ssd_menus::cheats::Row;
        let sram = saved_slots(&[0]);
        let checksum = checksum_of_slot(&sram, 0);
        let (mut driver, mut menu) = driver_on_cheats(&sram);
        driver.key(Key::Right, &menu.wram());
        let damage = *driver.cheats();
        let first = save_into(&mut driver, &mut menu, &sram, 0);
        assert_eq!(
            first.saved_cheats,
            Some(SavedCheats { slot: 0, checksum, marked: true, cheats: damage })
        );
        for _ in 0..20 {
            if driver.nav().cheats_row() == Row::RandomBattles {
                break;
            }
            driver.key(Key::Down, &menu.wram());
        }
        driver.key(Key::Right, &menu.wram());
        let both = *driver.cheats();
        let second = save_into(&mut driver, &mut menu, &sram, 0);
        assert!(second.saved.is_none(), "the bag is unchanged, the data being the same");
        assert_eq!(
            second.saved_cheats,
            Some(SavedCheats { slot: 0, checksum, marked: true, cheats: both }),
            "a save of the same data writes the cheats it is made with"
        );
        assert_eq!(driver.cheat_sidecar().entry_for(0, checksum).map(|s| s.cheats), Some(both));
        assert_eq!(driver.cheat_sidecar().saves.len(), 1, "the slot has one entry, replaced");
    }

    #[test]
    fn a_save_to_another_slot_writes_that_slot_and_the_run_keeps_its_own_save() {
        let sram = saved_slots(&[0, 1]);
        let checksum = checksum_of_slot(&sram, 1);
        let (mut driver, mut menu) = driver_on_cheats(&sram);
        assert_eq!(driver.state.keeper.bound_save(), Some((0, checksum_of_slot(&sram, 0))));
        driver.key(Key::Right, &menu.wram());
        let damage = *driver.cheats();
        let other = save_into(&mut driver, &mut menu, &sram, 1);
        assert_eq!(
            other.saved_cheats,
            Some(SavedCheats { slot: 1, checksum, marked: true, cheats: damage })
        );
        assert!(driver.cheat_sidecar().entry_for(0, checksum_of_slot(&sram, 0)).is_none());
        driver.frame(&menu.wram(), Some(&sram), Joypad::default());
        assert_eq!(*driver.cheats(), damage, "the run keeps its cheats on the save it is bound to");
    }

    #[test]
    fn a_change_of_the_cheats_is_recorded_and_a_replay_gives_them_to_its_run() {
        let mut driver = Driver::new(Labels::default()).with_cheats(CheatSidecar::empty());
        let mut menu = FakeMenu::default();
        menu.field = 1;
        menu.screen = 4;
        driver.state.curtain = Curtain::Up;
        driver.state.nav.enter(Screen::Cheats);
        driver.key(Key::Right, &menu.wram());
        let events = driver.frame(&menu.wram(), None, Joypad::default()).events;
        let payload = events.iter().find_map(|event| match event {
            HostEvent::App { tag, payload } if *tag == CHEATS_TAG => Some(payload.clone()),
            _ => None,
        });
        let payload = payload.expect("the change is recorded as an application event");
        let mut played = Driver::new(Labels::default()).with_cheats(CheatSidecar::empty());
        played.replay_event(CHEATS_TAG, &payload, &menu.wram()).expect("a cheats file");
        assert_eq!(played.cheats(), driver.cheats(), "the replay's run has the recorded cheats");
        assert!(played.replay_event(CHEATS_TAG, &[0xFF; 3], &menu.wram()).is_err());
    }

    /// A mod state for the tests: the extended flags all `flag`, the high parts all `high`.
    fn mod_state(flag: u8, high: u8) -> ModState {
        ModState { flags: Some(vec![flag; FLAG_BYTES]), high: Some(vec![high; HIGH_BYTES]) }
    }

    /// Writes the writes of `events` into `wram` (a work RAM slice, index `address - $7E:0000`).
    fn write_into(wram: &mut [u8], events: &[HostEvent]) {
        for event in events {
            if let HostEvent::Wram { address, bytes } = event {
                let at = (*address - 0x7E_0000) as usize;
                wram[at..at + bytes.len()].copy_from_slice(bytes);
            }
        }
    }

    /// The work RAM of `menu` with `state` in it, its markers included: the run has the state.
    fn with_mod_state(menu: &FakeMenu, state: &ModState) -> Vec<u8> {
        let mut wram = menu.wram();
        write_into(&mut wram, &apply_events(state));
        wram
    }

    #[test]
    fn a_save_of_a_run_with_mod_markers_keeps_its_mod_state_for_the_slot_and_reports_it() {
        let sram = saved_slots(&[0]);
        let checksum = checksum_of_slot(&sram, 0);
        let (mut driver, mut menu) = driver_on_cheats(&sram);
        let state = mod_state(0x5A, 0x3C);
        menu.screen = SAVE_SCREEN;
        menu.step = SAVE_LIST_STEP;
        driver.frame(&with_mod_state(&menu, &state), Some(&sram), Joypad::default());
        menu.step = SAVE_DONE_STEP;
        let frame = driver.frame(&with_mod_state(&menu, &state), Some(&sram), Joypad::default());
        assert_eq!(frame.saved_mods, Some((0, checksum)), "the Save reports its slot for the file");
        assert_eq!(driver.mods_sidecar().entry_for(0, checksum), Some(&state));
    }

    #[test]
    fn a_save_of_a_run_without_mod_markers_keeps_no_entry_and_the_file_drops_the_old_one() {
        let sram = saved_slots(&[0]);
        let checksum = checksum_of_slot(&sram, 0);
        let (mut driver, mut menu) = driver_on_cheats(&sram);
        driver.state.mods_sidecar.store(0, checksum, mod_state(0x5A, 0x3C));
        let frame = save_into(&mut driver, &mut menu, &sram, 0);
        assert!(driver.mods_sidecar().saves.is_empty(), "a run with no markers keeps nothing");
        assert_eq!(
            frame.saved_mods,
            Some((0, checksum)),
            "the file is written, so the old entry goes"
        );
    }

    #[test]
    fn a_bind_gives_the_save_s_recorded_mod_state_and_a_replay_writes_the_same_bytes() {
        let sram = saved_slots(&[0]);
        let checksum = checksum_of_slot(&sram, 0);
        let state = mod_state(0x5A, 0x3C);
        let mut sidecar = ModsSidecar::empty();
        sidecar.store(0, checksum, state.clone());
        let mut driver = Driver::new(Labels::default()).with_mods(sidecar);
        let mut menu = FakeMenu::default();
        menu.field = 1;
        menu.screen = 4;
        let events = driver.frame(&menu.wram(), Some(&sram), Joypad::default()).events;
        let payload = events.iter().find_map(|event| match event {
            HostEvent::App { tag, payload } if *tag == MODS_TAG => Some(payload.clone()),
            _ => None,
        });
        let payload = payload.expect("the bind records the mod state as an application event");
        let writes = driver.replay_event(MODS_TAG, &payload, &menu.wram()).expect("a mod state");
        assert_eq!(writes, apply_events(&state), "the replay writes what the run applied");
        let mut wram = menu.wram();
        write_into(&mut wram, &writes);
        assert_eq!(capture(&wram), state, "the replay puts the same bytes into work RAM");
        assert!(driver.replay_event(MODS_TAG, &[0xFF; 3], &menu.wram()).is_err());
    }

    #[test]
    fn a_bound_save_without_an_entry_clears_the_markers_only_where_they_are_set() {
        let sram = saved_slots(&[0]);
        let checksum = checksum_of_slot(&sram, 0);
        let mut menu = FakeMenu::default();
        menu.field = 1;
        menu.screen = 4;
        let bind = |wram: &[u8], sidecar: ModsSidecar| {
            let mut driver = Driver::new(Labels::default()).with_mods(sidecar);
            driver.frame(wram, Some(&sram), Joypad::default()).events
        };
        let no_markers = bind(&menu.wram(), ModsSidecar::empty());
        assert!(
            !no_markers.iter().any(
                |event| matches!(event, HostEvent::Wram { address, .. } if *address == MARKER_AT)
            ),
            "no marker is set, so nothing is written to it"
        );
        let markers = bind(&with_mod_state(&menu, &mod_state(0, 0)), ModsSidecar::empty());
        assert!(markers.contains(&HostEvent::Wram { address: MARKER_AT, bytes: vec![0; 4] }));
        assert!(markers.contains(&HostEvent::Wram { address: FLAGS_MARKER_AT, bytes: vec![0; 4] }));
        // A save with its flags and no high parts: the block's marker is cleared, so the run's high parts read neutral.
        let mut sidecar = ModsSidecar::empty();
        sidecar.store(0, checksum, ModState { flags: Some(vec![1; FLAG_BYTES]), high: None });
        let partial = bind(&with_mod_state(&menu, &mod_state(0, 0x3C)), sidecar);
        assert!(partial.contains(&HostEvent::Wram { address: MARKER_AT, bytes: vec![0; 4] }));
        assert!(
            !partial
                .iter()
                .any(|event| matches!(event, HostEvent::Wram { address, .. } if *address == BLOCK)),
            "the save keeps no high parts, so none are written"
        );
    }

    /// A driver on the Options screen of an open menu, bound to `sram`'s first frame.
    fn driver_on_options(sram: &[u8]) -> (Driver, FakeMenu) {
        let mut driver = Driver::new(Labels::default()).with_cheats(CheatSidecar::empty());
        let mut menu = FakeMenu::default();
        menu.field = 1;
        menu.screen = 4;
        driver.state.curtain = Curtain::Up;
        driver.state.nav.enter(Screen::Options);
        driver.frame(&menu.wram(), Some(sram), Joypad::default());
        (driver, menu)
    }

    #[test]
    fn the_options_text_speed_row_writes_the_games_text_speed() {
        let (mut driver, menu) = driver_on_options(&saved_slots(&[0]));
        assert_eq!(driver.nav().options_category(), Category::Difficulty);
        driver.key(Key::Tab, &menu.wram());
        assert_eq!(driver.nav().options_category(), Category::Gameplay, "R changes the page");
        driver.key(Key::TabBack, &menu.wram());
        assert_eq!(driver.nav().options_category(), Category::Difficulty, "L goes back");
        driver.key(Key::Tab, &menu.wram());
        driver.key(Key::Left, &menu.wram());
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert!(
            frame.events.contains(&HostEvent::Wram { address: d::TEXT_SPEED, bytes: vec![1] }),
            "text speed byte 0 (the fastest) one step slower is 1: {:?}",
            frame.events
        );
        assert!(
            frame.settings.is_none() && !frame.saved_options,
            "it is the game's value, nothing is stored"
        );
    }

    /// The Text speed row of the Gameplay page as the Options screen shows it now (its index in the row's choices).
    fn options_text_speed_index(driver: &Driver, menu: &FakeMenu) -> usize {
        let wram = menu.wram();
        let probe = MenuProbe::new(&wram, &driver.labels);
        driver.state.options.indices(Category::Gameplay, &driver.state.cheats, probe.text_speed())
            [0]
    }

    #[test]
    fn the_text_screen_and_the_options_text_speed_row_are_one_setting_the_games_byte() {
        // The Options row writes the game's byte, and the Text screen opens on it.
        for speed in 0..5_u8 {
            let (mut driver, mut menu) = driver_on_options(&saved_slots(&[0]));
            menu.text_speed = speed;
            assert_eq!(options_text_speed_index(&driver, &menu), usize::from(4 - speed));
            driver.key(Key::Tab, &menu.wram());
            driver.key(Key::Right, &menu.wram());
            let frame = driver.frame(&menu.wram(), None, Joypad::default());
            let written = frame.events.iter().find_map(|event| match event {
                HostEvent::Wram { address, bytes } if *address == d::TEXT_SPEED => bytes.first(),
                _ => None,
            });
            // One step to the right is one step faster (the row lists the slowest first), and the row wraps.
            let faster = if speed == 0 { 4 } else { speed - 1 };
            assert_eq!(written, Some(&faster), "speed {speed}");
            menu.text_speed = faster;
            driver.key(Key::Cancel, &menu.wram());
            for _ in 0..3 {
                driver.key(Key::Down, &menu.wram());
            }
            driver.key(Key::Confirm, &menu.wram());
            assert_eq!(driver.nav().screen(), Screen::Text);
            assert_eq!(
                driver.nav().text().speed(),
                faster,
                "the Text screen opens on the row's speed"
            );
        }
        // The Text screen keeps a speed through the original's own screen, and the row shows it.
        for target in 0..5_u8 {
            let (mut driver, mut menu) = driver_on_options(&saved_slots(&[0]));
            menu.text_speed = 2;
            driver.key(Key::Cancel, &menu.wram());
            for _ in 0..3 {
                driver.key(Key::Down, &menu.wram());
            }
            driver.key(Key::Confirm, &menu.wram());
            for _ in 0..4 {
                driver.key(if target > 2 { Key::Right } else { Key::Left }, &menu.wram());
            }
            driver.key(Key::Confirm, &menu.wram());
            for _ in 0..2_000 {
                let pad = driver.frame(&menu.wram(), None, Joypad::default()).pad;
                menu.tick(pad);
                if !driver.busy() {
                    break;
                }
            }
            let end = if target > 2 { 4 } else { 0 };
            assert_eq!(menu.text_speed, end, "the whole way to the end the keys went");
            assert_eq!(options_text_speed_index(&driver, &menu), usize::from(4 - end));
        }
    }

    #[test]
    fn the_gameplay_page_rows_reach_the_driver_as_stored_values_and_drive_the_auto_run_and_the_animation()
     {
        let (mut driver, menu) = driver_on_options(&saved_slots(&[0]));
        driver.key(Key::Tab, &menu.wram());
        // The rows: text speed, auto-run, wait, animation speed, encounters.
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Right, &menu.wram()); // Auto-run: Off to On
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Left, &menu.wram()); // Wait: Normal to Short
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Right, &menu.wram()); // Animation: Normal to x2
        let game = &driver.options().game;
        assert_eq!(game.get("gameplay.auto_run"), Some(1));
        assert_eq!(game.get("gameplay.auto_wait"), Some(0));
        assert_eq!(game.get("gameplay.anim_speed"), Some(1));
        // The curtain comes down: the player is back in the game, and a finished message waits for A.
        driver.state.curtain = Curtain::Down;
        let mut wram = menu.wram();
        wram[(d::MESSAGE_STATE - 0x7E_0000) as usize] = 0x83;
        let mut first = None;
        for frame in 0..100 {
            if driver.frame(&wram, None, Joypad::default()).pad.a {
                first = Some(frame);
                break;
            }
        }
        assert_eq!(first, Some(44), "the Short wait: 45 frames, then A");
        // The animation is sped up in the attack animation only.
        assert_eq!(driver.animation_factor(&wram), 1);
        wram[(d::MESSAGE_STATE - 0x7E_0000) as usize] = 0;
        wram[(d::IN_BATTLE - 0x7E_0000) as usize] = 0xFF;
        wram[(d::GAME_MODE - 0x7E_0000) as usize] = 4;
        wram[(d::BATTLE_PHASE - 0x7E_0000) as usize] = 4;
        assert_eq!(driver.animation_factor(&wram), 2, "x2 in the attack animation");
        // Reset this page puts the original back.
        driver.state.curtain = Curtain::Up;
        driver.key(Key::ResetPage, &wram);
        assert_eq!(driver.animation_factor(&wram), 1);
        assert_eq!(driver.options().game.get("gameplay.auto_run"), None);
    }

    #[test]
    fn the_encounter_row_is_the_cheats_row_and_the_same_cheat_block_is_written() {
        let (mut driver, menu) = driver_on_options(&saved_slots(&[0]));
        driver.key(Key::Tab, &menu.wram());
        driver.key(Key::Up, &menu.wram()); // wraps to the last row of the page: the Data packs button
        driver.key(Key::Up, &menu.wram()); // the row above it: the encounters
        driver.key(Key::Left, &menu.wram());
        assert_eq!(driver.cheats().index(ssd_menus::cheats::Setting::Encounters), 1, "x1 to x1/2");
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert!(frame.events.iter().any(|event| matches!(
            event,
            HostEvent::Wram { address, .. } if *address == crate::cheats::BLOCK
        )));
        driver.key(Key::ResetPage, &menu.wram());
        assert!(driver.cheats().is_neutral(), "Reset this page puts the encounter rate back to x1");
    }

    #[test]
    fn a_pc_option_is_handed_to_the_host_once_and_reset_all_asks_with_the_cursor_on_no() {
        let (mut driver, menu) = driver_on_options(&saved_slots(&[0]));
        for _ in 0..2 {
            driver.key(Key::Tab, &menu.wram());
        }
        assert_eq!(driver.nav().options_category(), Category::Video);
        // Display, Scale, V-Sync: V-Sync is applied and saved at once (it asks nothing).
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Right, &menu.wram());
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert_eq!(frame.settings.as_ref().and_then(|v| v.get("video.vsync")), Some(1));
        assert_eq!(frame.video, frame.settings, "and handed to the window");
        assert_eq!(driver.nav().options_video_confirm(), None);
        assert!(driver.frame(&menu.wram(), None, Joypad::default()).settings.is_none(), "once");
        driver.key(Key::ResetAll, &menu.wram());
        assert_eq!(driver.nav().options_confirm(), Some(1), "the cursor is on No");
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.options().pc.get("video.vsync"), Some(1), "No keeps the options");
        driver.key(Key::ResetAll, &menu.wram());
        driver.key(Key::Left, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.options().pc.get("video.vsync"), None, "Yes resets every option");
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert_eq!(frame.settings, Some(Values::new()));
    }

    #[test]
    fn the_controls_page_takes_a_free_key_refuses_a_used_one_and_clears_a_shortcut() {
        use ssd_menus::options::controls::{Capture, Device};
        let (mut driver, menu) = driver_on_options(&saved_slots(&[0]));
        let wram = menu.wram();
        for _ in 0..4 {
            driver.key(Key::Tab, &wram);
        }
        assert_eq!(driver.nav().options_category(), Category::Controls);
        assert_eq!(driver.listening(), None);
        // Down to Up, Right to A (confirm), A to wait.
        driver.key(Key::Down, &wram);
        driver.key(Key::Right, &wram);
        driver.key(Key::Confirm, &wram);
        assert_eq!(driver.listening(), Some(Device::Keyboard));
        // Z is B's key: refused, nothing stored, the row flashes and the strip says why.
        driver.capture(Capture::Code(29));
        assert_eq!(driver.listening(), None);
        assert_eq!(driver.options().pc.get("controls.kb.a"), None);
        let scene = driver.scene(&wram).unwrap();
        let texts: Vec<&str> = scene.texts().map(|(_, _, text)| text).collect();
        assert!(texts.contains(&"Key in use"), "{texts:?}");
        assert!(texts.contains(&"Key already used by B (back). Choose another key."), "{texts:?}");
        assert!(driver.frame(&wram, None, Joypad::default()).settings.is_none());
        // Esc cancels without a change; a free key (F) is taken and handed to the host once.
        driver.key(Key::Confirm, &wram);
        driver.capture(Capture::Cancel);
        assert_eq!(driver.listening(), None);
        assert_eq!(driver.options().pc.get("controls.kb.a"), None);
        driver.key(Key::Confirm, &wram);
        driver.capture(Capture::Code(9));
        assert_eq!(driver.options().pc.get("controls.kb.a"), Some(9));
        let frame = driver.frame(&wram, None, Joypad::default());
        assert_eq!(frame.settings.as_ref().and_then(|v| v.get("controls.kb.a")), Some(9));
        assert!(driver.frame(&wram, None, Joypad::default()).settings.is_none(), "once");
        let scene = driver.scene(&wram).unwrap();
        assert!(scene.texts().any(|(_, _, text)| text == "F"));
        // Backspace on a SNES button does nothing (it keeps waiting); on a shortcut it clears.
        driver.key(Key::Confirm, &wram);
        driver.capture(Capture::Clear);
        assert_eq!(driver.listening(), Some(Device::Keyboard));
        driver.capture(Capture::Cancel);
        for _ in 0..6 {
            driver.key(Key::Down, &wram);
        }
        driver.key(Key::Left, &wram);
        assert_eq!(driver.nav().controls_action(), Some(12), "Speed");
        driver.key(Key::Right, &wram);
        driver.key(Key::Confirm, &wram);
        driver.capture(Capture::Clear);
        assert_eq!(driver.options().pc.get("controls.kb.pause"), Some(0));
        assert_eq!(driver.listening(), None);
    }

    #[test]
    fn the_controls_device_follows_the_stored_value_and_reset_page_gives_back_todays_keys() {
        use ssd_menus::options::controls::{Capture, Device};
        let (mut driver, menu) = driver_on_options(&saved_slots(&[0]));
        let wram = menu.wram();
        for _ in 0..4 {
            driver.key(Key::Tab, &wram);
        }
        // Left and Right on the device row switch to the gamepad: its buttons show and wait for a button.
        driver.key(Key::Right, &wram);
        assert_eq!(driver.options().pc.get("controls.device"), Some(1));
        driver.key(Key::Down, &wram);
        assert_eq!(driver.nav().options_row(), 18, "the gamepad's first row");
        driver.key(Key::Confirm, &wram);
        assert_eq!(driver.listening(), Some(Device::Gamepad));
        driver.capture(Capture::Code(17)); // Left paddle 1: free
        assert_eq!(driver.options().pc.get("controls.pad.up"), Some(17));
        assert_eq!(driver.options().pc.get("controls.kb.up"), None, "the keyboard map is its own");
        driver.key(Key::ResetPage, &wram);
        assert!(driver.options().pc.get("controls.pad.up").is_none());
        assert!(driver.options().pc.get("controls.device").is_none());
        let frame = driver.frame(&wram, None, Joypad::default());
        assert_eq!(frame.settings, Some(Values::new()), "the file holds nothing but the defaults");
        let scene = driver.scene(&wram).unwrap();
        assert!(scene.texts().any(|(_, _, text)| text == "Up"), "the keyboard page is back");
    }

    /// A driver on the Video page with the cursor on the Aspect row.
    fn driver_on_video() -> (Driver, FakeMenu) {
        let (mut driver, menu) = driver_on_options(&saved_slots(&[0]));
        for key in [Key::Tab, Key::Tab, Key::Down, Key::Down, Key::Down, Key::Down, Key::Down] {
            driver.key(key, &menu.wram());
        }
        assert_eq!(driver.nav().options_category(), Category::Video);
        assert_eq!(driver.nav().options_row(), 5, "Aspect");
        (driver, menu)
    }

    #[test]
    fn a_change_of_aspect_is_applied_at_once_asked_about_and_written_only_when_kept() {
        let (mut driver, menu) = driver_on_video();
        driver.key(Key::Right, &menu.wram());
        assert_eq!(driver.nav().options_video_confirm(), Some(1), "the question, on Revert");
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert_eq!(
            frame.video.as_ref().and_then(|v| v.get("video.aspect")),
            Some(1),
            "applied now"
        );
        assert!(frame.settings.is_none(), "not written while the question is open");
        // Left and Right only move the question's cursor, to Keep.
        driver.key(Key::Left, &menu.wram());
        assert_eq!(driver.options().pc.get("video.aspect"), Some(1));
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.nav().options_video_confirm(), None);
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert_eq!(
            frame.settings.as_ref().and_then(|v| v.get("video.aspect")),
            Some(1),
            "Keep writes"
        );
        assert!(frame.video.is_none(), "nothing more to apply");
    }

    #[test]
    fn b_reverts_and_the_ten_seconds_run_out_on_revert_too() {
        let (mut driver, menu) = driver_on_video();
        driver.key(Key::Right, &menu.wram());
        driver.frame(&menu.wram(), None, Joypad::default());
        driver.key(Key::Cancel, &menu.wram());
        assert_eq!(driver.options().pc.get("video.aspect"), None, "B puts the aspect back");
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert_eq!(
            frame.video.as_ref().and_then(|v| v.get("video.aspect")),
            None,
            "and applies it"
        );
        // The countdown: 600 frames, the bar and the seconds follow it, and it reverts at the end.
        driver.key(Key::Right, &menu.wram());
        let ask =
            driver.state.options_view(&MenuProbe::new(&menu.wram(), &driver.labels)).video.unwrap();
        assert_eq!((ask.choice, ask.tenths), (1, 100));
        for _ in 0..300 {
            driver.frame(&menu.wram(), None, Joypad::default());
        }
        let ask =
            driver.state.options_view(&MenuProbe::new(&menu.wram(), &driver.labels)).video.unwrap();
        assert!((49..=51).contains(&ask.tenths), "{}", ask.tenths);
        for _ in 0..299 {
            driver.frame(&menu.wram(), None, Joypad::default());
        }
        assert_eq!(driver.options().pc.get("video.aspect"), Some(1), "one frame left");
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert_eq!(driver.options().pc.get("video.aspect"), None, "reverted at the timeout");
        assert_eq!(driver.nav().options_video_confirm(), None);
        assert_eq!(frame.video.as_ref().and_then(|v| v.get("video.aspect")), None);
    }

    #[test]
    fn scale_and_display_ask_too_but_look_scanlines_and_vsync_do_not() {
        let (mut driver, menu) = driver_on_video();
        for key in [Key::Down, Key::Right] {
            driver.key(key, &menu.wram());
        }
        assert_eq!(driver.nav().options_row(), 7, "Scanlines");
        assert_eq!(
            driver.nav().options_video_confirm(),
            None,
            "scanlines apply without a question"
        );
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert!(frame.video.is_some() && frame.settings.is_some());
        driver.key(Key::ResetPage, &menu.wram());
        assert_eq!(driver.nav().options_video_confirm(), None, "the mode did not change");
        // Reset this page asks when it changes the scale.
        driver.state.options.pc.set("video.scale", 5);
        driver.key(Key::ResetPage, &menu.wram());
        assert_eq!(driver.nav().options_video_confirm(), Some(1));
        driver.key(Key::Cancel, &menu.wram());
        assert_eq!(driver.options().pc.get("video.scale"), Some(5), "reverted to the old scale");
    }

    #[test]
    fn the_accessibility_rows_are_written_at_once_and_restyle_the_screen_the_same_frame() {
        let (mut driver, menu) = driver_on_options(&saved_slots(&[0]));
        for key in [Key::Tab, Key::Tab, Key::Tab, Key::Tab, Key::Tab] {
            driver.key(key, &menu.wram());
        }
        assert_eq!(driver.nav().options_category(), Category::Accessibility);
        let ops = |driver: &Driver| {
            driver.scene(&menu.wram()).expect("the options screen").ops().to_vec()
        };
        assert!(
            ops(&driver).iter().any(|op| matches!(op, ssd_menus::Op::Window { .. })),
            "the design as drawn"
        );
        // Contrast (row 1), then the cursor (row 2), then the text size (row 0).
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Right, &menu.wram());
        assert_eq!(driver.options().pc.get("access.contrast"), Some(1));
        let shown = ops(&driver);
        assert!(shown.iter().any(|op| matches!(op, ssd_menus::Op::WindowHc { .. })));
        assert!(!shown.iter().any(|op| matches!(op, ssd_menus::Op::Window { .. })));
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert_eq!(frame.settings.as_ref().and_then(|v| v.get("access.contrast")), Some(1));
        assert!(frame.video.is_none(), "nothing to ask about");
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Right, &menu.wram());
        assert_eq!(driver.options().pc.get("access.cursor"), Some(1));
        assert!(ops(&driver).iter().any(|op| matches!(op, ssd_menus::Op::Pointer { .. })));
        driver.key(Key::ResetPage, &menu.wram());
        assert_eq!(driver.options().pc.get("access.contrast"), None, "Reset this page");
        assert!(ops(&driver).iter().any(|op| matches!(op, ssd_menus::Op::Window { .. })));
    }

    #[test]
    fn a_question_open_when_the_driver_resets_is_reverted() {
        let (mut driver, menu) = driver_on_video();
        driver.key(Key::Right, &menu.wram());
        driver.reset();
        assert_eq!(driver.options().pc.get("video.aspect"), None);
        assert_eq!(driver.nav().options_video_confirm(), None);
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        assert!(frame.video.is_some(), "the window is told");
    }

    #[test]
    fn the_games_options_follow_the_save_slot_they_were_saved_in() {
        let sram = saved_slots(&[0, 1]);
        let (mut driver, mut menu) = driver_on_options(&sram);
        driver.state.options.game.set("gameplay.future", 7);
        let saved = save_into(&mut driver, &mut menu, &sram, 0);
        assert!(saved.saved_options, "a Save writes the slot file");
        let checksum = checksum_of_slot(&sram, 0);
        assert_eq!(
            driver.option_slots().entry_for(0, checksum).and_then(|v| v.get("gameplay.future")),
            Some(7)
        );
        assert_eq!(driver.option_slots().entry_for(1, checksum_of_slot(&sram, 1)), None);
        // A new run binds to the slot's values from the slot file.
        let slots = driver.option_slots().clone();
        let mut fresh = Driver::new(Labels::default())
            .with_cheats(CheatSidecar::empty())
            .with_options(slots, Values::new());
        fresh.state.curtain = Curtain::Up;
        fresh.frame(&menu.wram(), Some(&sram), Joypad::default());
        assert_eq!(fresh.options().game.get("gameplay.future"), Some(7));
    }
    #[test]
    fn a_difficulty_preset_writes_the_cheats_and_the_money_marks_the_save_and_follows_its_slot() {
        use ssd_menus::cheats::{MONEY, Multiplier, Setting};
        let sram = saved_slots(&[0]);
        let checksum = checksum_of_slot(&sram, 0);
        let (mut driver, mut menu) = driver_on_options(&sram);
        // Hard: enemy power x2, EXP x1, money x1/2, and the block carries the money pair.
        driver.key(Key::Right, &menu.wram());
        assert_eq!(driver.cheats().multiplier(Setting::Power), Multiplier::new(2, 1));
        assert_eq!(driver.cheats().money(), Multiplier::new(1, 2));
        let frame = driver.frame(&menu.wram(), None, Joypad::default());
        let block = frame.events.iter().find_map(|event| match event {
            HostEvent::Wram { address, bytes } if *address == crate::cheats::BLOCK => Some(bytes),
            _ => None,
        });
        assert_eq!(block.map(|bytes| &bytes[MONEY..MONEY + 2]), Some(&[1_u8, 2][..]));
        let saved = save_into(&mut driver, &mut menu, &sram, 0);
        assert!(saved.saved_cheats.is_some_and(|saved| saved.marked), "Hard marks the save");
        let kept = driver.option_slots().entry_for(0, checksum).unwrap();
        assert_eq!(
            (kept.get("difficulty.money"), kept.get("difficulty.preset")),
            (Some(0), Some(2))
        );
        // Continue: the slot's cheats come from the cheats file, the money from the options file.
        let mut fresh = Driver::new(Labels::default())
            .with_cheats(driver.cheat_sidecar().clone())
            .with_options(driver.option_slots().clone(), Values::new());
        fresh.state.curtain = Curtain::Up;
        let again = fresh.frame(&menu.wram(), Some(&sram), Joypad::default());
        assert_eq!(fresh.cheats().money(), Multiplier::new(1, 2));
        assert!(again.events.iter().any(|event| matches!(
            event,
            HostEvent::Wram { address, bytes } if *address == crate::cheats::BLOCK && bytes[MONEY..MONEY + 2] == [1, 2]
        )));
        // Normal: clean, and the file keeps nothing of the difficulty.
        driver.key(Key::Left, &menu.wram());
        assert!(driver.cheats().is_neutral());
        let clean = save_into(&mut driver, &mut menu, &sram, 0);
        assert!(clean.saved_cheats.is_some_and(|saved| !saved.marked), "Normal is clean");
        let kept = driver.option_slots().entry_for(0, checksum).unwrap();
        assert_eq!((kept.get("difficulty.money"), kept.get("difficulty.preset")), (None, None));
    }

    /// The Data packs of a run: `Alpha` on (with a chapter that is open and one that waits), `Beta` off, `Gamma` refused.
    fn two_packs() -> Catalog {
        let chapter = |name: &str, waits: bool| ssd_menus::packs::ChapterInfo {
            id: format!("{name}-id"),
            name: name.to_owned(),
            description: format!("{name} description."),
            waits,
        };
        let pack = |id: &str, on: bool, refusal: Option<&str>| ssd_menus::packs::PackInfo {
            id: id.to_owned(),
            name: id.trim_end_matches(".pack").to_owned(),
            version: "1.0.0".to_owned(),
            on,
            refusal: refusal.map(str::to_owned),
            ..Default::default()
        };
        let mut alpha = pack("alpha.pack", true, None);
        alpha.chapters = vec![chapter("Open", false), chapter("Waits", true)];
        Catalog {
            packs: vec![
                alpha,
                pack("beta.pack", false, None),
                pack("gamma.pack", false, Some("made for another ROM")),
            ],
            ..Catalog::default()
        }
    }

    /// The driver on the Options with the packs of `two_packs`, on the Data packs screen (opened from the Gameplay page).
    fn driver_on_packs(catalog: Catalog) -> (Driver, FakeMenu) {
        let (mut driver, menu) = driver_on_options(&saved_slots(&[0]));
        driver.state.packs = catalog;
        driver.key(Key::Tab, &menu.wram());
        for _ in 0..5 {
            driver.key(Key::Down, &menu.wram());
        }
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.nav().screen(), Screen::Mods, "the button opens the Data packs");
        (driver, menu)
    }

    /// The texts the Data packs screen draws now.
    fn mods_texts(driver: &Driver, menu: &FakeMenu) -> Vec<String> {
        driver
            .scene(&menu.wram())
            .expect("the screen is up")
            .texts()
            .map(|(_, _, text)| text.to_owned())
            .collect()
    }

    #[test]
    fn the_list_switches_packs_and_applies_the_set_in_its_order_once() {
        let (mut driver, menu) = driver_on_packs(two_packs());
        let shown = mods_texts(&driver, &menu);
        assert!(
            shown.iter().any(|text| text == "alpha")
                && shown.iter().any(|text| text == "-- Chapters --")
        );
        driver.key(Key::Confirm, &menu.wram());
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        driver.key(Key::Apply, &menu.wram());
        driver.key(Key::Up, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.take_mods_request(), Some(Request::Apply(vec!["beta.pack".to_owned()])));
        assert_eq!(driver.take_mods_request(), None, "taken once");
    }

    #[test]
    fn apply_with_the_set_in_use_asks_for_nothing_and_says_so() {
        let (mut driver, menu) = driver_on_packs(two_packs());
        driver.key(Key::Apply, &menu.wram());
        driver.key(Key::Up, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.take_mods_request(), None, "the set is the one in use");
        assert!(mods_texts(&driver, &menu).iter().any(|text| text == UNCHANGED));
    }

    #[test]
    fn with_y_held_a_pack_moves_and_the_order_of_the_set_is_the_order_applied() {
        let (mut driver, menu) = driver_on_packs(two_packs());
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        // Alpha and Beta are on now, in that order: Y held moves Alpha below Beta.
        driver.set_moving(true);
        driver.key(Key::Up, &menu.wram());
        driver.set_moving(false);
        let shown = mods_texts(&driver, &menu);
        let first = shown.iter().position(|text| text == "beta").expect("beta is listed");
        let second = shown.iter().position(|text| text == "alpha").expect("alpha is listed");
        assert!(first < second, "beta is first now: {shown:?}");
        driver.key(Key::Apply, &menu.wram());
        driver.key(Key::Up, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(
            driver.take_mods_request(),
            Some(Request::Apply(vec!["beta.pack".to_owned(), "alpha.pack".to_owned()]))
        );
    }

    #[test]
    fn a_chapter_starts_only_when_it_is_open_and_the_list_is_the_set_in_use() {
        let (mut driver, menu) = driver_on_packs(two_packs());
        // The lines: Alpha, Beta, Gamma, "Install a pack...", then the chapters of Alpha: Open (4) and Waits (5, locked).
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.nav().mods_ask(), Some((ssd_menus::nav::ModsAsk::Chapter(4), 1)));
        driver.key(Key::Left, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.take_mods_request(), Some(Request::Chapter("Open-id".to_owned())));
        // The list is changed and not applied yet: Beta goes on, and the chapter does not start; the screen says why.
        for _ in 0..3 {
            driver.key(Key::Up, &menu.wram());
        }
        driver.key(Key::Confirm, &menu.wram());
        for _ in 0..3 {
            driver.key(Key::Down, &menu.wram());
        }
        driver.key(Key::Confirm, &menu.wram());
        driver.key(Key::Left, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.take_mods_request(), None);
        assert!(
            mods_texts(&driver, &menu).iter().any(|text| text.starts_with("Apply the packs first"))
        );
    }

    #[test]
    fn a_locked_chapter_says_so_until_the_game_is_completed_or_the_cheat_unlocks_it() {
        let (mut driver, menu) = driver_on_packs(two_packs());
        for _ in 0..5 {
            driver.key(Key::Down, &menu.wram());
        }
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.nav().mods_ask(), None, "a locked chapter asks nothing");
        assert!(mods_texts(&driver, &menu).iter().any(|text| text.starts_with("Locked:")));
        // The cheat "Unlock chapters" offers it: the chapter asks now.
        driver.state.cheats.set_chapters(true);
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.nav().mods_ask(), Some((ssd_menus::nav::ModsAsk::Chapter(5), 1)));
        driver.key(Key::Cancel, &menu.wram());
        // B goes back to the Options, on the button.
        driver.key(Key::Cancel, &menu.wram());
        assert_eq!(driver.nav().screen(), Screen::Options);
        assert_eq!(driver.nav().options_row(), 5);
    }

    #[test]
    fn main_game_goes_back_to_the_main_saves_from_a_chapter_session() {
        let mut catalog = two_packs();
        catalog.playing = Some("Open".to_owned());
        let (mut driver, menu) = driver_on_packs(catalog);
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        assert!(mods_texts(&driver, &menu).iter().any(|text| text == "Main game"));
        driver.key(Key::Confirm, &menu.wram());
        driver.key(Key::Left, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.take_mods_request(), Some(Request::Main));
    }

    #[test]
    fn a_save_state_keeps_the_bag_and_the_cheats_and_a_load_gets_them_back() {
        let (mut driver, mut menu) = battle_with_bag();
        driver.state.cheats.set_chapters(true);
        let bytes = driver.session_bytes().expect("a bound bag");
        let units = driver.bag().map(Bag::units);
        // A load resets the driver: no bag, the cheats off, and the battle matches no save to bind to.
        driver.reset();
        assert!(driver.bag().is_none());
        let events = driver.restore_session(&bytes, &menu.wram()).unwrap();
        menu.absorb(&events);
        assert_eq!(driver.bag().map(Bag::units), units, "the bag is back");
        assert!(driver.state.cheats.chapters(), "and the cheats of that moment");
        frame(&mut driver, &mut menu, Joypad::default());
        assert!(driver.state.cheats.chapters(), "a frame keeps them: no bind follows");
        assert!(driver.restore_session(b"junk", &menu.wram()).is_err());
    }

    #[test]
    fn install_a_pack_asks_the_host_and_the_pack_it_installs_joins_the_list_off() {
        let (mut driver, menu) = driver_on_packs(two_packs());
        for _ in 0..3 {
            driver.key(Key::Down, &menu.wram());
        }
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.take_mods_request(), Some(Request::Install));
        let delta = PackInfo {
            id: "delta.pack".to_owned(),
            name: "delta".to_owned(),
            on: true,
            ..PackInfo::default()
        };
        driver.add_pack(delta.clone());
        assert!(mods_texts(&driver, &menu).iter().any(|text| text == "delta"), "listed");
        assert!(!driver.state.packs_edit.last().unwrap().on, "and off");
        // A new version of a pack takes its place
        driver.add_pack(PackInfo { name: "delta 2".to_owned(), ..delta });
        let shown = mods_texts(&driver, &menu);
        assert!(
            shown.iter().any(|text| text == "delta 2") && !shown.iter().any(|text| text == "delta")
        );
        assert_eq!(driver.state.packs.packs.len(), driver.state.packs_edit.len());
    }

    #[test]
    fn a_notice_the_host_gives_is_shown_until_the_next_key() {
        let (mut driver, menu) = driver_on_packs(two_packs());
        driver.set_mods_alert("mods.toml cannot be read.".to_owned());
        assert!(mods_texts(&driver, &menu).iter().any(|text| text == "mods.toml cannot be read."));
        driver.key(Key::Down, &menu.wram());
        assert!(!mods_texts(&driver, &menu).iter().any(|text| text == "mods.toml cannot be read."));
    }
}
