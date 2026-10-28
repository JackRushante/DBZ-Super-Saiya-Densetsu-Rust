//! `menu-test`: the HD menus played headless on the cached ROM, as a player plays them. A scenario
//! starts from a crafted save (its Continue, then the field) or from a frame of a replay, opens the
//! menu with a press of A, makes the HD's choices with keys (the driver of `ssd_game::menu::driver`
//! plays them), and checks what the original and the curtain show after each. `town` starts from the
//! shop's town. `diff` checks that a choice's own presses, replayed as a plain Classic run, end in the
//! same work RAM, and that the Classic timeline of the same choice reaches the same game state.
//! Nothing is written but the report on stdout (the items suite's scratch files are in a temporary
//! directory that it removes at its end).

use crate::save;
use anyhow::{Context, bail};
use ssd_game::data as d;
use ssd_game::menu::driver::{Binding, Curtain, Driver};
use ssd_game::menu::presses::{Next, Outcome};
use ssd_game::scripts;
use ssd_menus::Key;
use ssd_menus::bag::{Bag, Class, Rules, Sidecar};
use ssd_menus::view::{Labels, Sheet};
use ssd_oracle::{
    CORE_VERSION, HostEvent, Joypad, MODE_ENHANCED, Machine, MemorySaveStore, NullAudio, Recorder,
    Replay, apply_events,
};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The bus address of the first byte of work RAM.
const WRAM_BASE: u32 = 0x7E_0000;
/// Frames a choice may run before the test calls it stuck.
const CHOICE_LIMIT: u32 = 6_000;
/// The most presses of A [`Harness::answer`] gives a use, and the frames between two of them.
const ANSWER_PRESSES: u32 = 40;
const ANSWER_GAP: u32 = 40;
/// Idle frames after a comparison point, before the states are compared.
const SETTLE: u32 = 240;
/// Frames a message runs before it takes A (its text is drawn letter by letter).
const MESSAGE_RUN: u32 = 200;

/// Where a run starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// A battery save: its Continue of `slot` (0 to 2), then the field.
    Save { path: PathBuf, slot: usize },
    /// A replay, played for `frames` frames (the field of its map, with the menu closed).
    Replay { path: PathBuf, frames: u64 },
    /// The cheats suite's scenarios: the reference replays of `dir` (the workspace's by default), and the scripted
    /// loss from a battery save (its slot, from `--slot`), when one is given.
    Scenarios { dir: Option<PathBuf>, save: Option<(PathBuf, usize)>, data_out: Option<PathBuf> },
}

/// The command line of `menu-test`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// `field` (the scenarios) or `diff` (the comparison with Classic presses).
    pub suite: String,
    pub start: Start,
    /// The scenarios to run (all of the suite when empty).
    pub only: Vec<String>,
    /// `items` only: where the Use flow's replays for the snapshots are written.
    pub snapshots: Option<PathBuf>,
}

/// Parses `field|town|diff --save <file.sav> [--slot N] | --replay <file> --frames N [scenario...]`.
///
/// # Errors
///
/// A usage message.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut it = args.iter();
    let suite = match it.next() {
        Some(&suite @ ("field" | "town" | "diff" | "items" | "cheats" | "battle")) => {
            suite.to_owned()
        }
        _ => return Err("menu-test takes field, town, diff, items, cheats or battle".to_owned()),
    };
    let mut save = None;
    let mut slot = 0;
    let mut replay = None;
    let mut frames = None;
    let mut replays = None;
    let mut data_out = None;
    let mut only = Vec::new();
    let mut snapshots = None;
    while let Some(&arg) = it.next() {
        match arg {
            "--save" => save = it.next().map(PathBuf::from),
            "--snapshots" => snapshots = it.next().map(PathBuf::from),
            "--replays" => replays = it.next().map(PathBuf::from),
            "--data-out" => data_out = it.next().map(PathBuf::from),
            "--slot" => {
                slot = it
                    .next()
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|n| (1..=3).contains(n))
                    .ok_or("--slot takes 1, 2 or 3")?
                    - 1;
            }
            "--replay" => replay = it.next().map(PathBuf::from),
            "--frames" => {
                frames = it.next().and_then(|n| n.parse::<u64>().ok());
                if frames.is_none() {
                    return Err("--frames takes a number of frames".to_owned());
                }
            }
            option if option.starts_with("--") => return Err(format!("unknown option `{option}`")),
            name => only.push(name.to_owned()),
        }
    }
    let start = match (suite.as_str(), save, replay, frames) {
        ("cheats", save, None, None) => {
            Start::Scenarios { dir: replays, save: save.map(|path| (path, slot)), data_out }
        }
        (_, Some(path), None, None) => Start::Save { path, slot },
        (_, None, Some(path), Some(frames)) => Start::Replay { path, frames },
        _ => return Err("give --save <file.sav>, or --replay <file> with --frames N".to_owned()),
    };
    Ok(Args { suite, start, only, snapshots })
}

/// The checks of one scenario: each is a line of the report, and a failed one fails the suite.
#[derive(Default)]
pub(crate) struct Report {
    lines: Vec<(bool, String)>,
}

impl Report {
    pub(crate) fn check(&mut self, ok: bool, what: &str) {
        self.lines.push((ok, what.to_owned()));
    }

    pub(crate) fn passed(&self) -> bool {
        self.lines.iter().all(|(ok, _)| *ok)
    }
}

/// A run of the menu on the machine, with the driver that plays the HD.
pub(crate) struct Harness {
    pub(crate) machine: Machine,
    pub(crate) driver: Driver,
    /// The joypad of each frame run through the driver, when it is recorded.
    tape: Option<Vec<Joypad>>,
    /// The replay being recorded, when the frames are recorded (their host events too).
    recorder: Option<Recorder>,
}

impl Harness {
    /// A harness of a machine and its driver, with nothing recorded.
    pub(crate) fn new(machine: Machine, driver: Driver) -> Self {
        Self { machine, driver, tape: None, recorder: None }
    }

    /// One frame: the driver's joypad and host events for it (from the live one), the events made on
    /// the machine, and the frame run.
    pub(crate) fn frame(&mut self, live: Joypad) -> anyhow::Result<()> {
        let frame = self.driver.frame(self.machine.wram(), self.machine.sram(), live);
        apply_events(&mut self.machine, &frame.events)?;
        match self.recorder.as_mut() {
            Some(recorder) => {
                recorder.run_frame_with_events(
                    &mut self.machine,
                    frame.pad,
                    &frame.events,
                    &mut NullAudio,
                )?;
            }
            None => self.machine.run_frame(frame.pad, &mut NullAudio)?,
        }
        if let Some(tape) = self.tape.as_mut() {
            tape.push(frame.pad);
        }
        Ok(())
    }

    pub(crate) fn frames(&mut self, count: u32) -> anyhow::Result<()> {
        for _ in 0..count {
            self.frame(Joypad::default())?;
        }
        Ok(())
    }

    /// A press of the player's own: held for two frames, released on the third.
    pub(crate) fn press(&mut self, pad: Joypad) -> anyhow::Result<()> {
        self.frame(pad)?;
        self.frame(pad)?;
        self.frame(Joypad::default())
    }

    /// Frames until the driver has no choice under way.
    pub(crate) fn until_idle(&mut self) -> anyhow::Result<()> {
        for _ in 0..CHOICE_LIMIT {
            if !self.driver.busy() {
                return Ok(());
            }
            self.frame(Joypad::default())?;
        }
        bail!("a choice did not end in {CHOICE_LIMIT} frames (curtain {:?})", self.driver.curtain())
    }

    /// The player's part after a use: the original's screens (the fighter the use is for, its messages, its effect) are
    /// answered with A, one press a window apart, while the curtain is lifted and the menu is open. Returns the presses.
    pub(crate) fn answer(&mut self) -> anyhow::Result<u32> {
        for presses in 0..ANSWER_PRESSES {
            if self.driver.curtain() != Curtain::Lifted
                || self.driver.busy()
                || self.byte(d::FIELD_STATE) == 0
            {
                return Ok(presses);
            }
            self.press(a())?;
            self.frames(ANSWER_GAP)?;
        }
        bail!("the use was not over after {ANSWER_PRESSES} presses of A")
    }

    /// The HD's key, and the frames of its choice.
    pub(crate) fn key(&mut self, key: Key) -> anyhow::Result<()> {
        self.driver.key(key, self.machine.wram());
        self.frame(Joypad::default())?;
        self.until_idle()
    }

    /// The player's A on the field: the menu opens, and the HD shows its command window.
    pub(crate) fn open(&mut self) -> anyhow::Result<()> {
        self.driver.open();
        self.frame(Joypad::default())?;
        self.until_idle()?;
        self.frames(30)
    }

    /// A byte of work RAM.
    pub(crate) fn byte(&self, address: u32) -> u8 {
        self.machine.wram()[(address - WRAM_BASE) as usize]
    }

    /// A word of work RAM (little-endian).
    pub(crate) fn word(&self, address: u32) -> u16 {
        u16::from_le_bytes([self.byte(address), self.byte(address + 1)])
    }
}

fn a() -> Joypad {
    Joypad { a: true, ..Joypad::default() }
}

fn b() -> Joypad {
    Joypad { b: true, ..Joypad::default() }
}

/// What a start gives: a machine at the field, and how to make another one there.
struct Starts {
    /// The machine's state at the field.
    state: Vec<u8>,
    /// A machine of the same cartridge (the save in it), before its state is loaded.
    fresh: Box<dyn Fn() -> anyhow::Result<Machine>>,
}

/// Plays the start: the save's Continue or the replay's frames, and keeps the state there.
fn start_of(rom: &[u8], start: &Start) -> anyhow::Result<Starts> {
    let rom = rom.to_vec();
    match start {
        Start::Scenarios { .. } => {
            bail!("menu-test cheats runs its own scenarios: it has no start state")
        }
        Start::Save { path, slot } => {
            let sram =
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let make = move || {
                let mut saves = MemorySaveStore::default();
                saves.0.insert("sav".to_owned(), sram.clone());
                Ok(Machine::new(rom.clone(), Box::new(saves))?)
            };
            let mut machine = make()?;
            let timeline = scripts::parse_timeline(&save::continue_prefix(*slot))
                .map_err(anyhow::Error::msg)?;
            for pad in timeline {
                machine.run_frame(pad, &mut NullAudio)?;
            }
            let state = machine.save_state();
            Ok(Starts { state, fresh: Box::new(make) })
        }
        Start::Replay { path, frames } => {
            let bytes =
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let crc = ssd_rom::crc32(&rom);
            let replay = Replay::from_bytes(&bytes, crc, CORE_VERSION)?;
            let make = move || {
                let replay = Replay::from_bytes(&bytes, crc, CORE_VERSION)?;
                Ok(replay.power_on(rom.clone())?)
            };
            let mut machine = make()?;
            for frame in 0..*frames {
                machine.run_frame(replay.input(frame), &mut NullAudio)?;
            }
            let state = machine.save_state();
            Ok(Starts { state, fresh: Box::new(make) })
        }
    }
}

/// A harness at the start: a fresh machine with the start's state, and a driver with the labels. The
/// bag is not bound (the original's list is kept as it is) unless `bag` asks for it.
fn harness(rom: &[u8], starts: &Starts) -> anyhow::Result<Harness> {
    harness_with(rom, starts, false)
}

/// [`harness`], with the bag bound to the save when `bag` is set (as the window binds it).
fn harness_with(rom: &[u8], starts: &Starts, bag: bool) -> anyhow::Result<Harness> {
    let mut machine = (starts.fresh)()?;
    machine.load_state(&starts.state)?;
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    let mut driver = Driver::new(ssd_catalogue::menu_labels(&catalogue));
    if !bag {
        driver.set_binding(Binding::Recorded);
    }
    Ok(Harness { machine, driver, tape: None, recorder: None })
}

/// The scenarios of the field suite, by name.
type Scenario = fn(&mut Harness, &mut Report) -> anyhow::Result<()>;

const FIELD_SCENARIOS: [(&str, Scenario); 14] = [
    ("talk", talk),
    ("look", look),
    ("fly-land", fly_land),
    ("fly-menu", fly_menu),
    ("item", item),
    ("menu-cards", menu_cards),
    ("menu-status", menu_status),
    ("menu-order", menu_order),
    ("menu-text", menu_text),
    ("menu-save", menu_save_screen),
    ("cancel-command", cancel_command),
    ("cancel-menu", cancel_menu),
    ("menu-cheats", menu_cheats),
    ("abort", abort),
];

fn talk(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.open()?;
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Lifted, "Talk lifts the curtain for the message");
    r.check(h.byte(d::MENU_BUSY) == 1, "the message is up (menu busy)");
    // The text runs before it waits for A: a press during the run is not taken.
    h.frames(MESSAGE_RUN)?;
    h.press(a())?;
    h.frames(SETTLE)?;
    r.check(h.byte(d::FIELD_STATE) == 0, "the message closes and the field is free");
    r.check(h.driver.curtain() == Curtain::Down, "the curtain comes down with the field");
    Ok(())
}

fn look(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.open()?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Lifted, "Look lifts the curtain for the message");
    r.check(h.byte(d::MENU_BUSY) == 1, "the message is up");
    h.frames(MESSAGE_RUN)?;
    h.press(a())?;
    h.frames(SETTLE)?;
    r.check(h.byte(d::FIELD_STATE) == 0, "the message closes and the field is free");
    Ok(())
}

fn fly_land(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.open()?;
    h.key(Key::Down)?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Lifted, "Fly lifts the curtain for the take-off");
    r.check(h.byte(d::MENU_SCREEN) == 1, "the original takes off on its screen 1");
    h.frames(400)?;
    r.check(h.byte(d::TRAVEL_MODE) == 2, "the party flies");
    r.check(h.byte(d::FIELD_STATE) == 0, "the take-off closes the menu");
    r.check(h.driver.curtain() == Curtain::Down, "the curtain comes down");
    h.open()?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Command, "flying: the command window");
    h.key(Key::Confirm)?;
    r.check(h.byte(d::MENU_SCREEN) == 5, "Land runs on the original's screen 5");
    r.check(h.driver.curtain() == Curtain::Lifted, "Land lifts the curtain for the landing");
    h.frames(400)?;
    r.check(h.byte(d::TRAVEL_MODE) == 1, "the party walks again");
    r.check(h.byte(d::FIELD_STATE) == 0, "the landing closes the menu");
    Ok(())
}

fn item(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    // The bag is bound, as the window binds it: its keys move the HD cursor over the stacks.
    h.driver.set_binding(Binding::Auto);
    h.frames(1)?;
    h.open()?;
    for _ in 0..3 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Up, "Item keeps the curtain up: the HD item list");
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Items, "the HD shows the item list");
    r.check(h.byte(d::MENU_SCREEN) == 2, "the original is on its Item screen");
    h.key(Key::Down)?;
    h.key(Key::Down)?;
    r.check(h.driver.nav().cursor() == 2, "the HD cursor moves down the list");
    h.key(Key::Cancel)?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Command, "cancel goes back");
    r.check(h.driver.nav().cursor() == 3, "the command window keeps the Item row");
    r.check(h.byte(d::MENU_SCREEN) == 0, "the original is back on its command window");
    h.key(Key::Cancel)?;
    r.check(h.driver.curtain() == Curtain::Down, "cancel at the command window closes the menu");
    r.check(h.byte(d::FIELD_STATE) == 0, "the field is free");
    Ok(())
}

/// Opens the Menu list (the fourth row of the field's command window), for the four entries below.
pub(crate) fn menu_list(h: &mut Harness) -> anyhow::Result<()> {
    h.open()?;
    for _ in 0..4 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    Ok(())
}

/// The texts of the HD screen now, with the cursor and the hand the original dealt (its `card_hand` bytes).
fn cards_texts(h: &Harness) -> Vec<String> {
    let wram = h.machine.wram().to_vec();
    h.driver
        .scene(&wram)
        .map(|list| list.texts().map(|(_, _, text)| text.to_owned()).collect())
        .unwrap_or_default()
}

/// The Cards: the HD screen over the original's, which deals the hand and waits for A. The cursor walks the hand (the
/// original does not move), the numbers and the kanji shown are the ones in `card_hand`, B goes back to the Menu list
/// through the original's own close and reopening, and Start closes the menu.
fn menu_cards(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    menu_list(h)?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Menu, "the HD Menu list is up");
    h.key(Key::Confirm)?;
    r.check(
        h.driver.curtain() == Curtain::Up && h.driver.nav().screen() == ssd_menus::Screen::Cards,
        "Cards keeps the curtain up: the HD hand",
    );
    r.check(h.byte(d::MENU_SCREEN) == 6, "the original is on its Cards screen");
    // The original deals the hand one card at a time: the HD shows the backs until it has
    let first = cards_texts(h);
    r.check(
        first.iter().any(|text| text.starts_with("Hand ") && text != "Hand 5/5"),
        "the hand is dealt one by one: not all five at once",
    );
    h.frames(SETTLE)?;
    let hand = |h: &Harness, slot: u32| {
        let base = 0x7E_0140 + 6 * slot;
        (h.byte(base), h.byte(base + 3), h.byte(base + 4), h.byte(base + 5))
    };
    let slots: Vec<_> = (0..5).map(|slot| hand(h, slot)).collect();
    r.check(
        slots.iter().all(|&(flag, attack, defence, symbol)| {
            flag == 0 && (1..=8).contains(&attack) && (1..=8).contains(&defence) && symbol < 7
        }),
        &format!("the original dealt five cards of values 1 to 8 and symbols 0 to 6: {slots:?}"),
    );
    let readings = ["Kai", "Kami", "Ma", "Waku", "Ginyu", "Hitsu", "Chou"];
    let counts = |value: u8| if value == 8 { 10 } else { value };
    for (at, &(_, attack, defence, symbol)) in slots.iter().enumerate() {
        let texts = cards_texts(h);
        r.check(texts.iter().any(|text| text == "Hand 5/5"), "five cards in the hand");
        r.check(
            texts.contains(&readings[usize::from(symbol)].to_owned()),
            &format!("card {at}: the kanji is named {}", readings[usize::from(symbol)]),
        );
        let numbers: Vec<String> = h
            .driver
            .scene(h.machine.wram())
            .map(|list| {
                list.ops()
                    .iter()
                    .filter_map(|op| match op {
                        ssd_menus::Op::Text {
                            text, size, align: ssd_menus::Align::Right, ..
                        } if (*size - 30.0).abs() < f32::EPSILON => Some(text.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        r.check(
            numbers == [counts(attack).to_string(), counts(defence).to_string()],
            &format!("card {at}: attack {attack} and defence {defence} as the game holds them: {numbers:?}"),
        );
        if at < 4 {
            h.key(Key::Right)?;
        }
    }
    let faces = h.driver.scene(h.machine.wram()).map_or(0, |list| {
        list.ops().iter().filter(|op| matches!(op, ssd_menus::Op::Portrait { .. })).count()
    });
    r.check(faces >= 2, &format!("the party's faces are in the row of the favoured: {faces}"));
    r.check(h.driver.nav().cursor() == 4, "the cursor walked the five cards");
    r.check(h.byte(d::MENU_SCREEN) == 6, "the original stayed on its Cards screen");
    h.key(Key::Right)?;
    r.check(h.driver.nav().cursor() == 0, "the cursor wraps");
    // Back: the original closes and the menu opens again on the Menu list, the HD cursor on Cards
    h.key(Key::Cancel)?;
    r.check(h.driver.last_outcome() == Some(Outcome::Reached), "back reached the Menu list");
    r.check(
        h.driver.curtain() == Curtain::Up && h.driver.nav().screen() == ssd_menus::Screen::Menu,
        "B on the Cards goes back to the HD Menu list",
    );
    r.check(h.byte(d::MENU_SCREEN) == 4, "the original is on its Menu list");
    r.check(h.driver.nav().cursor() == 0, "the cursor is on Cards");
    // Cards again, then Start closes the menu
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Cards, "Cards opens again");
    h.frames(SETTLE)?;
    h.driver.close_menu();
    h.until_idle()?;
    h.frames(SETTLE)?;
    r.check(h.byte(d::FIELD_STATE) == 0, "Start closes the menu: the field is free");
    r.check(h.driver.curtain() == Curtain::Down, "the curtain is down");
    Ok(())
}

/// The HD Status screen is the original's two pages in one: Status opens under the HD's curtain with the original on its
/// page 1; Up and Down move the member (the right panel follows, nothing is sent to the original, and its records are not
/// written); B goes back to the Menu list with its cursor on Status; Start closes the menu from the screen.
fn menu_status(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    use ssd_menus::Screen;
    let records = |h: &Harness| h.machine.wram()[0x0B00..0x0C20].to_vec();
    let scene = |h: &Harness| {
        h.driver
            .scene(h.machine.wram())
            .map(|list| list.texts().map(|(_, _, text)| text.to_owned()).collect::<Vec<_>>())
    };
    menu_list(h)?;
    let before = records(h);
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Up, "Status keeps the curtain up: the HD screen");
    r.check(h.driver.nav().screen() == Screen::Status, "the HD shows the Status screen");
    r.check(h.byte(d::MENU_SCREEN) == 7, "the original runs its Status screen under it");
    h.frames(SETTLE)?;
    let texts = scene(h).unwrap_or_default();
    let party = (1..=9).find(|n| texts.contains(&format!("{n}/9"))).unwrap_or(0);
    r.check(party >= 2, &format!("the header counts the party over nine slots ({party}/9)"));
    if party < 2 {
        return Ok(());
    }
    for label in ["HP", "KI", "BP", "PLA.", "EXP", "To next LV", "Techniques", "KI-USE"] {
        r.check(texts.contains(&label.to_owned()), &format!("the panel has its {label} line"));
    }
    // Down moves the member and the panel follows, at once; the original is not touched.
    let screen_step = (h.byte(d::MENU_SCREEN), h.byte(d::MENU_STEP));
    h.key(Key::Down)?;
    r.check(h.driver.nav().cursor() == 1, "Down moves the cursor to the second member");
    r.check(!h.driver.busy(), "moving the member starts no press of the original");
    r.check(
        (h.byte(d::MENU_SCREEN), h.byte(d::MENU_STEP)) == screen_step,
        "the original stays on its Status page 1",
    );
    let second = scene(h).unwrap_or_default();
    r.check(second != texts, "the right panel follows the cursor");
    for _ in 0..party - 1 {
        h.key(Key::Down)?;
    }
    r.check(h.driver.nav().cursor() == 0, "Down wraps after the last member");
    h.key(Key::Up)?;
    r.check(usize::from(h.driver.nav().cursor()) == party - 1, "Up wraps before the first");
    r.check(records(h) == before, "the party records are the same: Status is read-only");
    // B goes back to the Menu list; its cursor is on Status again.
    h.key(Key::Cancel)?;
    r.check(h.driver.curtain() == Curtain::Up, "back from Status: the HD Menu list");
    r.check(h.driver.nav().screen() == Screen::Menu, "B goes back to the Menu list");
    r.check(h.driver.nav().cursor() == 1, "the Menu list keeps its cursor on Status");
    r.check(h.byte(d::MENU_SCREEN) == 4, "the original is back on its Menu list");
    // Status again, and Start closes the menu from it.
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().screen() == Screen::Status, "Status opens again");
    r.check(h.driver.nav().cursor() == 0, "on its first member");
    h.driver.close_menu();
    h.frame(Joypad::default())?;
    h.until_idle()?;
    h.frames(SETTLE)?;
    r.check(h.driver.curtain() == Curtain::Down, "Start closes the menu: the curtain is down");
    r.check(h.byte(d::FIELD_STATE) == 0, "the field is free");
    r.check(records(h) == before, "the party records are unchanged after the menu closes");
    Ok(())
}

/// Puts the party `ids` into the original's records (each a copy of the second record with its own character id), the
/// size and the party list, as the game holds a party of that many: the saves at hand have two members only.
fn stage_party(h: &mut Harness, ids: &[u8]) -> anyhow::Result<()> {
    let base: Vec<u8> = (0..32).map(|i| h.byte(d::PARTY_RECORDS + 32 + i)).collect();
    let mut events = Vec::new();
    for (row, &id) in ids.iter().enumerate() {
        let mut record = base.clone();
        record[0x1C] = id;
        events.push(HostEvent::Wram { address: d::PARTY_RECORDS + 32 * row as u32, bytes: record });
    }
    events.push(HostEvent::Wram { address: 0x7E_01BB, bytes: vec![ids.len() as u8] });
    let mut order = ids.to_vec();
    order.push(0xFF);
    events.push(HostEvent::Wram { address: d::PARTY_ORDER, bytes: order });
    apply_events(&mut h.machine, &events)?;
    Ok(())
}

/// Presses Down on the HD Order screen until its cursor is on `row` (a slot, or the Apply row after the last).
fn cursor_to(h: &mut Harness, row: usize) -> anyhow::Result<()> {
    for _ in 0..8 {
        if h.driver.order().cursor() == row {
            return Ok(());
        }
        h.key(Key::Down)?;
    }
    bail!("the Order screen's cursor did not reach row {row}")
}

/// Grabs the member in slot `from` of the HD Order screen, carries it to slot `to` and drops it.
fn carry(h: &mut Harness, from: usize, to: usize) -> anyhow::Result<()> {
    cursor_to(h, from)?;
    h.key(Key::Confirm)?;
    while h.driver.order().held().is_some_and(|slot| slot != to) {
        h.key(if h.driver.order().held().is_some_and(|slot| slot > to) {
            Key::Up
        } else {
            Key::Down
        })?;
    }
    h.key(Key::Confirm)
}

/// The party order list of the game, and the character of each of its first records.
fn party_now(h: &Harness, size: usize) -> (Vec<u8>, Vec<u8>) {
    let order = (0..=size as u32).map(|i| h.byte(d::PARTY_ORDER + i)).collect();
    let records = (0..size as u32).map(|i| h.byte(d::PARTY_RECORDS + 32 * i + 0x1C)).collect();
    (order, records)
}

/// The Order screen is the HD's own: the original keeps its Menu list while the HD holds the order (a member grabbed, moved
/// and dropped, B undoing the moves), nothing is written until "Apply order", and then the original's own Order screen
/// takes the members pick by pick (its cursor walked to each row, A on each, A on Yes), writes the records, and closes
/// the menu. The party is staged with five members.
fn menu_order(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    use ssd_menus::Screen;
    stage_party(h, &[0, 1, 2, 3, 4])?;
    let before = party_now(h, 5);
    r.check(before == (vec![0, 1, 2, 3, 4, 0xFF], vec![0, 1, 2, 3, 4]), "the staged party is in");
    menu_list(h)?;
    for _ in 0..2 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    r.check(
        h.driver.curtain() == Curtain::Up && h.driver.nav().screen() == Screen::Order,
        "Order opens the HD Order screen",
    );
    r.check(h.byte(d::MENU_SCREEN) == 4, "the original keeps its Menu list under it");
    r.check(h.driver.order().members() == [0, 1, 2, 3, 4], "it opens on the game's order");
    // Gohan to the front, Tien (the fifth) to the second place.
    carry(h, 1, 0)?;
    carry(h, 4, 1)?;
    r.check(h.driver.order().members() == [1, 4, 0, 2, 3], "the HD holds the new order");
    r.check(party_now(h, 5) == before, "editing writes nothing in the game");
    r.check(h.byte(d::MENU_SCREEN) == 4, "and presses nothing: the original is on its Menu list");
    // B undoes the last move, then the one before.
    h.key(Key::Cancel)?;
    r.check(h.driver.order().members() == [1, 0, 2, 3, 4], "B undoes the last move");
    h.key(Key::Cancel)?;
    r.check(h.driver.order().members() == [0, 1, 2, 3, 4], "and the one before it");
    // Make the order again and apply it.
    carry(h, 1, 0)?;
    carry(h, 4, 1)?;
    cursor_to(h, 5)?;
    h.key(Key::Confirm)?;
    h.frames(SETTLE)?;
    r.check(
        party_now(h, 5).1 == [1, 4, 0, 2, 3],
        "the original wrote the records in the new order",
    );
    r.check(h.byte(d::FIELD_STATE) == 0, "its Order screen closed the menu");
    r.check(h.driver.curtain() == Curtain::Down, "the curtain is down");
    // Reopened, the screen starts on the new order; the whole party reversed is applied too.
    menu_list(h)?;
    for _ in 0..2 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    r.check(h.driver.order().members() == [1, 4, 0, 2, 3], "it reopens on the order the game has");
    // Reverse: the first to the end, then the new first to the fourth place, and so on.
    for end in (1..5).rev() {
        carry(h, 0, end)?;
    }
    r.check(h.driver.order().members() == [3, 2, 0, 4, 1], "the HD holds the reversed order");
    cursor_to(h, 5)?;
    h.key(Key::Confirm)?;
    h.frames(SETTLE)?;
    r.check(party_now(h, 5).1 == [3, 2, 0, 4, 1], "the reversed party");
    r.check(h.byte(d::FIELD_STATE) == 0, "and the menu is closed");
    // B on an order not touched goes back to the Menu list, with nothing pressed.
    menu_list(h)?;
    for _ in 0..2 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    h.key(Key::Cancel)?;
    r.check(h.driver.nav().screen() == Screen::Menu, "B goes back to the Menu list");
    r.check(h.byte(d::MENU_SCREEN) == 4, "the original is still on its Menu list");
    for _ in 0..3 {
        h.key(Key::Cancel)?;
    }
    r.check(h.byte(d::FIELD_STATE) == 0, "the menu closes");
    Ok(())
}

/// The Text screen is the HD's own: the original keeps its Menu list while the HD holds a speed, B puts nothing
/// in the game, and A has the original's own Text screen take the speed (its cursor moved with the d-pad, then A,
/// which stores it in `$01D0` and closes the menu). Options > Gameplay shows the same byte (a view of it).
fn menu_text(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    use ssd_menus::Screen;
    menu_list(h)?;
    for _ in 0..3 {
        h.key(Key::Down)?;
    }
    let old = h.byte(d::TEXT_SPEED);
    h.key(Key::Confirm)?;
    r.check(
        h.driver.curtain() == Curtain::Up && h.driver.nav().screen() == Screen::Text,
        "Text opens the HD Text screen",
    );
    r.check(h.byte(d::MENU_SCREEN) == 4, "the original keeps its Menu list under it");
    r.check(h.driver.nav().text().speed() == old, "the screen opens on the game's speed");
    let (other, key) = if old < 4 { (old + 1, Key::Right) } else { (old - 1, Key::Left) };
    h.key(key)?;
    r.check(h.driver.nav().text().speed() == other, "Left and Right move the speed");
    r.check(h.byte(d::TEXT_SPEED) == old, "moving writes nothing in the game");
    h.frames(20)?;
    r.check(h.driver.nav().text().frames() >= 20, "the preview runs");
    h.key(Key::Cancel)?;
    r.check(h.driver.nav().screen() == Screen::Menu, "B goes back to the Menu list");
    r.check(h.byte(d::TEXT_SPEED) == old, "B leaves the old speed");
    r.check(h.byte(d::MENU_SCREEN) == 4, "the original is still on its Menu list");
    h.key(Key::Confirm)?;
    h.key(key)?;
    h.key(Key::Confirm)?;
    h.frames(SETTLE)?;
    r.check(h.byte(d::TEXT_SPEED) == other, "A has the original's screen take the new speed");
    r.check(h.byte(d::FIELD_STATE) == 0, "the original's Text screen returns to the field");
    r.check(h.driver.curtain() == Curtain::Down, "the curtain is down");
    // Reopened, the screen opens on the speed that was kept.
    menu_list(h)?;
    for _ in 0..3 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().text().speed() == other, "the screen reopens on the kept speed");
    // From one end to the other: the cursor of the original's screen goes the whole way, Left and Right.
    for (key, end) in [(Key::Left, 0), (Key::Right, 4), (Key::Left, 0)] {
        for _ in 0..4 {
            h.key(key)?;
        }
        h.key(Key::Confirm)?;
        h.frames(SETTLE)?;
        r.check(h.byte(d::TEXT_SPEED) == end, &format!("the whole way to speed {}", end + 1));
        r.check(h.byte(d::FIELD_STATE) == 0, "and the menu is closed");
        menu_list(h)?;
        for _ in 0..3 {
            h.key(Key::Down)?;
        }
        h.key(Key::Confirm)?;
    }
    for _ in 0..3 {
        h.key(Key::Cancel)?;
    }
    r.check(h.byte(d::FIELD_STATE) == 0, "the menu closes");
    Ok(())
}

/// The slots of the cartridge as the game's save has them: whether each holds a game.
fn slots_used(h: &Harness) -> [bool; 3] {
    let file = h.machine.sram().and_then(|sram| ssd_save::SaveFile::from_bytes(sram).ok());
    std::array::from_fn(|slot| {
        file.as_ref()
            .and_then(|file| file.status(slot).ok())
            .is_some_and(|status| status != ssd_save::SlotStatus::Empty)
    })
}

/// The texts of the HD screen now.
fn screen_texts(h: &Harness) -> Vec<String> {
    h.driver
        .scene(h.machine.wram())
        .map(|list| list.texts().map(|(_, _, text)| text.to_owned()).collect())
        .unwrap_or_default()
}

/// Saves to `slot` (0 to 2) through the HD Save screen, from the open screen with its cursor on the first slot: the
/// cursor goes to the slot, A, and Yes if the slot holds a game and the screen asks. The original runs its own Save
/// (its entry, its question, the slot); the call ends when the original shows "Completed!". Returns whether it asked.
pub(crate) fn hd_save_to(h: &mut Harness, slot: usize) -> anyhow::Result<bool> {
    for _ in 0..slot {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    let asked = h.driver.nav().save().ask().is_some();
    if asked {
        h.key(Key::Left)?;
        h.key(Key::Confirm)?;
    }
    Ok(asked)
}

/// Opens the HD Save screen from the field (the Menu list's fifth row).
pub(crate) fn open_hd_save(h: &mut Harness) -> anyhow::Result<()> {
    menu_list(h)?;
    for _ in 0..4 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)
}

/// Saves the game to slot 1 through the HD Save screen from the field, as the scenarios that need a save made do (the
/// cheats, the bag): the original's own Save runs to Completed, and the game is back on the field.
pub(crate) fn menu_save(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    open_hd_save(h)?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Save, "Save opens the HD Save screen");
    hd_save_to(h, 0)?;
    r.check(
        h.byte(d::MENU_SCREEN) == 10 && h.byte(d::MENU_STEP) == 2,
        "the original's Save runs to Completed",
    );
    h.key(Key::Confirm)?;
    r.check(h.byte(d::FIELD_STATE) == 0, "Save returns to the field");
    Ok(())
}

/// Menu > Save is the HD's own: the screen shows the slots and holds the choice while the original stays on its Menu
/// list (its first "Save?" is never asked); B leaves with nothing written; A on a slot that holds a game asks "Overwrite"
/// with the cursor on No; the save is the original's own presses, and a third A closes the menu.
fn menu_save_screen(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    use ssd_menus::Screen;
    let used = slots_used(h);
    let (Some(empty), Some(held)) = (used.iter().position(|&u| !u), used.iter().position(|&u| u))
    else {
        bail!("menu-save needs a cartridge with a used slot and an empty one: {used:?}");
    };
    open_hd_save(h)?;
    r.check(h.driver.nav().screen() == Screen::Save, "Save opens the HD Save screen");
    r.check(h.driver.curtain() == Curtain::Up, "the HD screen has the curtain");
    r.check(h.byte(d::MENU_SCREEN) == 4, "the original keeps its Menu list: no \"Save?\" question");
    let text = screen_texts(h);
    r.check(
        text.iter().any(|t| t == "Empty") && text.iter().any(|t| t.starts_with("LV ")),
        &format!("the screen marks an empty slot and shows what a used one holds: {text:?}"),
    );
    let cartridge = h.machine.sram().map(<[u8]>::to_vec);
    // B goes back with nothing written.
    h.key(Key::Cancel)?;
    r.check(h.driver.nav().screen() == Screen::Menu, "B goes back to the Menu list");
    r.check(h.machine.sram().map(<[u8]>::to_vec) == cartridge, "B saved nothing");
    h.key(Key::Confirm)?;
    // A on a slot that holds a game asks first, with the cursor on No, and writes nothing.
    for _ in 0..held {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().save().ask() == Some(1), "A on a used slot asks, the cursor on No");
    r.check(
        screen_texts(h).iter().any(|t| t == &format!("Overwrite slot {:02}?", held + 1)),
        "the question names the slot",
    );
    r.check(h.byte(d::MENU_SCREEN) == 4, "the original has been sent nothing");
    h.key(Key::Confirm)?;
    r.check(
        h.driver.nav().save().ask().is_none() && h.machine.sram().map(<[u8]>::to_vec) == cartridge,
        "A on No closes the question and saves nothing",
    );
    // A on an empty slot saves at once, through the original's own screen.
    for _ in 0..(empty + 3 - held) % 3 {
        h.key(Key::Down)?;
    }
    let asked = hd_save_to(h, 0)?;
    r.check(!asked, "an empty slot is saved to at once");
    r.check(
        h.byte(d::MENU_SCREEN) == 10 && h.byte(d::MENU_STEP) == 2,
        "the original's Save wrote the slot: it shows Completed",
    );
    let after = slots_used(h);
    r.check(
        after[empty] && (0..3).all(|slot| slot == empty || after[slot] == used[slot]),
        "only the slot chosen was written",
    );
    r.check(
        screen_texts(h).iter().any(|t| t == &format!("Saved to slot {:02}.", empty + 1)),
        "the HD says where it saved",
    );
    r.check(h.driver.curtain() == Curtain::Up, "the HD message holds the curtain");
    // A takes the game back to the field.
    h.key(Key::Confirm)?;
    r.check(h.byte(d::FIELD_STATE) == 0, "Save returns to the field");
    r.check(h.driver.curtain() == Curtain::Down, "the curtain is down");
    // The slot just written holds a game now: A on it asks, Yes overwrites it.
    open_hd_save(h)?;
    for _ in 0..empty {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().save().ask() == Some(1), "the slot just written asks to be overwritten");
    h.key(Key::Left)?;
    h.key(Key::Confirm)?;
    r.check(h.byte(d::MENU_SCREEN) == 10 && h.byte(d::MENU_STEP) == 2, "Yes saves again");
    h.key(Key::Confirm)?;
    r.check(h.byte(d::FIELD_STATE) == 0, "and the game is back on the field");
    Ok(())
}

fn cancel_command(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.open()?;
    h.key(Key::Cancel)?;
    r.check(h.driver.curtain() == Curtain::Down, "cancel closes the menu");
    r.check(h.byte(d::FIELD_STATE) == 0, "the field is free");
    Ok(())
}

fn cancel_menu(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    menu_list(h)?;
    h.key(Key::Cancel)?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Command, "cancel leaves the Menu list");
    r.check(h.byte(d::MENU_SCREEN) == 0, "the original is on its command window");
    r.check(h.driver.nav().cursor() == 4, "the command window keeps the Menu row");
    h.key(Key::Cancel)?;
    r.check(h.driver.curtain() == Curtain::Down, "a second cancel closes the menu");
    Ok(())
}

/// The Menu list's Cheats opens the HD cheats screen: nothing is sent to the original, which keeps its Menu list
/// under it, and B goes back to the Menu list. Options opens the HD options screen the same way.
fn menu_cheats(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    menu_list(h)?;
    for _ in 0..5 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Cheats, "Cheats opens the cheats screen");
    r.check(h.driver.curtain() == Curtain::Up, "the cheats screen is the HD's: the curtain is up");
    r.check(h.byte(d::MENU_SCREEN) == 4, "nothing is sent to the original for Cheats");
    r.check(!h.driver.busy(), "no choice runs for Cheats");
    h.key(Key::Cancel)?;
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Menu, "B goes back to the Menu list");
    r.check(h.driver.nav().cursor() == 5, "the Menu list keeps its cursor on Cheats");
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(
        h.driver.nav().screen() == ssd_menus::Screen::Options,
        "Options opens the options screen",
    );
    r.check(h.byte(d::MENU_SCREEN) == 4 && !h.driver.busy(), "nor for Options");
    h.key(Key::Cancel)?;
    r.check(
        h.driver.nav().screen() == ssd_menus::Screen::Menu,
        "B goes back to the Menu list from the options",
    );
    Ok(())
}

/// The flying command window: Land, Item and Menu; Item and Menu go to the same screens as on foot.
fn fly_menu(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.open()?;
    h.key(Key::Down)?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    h.frames(400)?;
    h.open()?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(h.byte(d::MENU_SCREEN) == 2, "flying: Item opens the item list");
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    r.check(h.driver.curtain() == Curtain::Down, "flying: the menu closes");
    Ok(())
}

/// A macro that cannot reach its state aborts, and the field is playable again: the plan is run by
/// hand with an expectation the Item list never meets.
fn abort(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    use ssd_game::menu::presses::{Button, Player, Step, Until};
    h.open()?;
    let mut player = Player::new(vec![
        Step::Row { row: 3, rows: 5 },
        Step::Press { button: Button::A, until: Until::Screen(9) },
    ]);
    // The plan only reads the menu's state, so the labels are not needed.
    let labels = ssd_menus::Labels::default();
    let outcome = loop {
        let pad = {
            let probe = ssd_game::MenuProbe::new(h.machine.wram(), &labels);
            player.next(&probe)
        };
        match pad {
            Next::Frame(pad) => h.machine.run_frame(pad, &mut NullAudio)?,
            Next::Stage(_) | Next::Lift => {}
            Next::Done(outcome) => break outcome,
        }
    };
    r.check(
        matches!(outcome, Outcome::Aborted { .. }),
        "a plan that cannot reach its state aborts and closes the menu",
    );
    r.check(h.byte(d::FIELD_STATE) == 0, "the field is free after the abort");
    let leader = |h: &Harness| (h.word(LEADER_X), h.word(LEADER_Y));
    let before = leader(h);
    for _ in 0..30 {
        h.machine.run_frame(Joypad { right: true, ..Joypad::default() }, &mut NullAudio)?;
    }
    r.check(leader(h) != before, "the party walks again after the abort");
    Ok(())
}

/// The town's command window (screen 13): Talk, Look, Item and Menu, four rows and no Fly. It starts
/// from the shop replay's frame before its town menu opens.
fn town_open(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.open()?;
    r.check(h.byte(d::MENU_SCREEN) == 13, "the town's command window is screen 13");
    r.check(
        h.driver.nav().screen() == ssd_menus::Screen::Command,
        "the HD shows the command window",
    );
    for _ in 0..4 {
        h.key(Key::Down)?;
    }
    r.check(h.driver.nav().cursor() == 0, "four rows: Down wraps after Menu, with no Fly");
    Ok(())
}

fn town_item(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    town_open(h, r)?;
    h.key(Key::Down)?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(h.driver.curtain() == Curtain::Up, "Item in town opens the HD item list");
    r.check(h.byte(d::MENU_SCREEN) == 2, "the original is on its Item screen");
    h.key(Key::Cancel)?;
    r.check(h.byte(d::MENU_SCREEN) == 13, "cancel brings back the town's command window");
    h.key(Key::Cancel)?;
    r.check(h.driver.curtain() == Curtain::Down && h.byte(d::FIELD_STATE) == 0, "cancel closes it");
    Ok(())
}

fn town_menu(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    town_open(h, r)?;
    h.key(Key::Down)?;
    h.key(Key::Down)?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(
        h.driver.nav().screen() == ssd_menus::Screen::Menu,
        "Menu in town opens the HD Menu list",
    );
    h.key(Key::Confirm)?;
    r.check(
        h.driver.curtain() == Curtain::Up && h.driver.nav().screen() == ssd_menus::Screen::Cards,
        "Cards opens the HD hand",
    );
    r.check(h.byte(d::MENU_SCREEN) == 6, "over the original's Cards screen");
    h.frames(SETTLE)?;
    h.driver.close_menu();
    h.until_idle()?;
    h.frames(SETTLE)?;
    r.check(h.byte(d::FIELD_STATE) == 0, "Start closes back to the field");
    Ok(())
}

fn town_talk(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    town_open(h, r)?;
    h.key(Key::Confirm)?;
    r.check(
        h.driver.curtain() == Curtain::Lifted,
        "Talk in town lifts the curtain for its message",
    );
    // The shopkeeper's talk runs over several messages: A goes on until the field is free again.
    h.frames(MESSAGE_RUN)?;
    for _ in 0..12 {
        if h.byte(d::FIELD_STATE) == 0 {
            break;
        }
        h.press(a())?;
        h.frames(MESSAGE_RUN)?;
    }
    h.frames(SETTLE)?;
    r.check(h.byte(d::FIELD_STATE) == 0, "the message closes and the field is free");
    Ok(())
}

/// The shop's Sell list shows the units of the stack under its cursor (a stack of three, then two after one sale), on the
/// bag's count: the original lists a stack as one row with no count, and the host draws it beside the price box.
fn town_sell(h: &mut Harness, r: &mut Report, rom: &[u8]) -> anyhow::Result<()> {
    h.frames(2)?;
    r.check(h.driver.bag().is_some(), "the bag is bound in the town");
    // The list's first row (a stack the original lets a shop buy) holds three units.
    let object = h.byte(d::ITEM_LIST);
    let held = |h: &Harness| h.driver.bag().map_or(0, |bag| bag.units_of(object));
    while held(h) < 3 {
        append_entry(h, rom, object)?;
        h.frames(2)?;
    }
    let units = held(h);
    r.check(units >= 3, &format!("a stack of {units} of object {object} is in the bag"));
    r.check(h.driver.sell_count(h.machine.wram()).is_none(), "no count outside the shop");
    // Talk to the shopkeeper: A goes on through the messages to the Buy / Sell choice, then Sell and its list.
    town_open(h, r)?;
    h.key(Key::Confirm)?;
    let list_up = |h: &Harness| {
        h.byte(d::SHOP_MODE) & 0xBF == 0x81
            && h.byte(d::SHOP_STEP) == 2
            && h.byte(d::MENU_DRAW_DONE) & 0x80 != 0
    };
    let mut chose = false;
    for _ in 0..40 {
        h.frames(MESSAGE_RUN / 2)?;
        if list_up(h) {
            break;
        }
        // The Buy / Sell choice (mode 0, its question waiting): Sell is the second row.
        if h.byte(d::SHOP_MODE) == 0 && h.byte(d::MESSAGE_STATE) == 0x91 && !chose {
            h.press(Joypad { down: true, ..Joypad::default() })?;
            h.frames(20)?;
            chose = true;
        }
        h.press(a())?;
    }
    r.check(list_up(h), "the Sell list is up");
    h.frames(30)?;
    let count = h.driver.sell_count(h.machine.wram());
    r.check(
        count.map(u32::from) == Some(units),
        &format!("the Sell list shows x{units}: {count:?}"),
    );
    let wram = h.machine.wram().to_vec();
    r.check(
        ssd_game::menu::MenuProbe::new(&wram, &ssd_menus::Labels::default()).sell_row() == Some(0),
        "the cursor is on the first row",
    );
    // Sell the units one by one: A on the row, then the shop's messages, until its list is back.
    let before = h.driver.bag().map_or(0, |bag| bag.stacks().len());
    for left in (0..units).rev() {
        h.press(a())?;
        for _ in 0..30 {
            h.frames(MESSAGE_RUN / 2)?;
            if list_up(h) {
                break;
            }
            h.press(a())?;
        }
        h.frames(30)?;
        r.check(held(h) == left, &format!("the stack keeps {left} unit(s): {}", held(h)));
        r.check(list_is_projection(h), "the list is the bag's projection");
        if left > 0 {
            let count = h.driver.sell_count(h.machine.wram());
            let first = h.byte(d::ITEM_LIST);
            r.check(
                first == object
                    && h.byte(d::MENU_ITEM_ID) == object
                    && count.map(u32::from) == Some(left),
                &format!("the shop's list keeps the stack with x{left}: {count:?}"),
            );
        }
    }
    let after = h.driver.bag().map_or(0, |bag| bag.stacks().len());
    r.check(after + 1 == before, "the last unit takes the row off the list and the bag");
    r.check(h.byte(d::ITEM_LIST) != object, "the sold object is not on the list any more");
    Ok(())
}

/// The town scenarios, from the shop replay.
const TOWN_SCENARIOS: [(&str, Scenario); 3] =
    [("town-item", town_item), ("town-menu", town_menu), ("town-talk", town_talk)];

/// The town scenarios that need the bag (bound as the window binds it) and a start of their own, run by name.
fn sell_suite(
    rom: &[u8],
    starts: &Starts,
    only: &[String],
    out: &mut String,
) -> anyhow::Result<bool> {
    // It starts from the shopkeeper (the shop replay at frame 65340), not from the town the others start in: named only.
    if !only.iter().any(|o| o == "town-sell") {
        return Ok(true);
    }
    let mut report = Report::default();
    let mut h = bag_harness(rom, starts, &Sidecar::empty())?;
    if let Err(error) = town_sell(&mut h, &mut report, rom) {
        report.check(false, &format!("the scenario failed: {error:#}"));
    }
    let ok = report.passed();
    writeln!(out, "{} town-sell", if ok { "ok  " } else { "FAIL" })?;
    for (passed, what) in &report.lines {
        if !passed {
            writeln!(out, "       - {what}")?;
        }
    }
    Ok(ok)
}

/// The leader's position on the map, x and y words (`re/symbols.toml`, the party's position).
const LEADER_X: u32 = 0x7E_01A2;
const LEADER_Y: u32 = 0x7E_01A4;

/// The field suite: every scenario from the start, and its checks as a report.
fn field_suite(
    rom: &[u8],
    starts: &Starts,
    only: &[String],
    out: &mut String,
) -> anyhow::Result<bool> {
    scenario_suite(rom, starts, &FIELD_SCENARIOS, "field", only, out)
}

/// The scenarios of a suite, each from the start, with its checks as a report.
fn scenario_suite(
    rom: &[u8],
    starts: &Starts,
    scenarios: &[(&str, Scenario)],
    label: &str,
    only: &[String],
    out: &mut String,
) -> anyhow::Result<bool> {
    let mut all_ok = true;
    let mut ran = 0;
    for &(name, scenario) in scenarios {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        ran += 1;
        let mut report = Report::default();
        let mut h = harness(rom, starts)?;
        let result = scenario(&mut h, &mut report);
        if let Err(error) = &result {
            report.check(false, &format!("the scenario failed: {error:#}"));
        }
        let ok = report.passed();
        all_ok &= ok;
        writeln!(out, "{} {name}", if ok { "ok  " } else { "FAIL" })?;
        for (passed, what) in &report.lines {
            if !passed {
                writeln!(out, "       - {what}")?;
            }
        }
    }
    writeln!(
        out,
        "menu-test {label}: {ran} scenarios, {}",
        if all_ok { "all ok" } else { "FAILED" }
    )?;
    Ok(all_ok)
}

/// The game's own state the Classic check compares, as ranges of work RAM: the items, the money,
/// the party, the party's mode and its map. The timers and the random generators are left out.
const GAME_STATE: [(u32, u32); 8] = [
    (0x7E_0100, 0x30),  // item_list
    (0x7E_01B2, 0x03),  // item_count and zeni (a word)
    (0x7E_0190, 0x10),  // party_order
    (0x7E_0B00, 0x120), // party_records
    (0x7E_0D55, 0x01),  // travel_mode
    (0x7E_01A0, 0x01),  // map_id
    (0x7E_01A2, 0x04),  // the leader's position
    (0x7E_01D0, 0x01),  // text_speed
];

/// The part of work RAM the ranges of `GAME_STATE` cover, as (address, value) pairs.
fn game_state(machine: &Machine) -> Vec<(u32, u8)> {
    let wram = machine.wram();
    GAME_STATE
        .iter()
        .flat_map(|&(start, len)| {
            (start..start + len).map(move |address| (address, wram[(address - WRAM_BASE) as usize]))
        })
        .collect()
}

/// One choice and its checks, against the Classic presses of the same choice.
struct Case {
    name: &'static str,
    /// The choice, played through the driver (its presses are the driver's).
    hd: fn(&mut Harness) -> anyhow::Result<()>,
    /// The same presses as a Classic timeline in the style of the reference scripts (`menus.txt`):
    /// the menu opens with A on the field, Down moves the cursor, A chooses, and the waits let each
    /// screen draw.
    classic: &'static str,
}

/// The choices the differential checks: the command window, the Menu list, the flying window, and
/// the cancels.
const CASES: [Case; 14] = [
    Case {
        name: "talk",
        hd: |h| {
            h.open()?;
            h.key(Key::Confirm)
        },
        classic: "4 a\n120 -\n4 a\n260 -\n",
    },
    Case {
        name: "look",
        hd: |h| {
            h.open()?;
            h.key(Key::Down)?;
            h.key(Key::Confirm)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 a\n260 -\n",
    },
    Case {
        name: "item",
        hd: |h| {
            h.open()?;
            for _ in 0..3 {
                h.key(Key::Down)?;
            }
            h.key(Key::Confirm)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n150 -\n",
    },
    Case {
        name: "cancel-item",
        hd: |h| {
            h.open()?;
            for _ in 0..3 {
                h.key(Key::Down)?;
            }
            h.key(Key::Confirm)?;
            h.key(Key::Cancel)?;
            h.key(Key::Cancel)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n150 -\n4 b\n120 -\n4 b\n120 -\n",
    },
    Case {
        name: "menu-cards",
        hd: |h| {
            menu_list(h)?;
            h.key(Key::Confirm)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n120 -\n4 a\n200 -\n",
    },
    Case {
        name: "menu-status",
        hd: |h| {
            menu_list(h)?;
            h.key(Key::Down)?;
            h.key(Key::Confirm)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n120 -\n4 down\n30 -\n4 a\n200 -\n",
    },
    Case {
        name: "status-start",
        hd: |h| {
            menu_list(h)?;
            h.key(Key::Down)?;
            h.key(Key::Confirm)?;
            h.driver.close_menu();
            h.frame(Joypad::default())?;
            h.until_idle()
        },
        classic: "4 a
120 -
4 down
30 -
4 down
30 -
4 down
30 -
4 down
30 -
4 a
120 -
4 down
30 -
4 a
200 -
4 b
200 -
4 b
200 -
4 b
200 -
",
    },
    Case {
        name: "menu-order",
        hd: |h| {
            menu_list(h)?;
            for _ in 0..2 {
                h.key(Key::Down)?;
            }
            // The HD's Order screen: the second member to the front, then Apply order (the original's own screen takes it:
            // Down to the second member, A, the last one is taken by itself, A on Yes).
            h.key(Key::Confirm)?;
            for key in [Key::Down, Key::Confirm, Key::Up, Key::Confirm, Key::Down, Key::Down] {
                h.key(key)?;
            }
            h.key(Key::Confirm)
        },
        classic: "4 a
120 -
4 down
30 -
4 down
30 -
4 down
30 -
4 down
30 -
4 a
120 -
4 down
30 -
4 down
30 -
4 a
120 -
4 down
30 -
4 a
120 -
4 a
200 -
",
    },
    Case {
        name: "menu-text",
        hd: |h| {
            menu_list(h)?;
            for _ in 0..3 {
                h.key(Key::Down)?;
            }
            // The HD's Text screen: one step slower, kept with A (the original's screen then takes it).
            h.key(Key::Confirm)?;
            h.key(Key::Right)?;
            h.key(Key::Confirm)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n120 -\n4 right\n30 -\n4 a\n200 -\n",
    },
    Case {
        name: "menu-save",
        hd: |h| {
            // The HD Save screen: the third slot, saved to through the original's own Save (its question is answered by the
            // HD, which never asks it).
            open_hd_save(h)?;
            hd_save_to(h, 2)?;
            h.key(Key::Confirm)
        },
        classic: "4 a
120 -
4 down
30 -
4 down
30 -
4 down
30 -
4 down
30 -
4 a
120 -
4 down
30 -
4 down
30 -
4 down
30 -
4 down
30 -
4 a
120 -
4 a
120 -
4 down
30 -
4 down
30 -
4 a
200 -
4 a
200 -
",
    },
    Case {
        name: "menu-save-over",
        hd: |h| {
            // The first slot: it holds a game, so the HD asks first (Yes), then the original saves as above.
            open_hd_save(h)?;
            let asked = hd_save_to(h, 0)?;
            anyhow::ensure!(asked, "slot 1 of the start holds a game: the HD asks before it saves");
            h.key(Key::Confirm)
        },
        classic: "4 a
120 -
4 down
30 -
4 down
30 -
4 down
30 -
4 down
30 -
4 a
120 -
4 down
30 -
4 down
30 -
4 down
30 -
4 down
30 -
4 a
120 -
4 a
120 -
4 a
200 -
4 a
200 -
",
    },
    Case {
        name: "fly",
        hd: |h| {
            h.open()?;
            h.key(Key::Down)?;
            h.key(Key::Down)?;
            h.key(Key::Confirm)?;
            h.until_idle()
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 a\n400 -\n",
    },
    Case {
        name: "land",
        hd: |h| {
            h.open()?;
            h.key(Key::Down)?;
            h.key(Key::Down)?;
            h.key(Key::Confirm)?;
            h.until_idle()?;
            h.frames(400)?;
            h.open()?;
            h.key(Key::Confirm)?;
            h.until_idle()
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 a\n400 -\n4 a\n120 -\n4 a\n400 -\n",
    },
    Case {
        name: "cancel-menu",
        hd: |h| {
            menu_list(h)?;
            h.key(Key::Cancel)?;
            h.key(Key::Cancel)
        },
        classic: "4 a\n120 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 down\n30 -\n4 a\n120 -\n4 b\n120 -\n4 b\n120 -\n",
    },
];

/// Checks one choice: the joypad words the driver played for it, replayed on a plain machine with
/// no driver, end in exactly the same work RAM (every byte), frame for frame. So a choice is nothing
/// but presses, and a recorded replay of it is a Classic replay. Returns the replayed machine.
fn tape_check(
    rom: &[u8],
    starts: &Starts,
    case: &Case,
    out: &mut String,
) -> anyhow::Result<(bool, Machine)> {
    let mut played = harness(rom, starts)?;
    played.tape = Some(Vec::new());
    (case.hd)(&mut played)?;
    played.frames(SETTLE)?;
    let tape = played.tape.take().unwrap_or_default();
    let mut replayed = (starts.fresh)()?;
    replayed.load_state(&starts.state)?;
    for pad in &tape {
        replayed.run_frame(*pad, &mut NullAudio)?;
    }
    let (a, b) = (played.machine.wram(), replayed.wram());
    let differing: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
    let same_frames = played.machine.frame_count() == replayed.frame_count();
    // The battery save too: a Save the HD plays writes the same bytes as the same presses on a plain machine.
    let sram_differs = played.machine.sram() != replayed.sram();
    let ok = differing.is_empty() && same_frames && !sram_differs;
    let shown: Vec<String> = differing
        .iter()
        .take(12)
        .map(|&i| format!("{:06X} {:02X}/{:02X}", WRAM_BASE + i as u32, a[i], b[i]))
        .collect();
    let presses = usize::from(tape.first().is_some_and(|pad| pad.a))
        + tape.windows(2).filter(|w| !w[0].a && w[1].a).count();
    writeln!(
        out,
        "{} {} tape: {} frames, {presses} presses of A; replayed as Classic, {} bytes of work RAM differ{}{}, the battery save {}",
        if ok { "ok  " } else { "FAIL" },
        case.name,
        tape.len(),
        differing.len(),
        if shown.is_empty() { "" } else { ": " },
        shown.join(" "),
        if sram_differs { "DIFFERS" } else { "is equal" }
    )?;
    Ok((ok, replayed))
}

/// Checks that the hand-written Classic timeline of a choice reaches the same game state (items,
/// money, party, travel mode, map and position) as the choice's own presses, at the same frame.
fn classic_check(
    rom: &[u8],
    starts: &Starts,
    case: &Case,
    by_tape: Machine,
    out: &mut String,
) -> anyhow::Result<bool> {
    let _ = rom;
    let mut by_hand = (starts.fresh)()?;
    by_hand.load_state(&starts.state)?;
    for pad in scripts::parse_timeline(case.classic).map_err(anyhow::Error::msg)? {
        by_hand.run_frame(pad, &mut NullAudio)?;
    }
    let mut by_tape = by_tape;
    let target = by_tape.frame_count().max(by_hand.frame_count());
    while by_hand.frame_count() < target {
        by_hand.run_frame(Joypad::default(), &mut NullAudio)?;
    }
    while by_tape.frame_count() < target {
        by_tape.run_frame(Joypad::default(), &mut NullAudio)?;
    }
    let (ours, theirs) = (game_state(&by_tape), game_state(&by_hand));
    let differing: Vec<String> = ours
        .iter()
        .zip(theirs.iter())
        .filter(|(x, y)| x != y)
        .take(12)
        .map(|(x, y)| format!("{:06X} {:02X}/{:02X}", x.0, x.1, y.1))
        .collect();
    // The battery save is compared whole: a Save made with the HD's presses writes what the original's own screen writes.
    let sram_differs = by_tape.sram() != by_hand.sram();
    let ok = differing.is_empty() && !sram_differs;
    writeln!(
        out,
        "{} {} Classic timeline: the game state at frame {target} {}{}, the battery save {}",
        if ok { "ok  " } else { "FAIL" },
        case.name,
        if ok { "is equal" } else { "differs:" },
        if ok { String::new() } else { format!(" {}", differing.join(" ")) },
        if sram_differs { "DIFFERS" } else { "is equal" }
    )?;
    Ok(ok)
}

/// The differential suite: each choice's presses against its recorded replay, and against its
/// hand-written Classic timeline.
fn diff_suite(
    rom: &[u8],
    starts: &Starts,
    only: &[String],
    out: &mut String,
) -> anyhow::Result<bool> {
    let mut all_ok = true;
    for case in &CASES {
        if !only.is_empty() && !only.iter().any(|o| o == case.name) {
            continue;
        }
        let (tape_ok, replayed) = tape_check(rom, starts, case, out)?;
        let hand_ok = classic_check(rom, starts, case, replayed, out)?;
        all_ok &= tape_ok && hand_ok;
    }
    writeln!(out, "menu-test diff: {}", if all_ok { "all ok" } else { "DIFFERENCES" })?;
    Ok(all_ok)
}

/// Runs the command: prints the report and returns the exit code (0 all ok, 1 a failure).
///
/// # Errors
///
/// The start cannot be played (the save, the replay, the ROM).
pub fn run(rom: &[u8], args: &Args, out: &mut dyn std::io::Write) -> anyhow::Result<u8> {
    if args.suite == "cheats" {
        let mut text = format!("{}\n", header(args));
        let ok = crate::cheats_test::suite(rom, &args.start, &args.only, &mut text)?;
        write!(out, "{text}")?;
        return Ok(u8::from(!ok));
    }
    if args.suite == "battle" {
        let mut text = format!("{}\n", header(args));
        let ok = battle_suite(rom, &args.start, &args.only, &mut text)?;
        write!(out, "{text}")?;
        return Ok(u8::from(!ok));
    }
    let starts = start_of(rom, &args.start)?;
    let mut text = format!("{}\n", header(args));
    let ok = match args.suite.as_str() {
        "field" => field_suite(rom, &starts, &args.only, &mut text)?,
        "town" => {
            let ok = scenario_suite(rom, &starts, &TOWN_SCENARIOS, "town", &args.only, &mut text)?;
            let sold = sell_suite(rom, &starts, &args.only, &mut text)?;
            ok && sold
        }
        "items" => {
            let Start::Save { path, .. } = &args.start else {
                bail!("menu-test items takes --save <file.sav>: its crafted starts come from it");
            };
            let base =
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            if let Some(dir) = &args.snapshots {
                record_snapshots(rom, &base, dir, &mut text)?;
            }
            let ok = items_suite(rom, &base, &args.only, &mut text);
            // Whatever the suite wrote is removed, whether it ran or failed.
            let _ = std::fs::remove_dir_all(scratch_dir());
            ok?
        }
        _ => diff_suite(rom, &starts, &args.only, &mut text)?,
    };
    write!(out, "{text}")?;
    Ok(u8::from(!ok))
}

/// Where the scenario starts, for the report.
#[must_use]
pub fn describe(start: &Start) -> String {
    match start {
        Start::Save { path, slot } => format!("save {} slot {}", path.display(), slot + 1),
        Start::Replay { path, frames } => format!("replay {} at frame {frames}", path.display()),
        Start::Scenarios { dir, save, .. } => format!(
            "scenarios in {}{}",
            dir.as_deref().unwrap_or(Path::new(".work/replays")).display(),
            save.as_ref().map_or_else(String::new, |(path, _)| format!(
                ", scripted loss from {}",
                path.display()
            )),
        ),
    }
}

/// The path of the cached ROM's identifier, for the report's header.
#[must_use]
pub fn header(args: &Args) -> String {
    format!("menu-test {}: from {}", args.suite, describe(&args.start))
}

// --- the bag (suite `items`): Enhanced mode on crafted saves --------------------------------------

/// The Item screen's start: the bag bound to the save with `sidecar` (as the window binds it), with
/// the bag's rules. The field is the start's; the first frame binds the bag.
fn bag_harness(rom: &[u8], starts: &Starts, sidecar: &Sidecar) -> anyhow::Result<Harness> {
    let mut h = harness_with(rom, starts, true)?;
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    h.driver = Driver::with_bag(
        ssd_catalogue::menu_labels(&catalogue),
        ssd_menus::bag::Rules::default(),
        sidecar.clone(),
    )
    .with_cheats(ssd_menus::cheats::CheatSidecar::empty());
    enhanced_hooks(&mut h.machine);
    Ok(h)
}

/// The hook table of an Enhanced run as the window installs it: the cheat wrappers are on it, so the menus are
/// played on the same routines the window runs (a Classic harness has none).
fn enhanced_hooks(machine: &mut Machine) {
    let mut table = machine.hook_table().cloned().unwrap_or_else(ssd_oracle::HookTable::new);
    ssd_game::cheats::install(&mut table);
    ssd_game::mods::install(&mut table);
    machine.set_hook_table(Some(table));
}

/// The save item of object `id`, with the attribute byte the game keeps for the object.
fn save_item(rom: &[u8], id: u8) -> anyhow::Result<ssd_save::Item> {
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    let item = catalogue.items.get(usize::from(id)).context("no such object")?;
    Ok(ssd_save::Item { id, attr: item.attribute })
}

/// The save `base` (a save file's bytes) with the items `ids` in slot 1, and the first party member
/// at `hp` when given. The checksum and the backup are written as the game writes them.
fn craft_save(rom: &[u8], base: &[u8], ids: &[u8], hp: Option<u16>) -> anyhow::Result<Vec<u8>> {
    let mut file = ssd_save::SaveFile::from_bytes(base)?;
    let mut slot = file.slot(0)?;
    let items = ids.iter().map(|&id| save_item(rom, id)).collect::<anyhow::Result<Vec<_>>>()?;
    slot.set_items(&items)?;
    if let (Some(hp), Some(&first)) = (hp, slot.party().first()) {
        let mut record = slot.record(first)?;
        record.set_hp(hp);
        slot.set_record(first, &record)?;
    }
    file.set_slot(0, &slot)?;
    Ok(file.as_bytes().to_vec())
}

/// The checksum word of slot 1 of a battery save (what the sidecar keys the bag by).
fn slot_checksum(sram: &[u8]) -> anyhow::Result<u16> {
    let raw: &[u8; ssd_save::SLOT_LEN] = sram
        .get(ssd_save::SLOT_BASES[0]..ssd_save::SLOT_BASES[0] + ssd_save::SLOT_LEN)
        .context("a save slot")?
        .try_into()?;
    Ok(ssd_save::checksum(raw))
}

/// The directory of the items suite's scratch files (crafted saves, the recorded replay): the
/// system's temporary directory, emptied when the suite ends.
fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join(format!("ssd-menu-test-{}", std::process::id()))
}

/// A scratch save file of `bytes`, for a scenario's start.
fn scratch_save(name: &str, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    let dir = scratch_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.sav"));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// The first party member's HP (the party order's first character, its record in use).
fn first_member_hp(h: &Harness) -> u16 {
    let character = h.byte(d::PARTY_ORDER);
    (0..9)
        .map(|index| d::PARTY_RECORDS + 32 * index)
        .find(|&record| h.byte(record) & 0x80 != 0 && h.byte(record + 0x1C) == character)
        .map_or(0, |record| h.word(record + 3))
}

/// The bag's stacks as (object, count) in bag order, and the original's list (its objects).
fn bag_summary(h: &Harness) -> (Vec<(u8, u16)>, Vec<u8>) {
    let stacks = h
        .driver
        .bag()
        .map_or_else(Vec::new, |bag| bag.stacks().iter().map(|s| (s.object, s.count)).collect());
    let count = usize::from(h.byte(d::ITEM_COUNT)).min(24);
    let list = (0..count).map(|i| h.byte(d::ITEM_LIST + 2 * i as u32)).collect();
    (stacks, list)
}

/// Whether the original's list is the bag's projection: the same objects, in order.
fn list_is_projection(h: &Harness) -> bool {
    let Some(bag) = h.driver.bag() else { return false };
    let objects: Vec<u8> = bag
        .project(None)
        .iter()
        .filter_map(|id| bag.stack(*id))
        .map(|stack| stack.object)
        .collect();
    bag_summary(h).1 == objects
}

/// The original's list as the game writes a pickup, a purchase or a battle drop: an entry appended
/// (object and its attribute byte), the count raised.
fn append_entry(h: &mut Harness, rom: &[u8], object: u8) -> anyhow::Result<()> {
    let count = h.byte(d::ITEM_COUNT);
    let item = save_item(rom, object)?;
    let at = d::ITEM_LIST + 2 * u32::from(count);
    h.machine.write_wram(at, &[item.id, item.attr])?;
    h.machine.write_wram(d::ITEM_COUNT, &[count + 1])?;
    Ok(())
}

/// Opens the bag: the field's Item command (row 3 of the command window), from the closed menu.
fn open_items(h: &mut Harness) -> anyhow::Result<()> {
    h.open()?;
    for _ in 0..3 {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    h.frames(20)
}

/// The shared start of the bag's scenarios: the crafted save and its Continue, the bag bound.
fn bag_start(
    rom: &[u8],
    base: &[u8],
    name: &str,
    ids: &[u8],
    hp: Option<u16>,
) -> anyhow::Result<Starts> {
    let crafted = craft_save(rom, base, ids, hp)?;
    let path = scratch_save(name, &crafted)?;
    start_of(rom, &Start::Save { path, slot: 0 })
}

/// Use Bulma from a stack of 5: one unit on the first member (the use-on list asks for the member, and A
/// chooses it), so the first member heals 25 and the stack keeps 4. The original's list is the bag's projection.
fn bag_use(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    let hp_before = first_member_hp(h);
    open_items(h)?;
    r.check(h.driver.bag().is_some(), "the bag is bound to the save");
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Items, "the HD shows the bag");
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().sheet() == Sheet::Choose, "Use or Drop for the stack");
    h.key(Key::Confirm)?;
    h.until_idle()?;
    // The fighter and the message are the player's, on the original's screens.
    h.answer()?;
    h.frames(SETTLE)?;
    let (stacks, _) = bag_summary(h);
    r.check(
        h.driver.last_outcome() == Some(Outcome::Reached),
        &format!(
            "the plan: {:?} (screen {} step {} busy {})",
            h.driver.last_outcome(),
            h.byte(d::MENU_SCREEN),
            h.byte(d::MENU_STEP),
            h.byte(d::MENU_BUSY)
        ),
    );
    r.check(h.byte(d::FIELD_STATE) == 0, "the menu closed after the use");
    let hp_after = first_member_hp(h);
    r.check(hp_after == hp_before + 25, &format!("one unit: +25 HP, {hp_before} to {hp_after}"));
    r.check(stacks.contains(&(2, 4)), &format!("a stack of 4 Bulma is left: {stacks:?}"));
    r.check(list_is_projection(h), "the original's list is the bag's projection");
    Ok(())
}

/// Use the second stack of the bag (Bulma, under a Roshi): the original uses Bulma, not the first entry of its list. The
/// original reads the entry under its cursor when the cursor moves, so the plan moves it onto the staged entry.
fn bag_use_second(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    let hp_before = first_member_hp(h);
    open_items(h)?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    h.key(Key::Confirm)?;
    h.until_idle()?;
    r.check(
        h.byte(d::MENU_ITEM_ID) == 2,
        &format!("the original uses Bulma: object {}", h.byte(d::MENU_ITEM_ID)),
    );
    h.answer()?;
    h.frames(SETTLE)?;
    let hp_after = first_member_hp(h);
    r.check(
        hp_after == hp_before + 25,
        &format!("Bulma heals 25, not Roshi's 50: {hp_before} to {hp_after}"),
    );
    let (stacks, _) = bag_summary(h);
    r.check(
        stacks.contains(&(1, 1)) && !stacks.iter().any(|&(object, _)| object == 2),
        &format!("Bulma leaves the bag and Roshi stays: {stacks:?}"),
    );
    Ok(())
}

/// A pickup while 21 stacks are held lands in the bag: the original's list keeps its 21 entries and
/// the 22nd stack is in the bag.
fn bag_pickup(h: &mut Harness, r: &mut Report, rom: &[u8]) -> anyhow::Result<()> {
    h.frames(2)?;
    r.check(h.driver.bag().map_or(0, |bag| bag.stacks().len()) == 21, "21 stacks are held");
    append_entry(h, rom, 21)?;
    h.frames(2)?;
    let (stacks, list) = bag_summary(h);
    r.check(stacks.len() == 22 && stacks.last() == Some(&(21, 1)), "the pickup is in the bag");
    r.check(list.len() == 21 && !list.contains(&21), "the list still shows 21 entries");
    r.check(list_is_projection(h), "the list is the bag's projection");
    Ok(())
}

/// A purchase adds to a stack the bag holds: the money is paid, the stack grows, the list does not.
fn bag_shop(h: &mut Harness, r: &mut Report, rom: &[u8]) -> anyhow::Result<()> {
    h.frames(2)?;
    let zeni = h.word(d::ZENI);
    append_entry(h, rom, 2)?;
    h.machine.write_wram(d::ZENI, &(zeni - 3).to_le_bytes())?;
    h.frames(2)?;
    let (stacks, _) = bag_summary(h);
    r.check(stacks.contains(&(2, 2)), &format!("the bought Bulma joins its stack: {stacks:?}"));
    r.check(list_is_projection(h), "the list is the bag's projection");
    r.check(h.word(d::ZENI) == zeni - 3, "the price is paid");
    Ok(())
}

/// Drop one Bulma from the stack of two: Drop one? asks Yes or No, the bag and the list follow, the Scouter
/// stays, and the Item screen is back.
fn bag_drop(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    open_items(h)?;
    h.key(Key::Confirm)?;
    h.key(Key::Down)?;
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().sheet() == Sheet::Confirm, "Drop one? asks for a Yes");
    h.key(Key::Up)?;
    h.key(Key::Confirm)?;
    h.until_idle()?;
    h.frames(SETTLE)?;
    let (stacks, list) = bag_summary(h);
    r.check(
        stacks == vec![(2, 1), (16, 1)],
        &format!("one Bulma is dropped, the Scouter stays: {stacks:?}"),
    );
    r.check(list == vec![2, 16], "the list shows both");
    r.check(h.driver.nav().screen() == ssd_menus::Screen::Items, "the Item screen is back");
    Ok(())
}

/// Save, then Continue: the bag of the save (more stacks than its list) comes back from the sidecar.
fn bag_save_continue(h: &mut Harness, r: &mut Report, rom: &[u8]) -> anyhow::Result<()> {
    h.frames(2)?;
    append_entry(h, rom, 21)?;
    h.frames(2)?;
    menu_save(h, r)?;
    let saved = h.machine.sram().context("the save")?.to_vec();
    let sidecar = h.driver.sidecar().clone();
    r.check(sidecar.saves.len() == 1, "the sidecar keeps the bag of the save");
    let starts = start_of(rom, &Start::Save { path: scratch_save("saved", &saved)?, slot: 0 })?;
    let mut again = bag_harness(rom, &starts, &sidecar)?;
    again.frames(2)?;
    let (stacks, _) = bag_summary(&again);
    r.check(stacks.len() == 22, &format!("Continue restores 22 stacks: {}", stacks.len()));
    r.check(stacks.last() == Some(&(21, 1)), "the stack that the list did not show is back");
    Ok(())
}

/// A Classic-modified save (its checksum no longer matches the sidecar's) rebuilds the bag from its
/// own list.
fn bag_classic_rebuild(
    h: &mut Harness,
    r: &mut Report,
    rom: &[u8],
    base: &[u8],
) -> anyhow::Result<()> {
    h.frames(2)?;
    append_entry(h, rom, 21)?;
    h.frames(2)?;
    menu_save(h, r)?;
    let sidecar = h.driver.sidecar().clone();
    let saved = h.machine.sram().context("the save")?.to_vec();
    let _ = base;
    // A change made by Classic play: slot 1 holds Kami and Puar instead.
    let edited = craft_save(rom, &saved, &[0, 4], None)?;
    let starts = start_of(rom, &Start::Save { path: scratch_save("classic", &edited)?, slot: 0 })?;
    let mut again = bag_harness(rom, &starts, &sidecar)?;
    again.frames(2)?;
    let (stacks, list) = bag_summary(&again);
    r.check(
        stacks == vec![(0, 1), (4, 1)],
        &format!("the bag is rebuilt from the list: {stacks:?}"),
    );
    r.check(list == vec![0, 4], "and the list is the same");
    Ok(())
}

/// A full bag (60 stacks, from the sidecar of the save) shows its notice, and the original's list is at
/// its full count of 24: the original refuses the next pickup by its own check, and nothing is added.
fn bag_full(h: &mut Harness, r: &mut Report, _rom: &[u8]) -> anyhow::Result<()> {
    h.frames(2)?;
    let stacks = h.driver.bag().map_or(0, |bag| bag.stacks().len());
    r.check(
        stacks == 60 && h.driver.bag().is_some_and(Bag::is_full),
        &format!("60 stacks: {stacks}"),
    );
    r.check(h.byte(d::ITEM_COUNT) == 24, "the original's list is full (24 entries)");
    open_items(h)?;
    let notice = h
        .driver
        .scene(h.machine.wram())
        .is_some_and(|scene| scene.texts().any(|(_, _, text)| text.starts_with("Bag full")));
    r.check(notice, "the Item screen says the bag is full");
    // The original adds nothing at 24 entries: no write is made for the refused pickup.
    h.frames(2)?;
    let (after, _) = bag_summary(h);
    r.check(after.len() == 60 && h.byte(d::ITEM_COUNT) == 24, "the 61st stack is refused");
    Ok(())
}

/// The sidecar of a full bag: 60 stacks of 60 different objects (one unit each), keyed by the checksum of the
/// crafted start (slot 1 with the first 21 objects). Units of one object join its stack, and a state object (the
/// Porunga, whose units stack by their state word) would not give 60 stacks, so each stack is a different object.
fn full_sidecar(rom: &[u8], base: &[u8]) -> anyhow::Result<Sidecar> {
    let crafted = craft_save(rom, base, &(0..21).collect::<Vec<u8>>(), None)?;
    let mut bag = Bag::new(Rules::default());
    for object in 1..=60_u8 {
        bag.add(object, 0, Class::of(object, false))
            .map_err(|_| anyhow::anyhow!("the bag is full too soon"))?;
    }
    let mut sidecar = Sidecar::empty();
    sidecar.store(0, slot_checksum(&crafted)?, &bag);
    Ok(sidecar)
}

/// The party's size: the entries of the party order in use.
fn party_len(h: &Harness) -> usize {
    (0..16_u32).take_while(|&i| h.byte(d::PARTY_ORDER + i) != 0xFF).count()
}

/// The record of the party's member in `row`: the record in use of the character the party order gives the row.
fn member_record(h: &Harness, row: u8) -> Option<u32> {
    let character = h.byte(d::PARTY_ORDER + u32::from(row));
    (0..9_u32)
        .map(|index| d::PARTY_RECORDS + 32 * index)
        .find(|&record| h.byte(record) & 0x80 != 0 && h.byte(record + 0x1C) == character)
}

/// The HP of the party's member in `row`.
fn member_hp(h: &Harness, row: u8) -> u16 {
    member_record(h, row).map_or(0, |record| h.word(record + 3))
}

/// Sets the HP and the maximum HP of the party's member in `row` (its record in work RAM), so a heal has room.
fn set_member_hp(h: &mut Harness, row: u8, hp: u16) -> anyhow::Result<()> {
    let record = member_record(h, row).context("a party member")?;
    h.machine.write_wram(record + 3, &hp.to_le_bytes())?;
    h.machine.write_wram(record + 10, &200_u16.to_le_bytes())?;
    Ok(())
}

/// Sets the KI and the maximum KI of the party's member in `row` (its record in work RAM), so a KI heal has room.
fn set_member_ki(h: &mut Harness, row: u8, ki: u16) -> anyhow::Result<()> {
    let record = member_record(h, row).context("a party member")?;
    h.machine.write_wram(record + 5, &ki.to_le_bytes())?;
    h.machine.write_wram(record + 12, &200_u16.to_le_bytes())?;
    Ok(())
}

/// A key of the HD that may start a plan: the original's (screen, step) of each frame of the plan.
fn key_traced(h: &mut Harness, key: Key) -> anyhow::Result<Vec<(u8, u8)>> {
    h.driver.key(key, h.machine.wram());
    let mut seen = Vec::new();
    for _ in 0..CHOICE_LIMIT {
        if !h.driver.busy() {
            return Ok(seen);
        }
        seen.push((h.byte(d::MENU_SCREEN), h.byte(d::MENU_STEP)));
        h.frame(Joypad::default())?;
    }
    bail!("a choice did not end in {CHOICE_LIMIT} frames (curtain {:?})", h.driver.curtain())
}

/// The Dragon Radar from the bag, as one use: the use is taken at once (no quantity, no member), the radar's
/// own screen stays up for the player (the curtain lifts for it), B leaves it, and the Radar stays in the bag.
fn bag_radar(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.frames(1)?;
    open_items(h)?;
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().sheet() == Sheet::Choose, "Use or Drop for the Radar");
    let trace = key_traced(h, Key::Confirm)?;
    r.check(h.driver.nav().sheet() == Sheet::Browse, "the Radar's use is taken at once");
    r.check(
        !trace.is_empty() && trace.iter().all(|&(screen, _)| screen == 2),
        &format!("the original's Item screen takes the use: {trace:?}"),
    );
    h.frames(SETTLE)?;
    r.check(
        h.driver.last_outcome() == Some(Outcome::Reached),
        &format!("the plan: {:?}", h.driver.last_outcome()),
    );
    r.check(
        h.driver.curtain() == Curtain::Lifted,
        "the radar's own screen is up: the curtain lifts for it",
    );
    r.check(h.byte(d::FIELD_STATE) != 0, "the menu is still open on the radar's screen");
    h.press(b())?;
    h.frames(SETTLE)?;
    r.check(h.byte(d::FIELD_STATE) == 0, "B leaves the radar: the field is free");
    r.check(h.driver.curtain() == Curtain::Down, "the curtain comes down with it");
    let (stacks, _) = bag_summary(h);
    r.check(stacks == vec![(23, 1)], &format!("the Radar stays in the bag: {stacks:?}"));
    Ok(())
}

/// A healing item from the bag, as the original takes it: Roshi (50 HP) heals the first fighter, the stack keeps two,
/// and the second fighter is left alone. The original asks for no fighter on the map.
fn bag_heal(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.frames(1)?;
    let party = party_len(h);
    r.check(party >= 2, &format!("the party has two fighters: {party}"));
    if party < 2 {
        return Ok(());
    }
    set_member_hp(h, 0, 10)?;
    set_member_hp(h, 1, 10)?;
    let before: Vec<u16> = (0..party as u8).map(|row| member_hp(h, row)).collect();
    open_items(h)?;
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().sheet() == Sheet::Choose, "Use or Drop for the stack");
    h.key(Key::Confirm)?;
    h.until_idle()?;
    // The player takes the first fighter of the original's use-on list (its cursor's row), and its message.
    h.answer()?;
    h.frames(SETTLE)?;
    let after: Vec<u16> = (0..party as u8).map(|row| member_hp(h, row)).collect();
    r.check(
        after[0] == before[0] + 50,
        &format!("the first fighter heals 50: {} to {}", before[0], after[0]),
    );
    r.check(
        after[1] == before[1],
        &format!("the second fighter keeps its HP: {} to {}", before[1], after[1]),
    );
    let (stacks, _) = bag_summary(h);
    r.check(stacks.contains(&(1, 2)), &format!("a stack of 2 Roshi is left: {stacks:?}"));
    r.check(h.byte(d::FIELD_STATE) == 0, "the menu closed after the use");
    Ok(())
}

/// An ally from the Allies tab on the map: Vegeta is a fighter of the story whose object is for battles, so the Allies
/// tab lists him and his use is greyed out: A says why, and the bag is the same afterwards.
fn bag_ally(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    h.frames(1)?;
    open_items(h)?;
    h.key(Key::Tab)?;
    r.check(h.driver.nav().tab() == ssd_menus::view::Tab::Allies, "the Allies tab is shown");
    h.key(Key::Confirm)?;
    r.check(
        h.driver.nav().sheet() == Sheet::Choose && h.driver.nav().choice() == 1,
        "Vegeta is greyed for use: A opens his Drop, the only choice on the map",
    );
    h.key(Key::Cancel)?;
    h.key(Key::Cancel)?;
    r.check(
        h.driver.nav().screen() == ssd_menus::Screen::Command,
        "B goes back to the command window",
    );
    let (stacks, _) = bag_summary(h);
    r.check(stacks.contains(&(19, 1)), &format!("Vegeta stays in the bag: {stacks:?}"));
    Ok(())
}

/// Every key of the Item screen does something: A on a tab with no stacks says so, the tabs move on, A on a stack
/// opens its sheet, B closes each level, and the menu closes at the command window. (The unit tests of
/// `ssd_menus::nav` walk the other screens and sheets.)
fn bag_dead_input(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    open_items(h)?;
    h.key(Key::Tab)?;
    r.check(
        h.driver.nav().tab() == ssd_menus::view::Tab::Allies,
        "the Allies tab has no stack in this bag",
    );
    h.key(Key::Confirm)?;
    r.check(h.driver.notice() == Some("Nothing on this tab."), "A on the empty tab says so");
    h.key(Key::Tab)?;
    h.key(Key::Tab)?;
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().sheet() == Sheet::Choose, "A on a stack opens its Use or Drop");
    h.key(Key::Cancel)?;
    r.check(h.driver.nav().sheet() == Sheet::Browse, "B closes the sheet");
    h.key(Key::Cancel)?;
    r.check(
        h.driver.curtain() == Curtain::Up && h.driver.nav().screen() == ssd_menus::Screen::Command,
        "B from the list goes back to the command window",
    );
    h.key(Key::Cancel)?;
    r.check(
        h.driver.curtain() == Curtain::Down && h.byte(d::FIELD_STATE) == 0,
        "B closes the menu",
    );
    Ok(())
}

/// Each object the HD offers for use, used once from its own bag, the party hurt (so a heal has room). The original
/// takes every use the HD offers: no "can't use" (step 8). A card's use leaves the original's card list to the
/// player, the Radar's own screen too, and every other use is taken, so the menu closes.
fn bag_kinds(_h: &mut Harness, r: &mut Report, rom: &[u8], base: &[u8]) -> anyhow::Result<()> {
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    let labels = ssd_catalogue::menu_labels(&catalogue);
    for item in &catalogue.items {
        let id = item.id;
        let Some(label) = labels.objects.get(usize::from(id)) else { continue };
        // The HD offers the object exactly when the rule of the driver says so (the battle effects are 27 to 31).
        if !label.field || label.story || label.attribute & 0x01 != 0 || (27..=31).contains(&id) {
            continue;
        }
        let starts = bag_start(rom, base, &format!("kind-{id}"), &[id], None)?;
        let mut h = bag_harness(rom, &starts, &Sidecar::empty())?;
        h.frames(1)?;
        for row in 0..party_len(&h) as u8 {
            set_member_hp(&mut h, row, 30)?;
            set_member_ki(&mut h, row, 5)?;
        }
        open_items(&mut h)?;
        h.key(Key::Confirm)?;
        let trace = key_traced(&mut h, Key::Confirm)?;
        // A card's list and the Radar's screen stay the player's; any other use is answered as a player does.
        if !label.card && id != DRAGON_RADAR_ID {
            h.answer()?;
        }
        h.frames(SETTLE)?;
        r.check(
            !trace.contains(&(2, 8)),
            &format!(
                "{} ({id}): the original takes the use, no \"can't use\": {trace:?}",
                label.name
            ),
        );
        let left =
            bag_summary(&h).0.iter().find(|&&(object, _)| object == id).map_or(0, |&(_, c)| c);
        r.check(
            h.driver.last_outcome() == Some(Outcome::Reached),
            &format!(
                "{} ({id}): the plan reaches its answer: {:?}",
                label.name,
                h.driver.last_outcome()
            ),
        );
        if label.card {
            r.check(
                h.driver.curtain() == Curtain::Lifted,
                &format!("{} ({id}): the card list is the player's", label.name),
            );
        } else if id == DRAGON_RADAR_ID {
            r.check(
                h.driver.curtain() == Curtain::Lifted && left == 1,
                &format!(
                    "{} ({id}): its screen is the player's and it stays in the bag ({left})",
                    label.name
                ),
            );
        } else {
            r.check(
                h.byte(d::FIELD_STATE) == 0 && left == 0,
                &format!(
                    "{} ({id}): the use is taken (stack {left}) and the menu closes: {trace:?}",
                    label.name
                ),
            );
        }
    }
    Ok(())
}

/// The Dragon Radar's object id: its screen is the player's to leave (see [`bag_radar`]).
const DRAGON_RADAR_ID: u8 = 23;

/// The save `base` with the items `ids` in slot 1 and the party's fighter in row `row` at `hp` (its record's current HP).
fn craft_save_with_hp(
    rom: &[u8],
    base: &[u8],
    ids: &[u8],
    row: usize,
    hp: u16,
) -> anyhow::Result<Vec<u8>> {
    let mut file = ssd_save::SaveFile::from_bytes(base)?;
    let mut slot = file.slot(0)?;
    let items = ids.iter().map(|&id| save_item(rom, id)).collect::<anyhow::Result<Vec<_>>>()?;
    slot.set_items(&items)?;
    let character = *slot.party().get(row).context("a fighter of the party")?;
    let mut record = slot.record(character)?;
    record.set_hp(hp);
    slot.set_record(character, &record)?;
    file.set_slot(0, &slot)?;
    Ok(file.as_bytes().to_vec())
}

/// The Use of the Dragon Radar, from the bag, as a recording: the Choose sheet, then the use.
fn flow_radar(h: &mut Harness, out: &mut String) -> anyhow::Result<()> {
    h.frames(30)?;
    open_items(h)?;
    h.key(Key::Confirm)?;
    writeln!(out, "radar: the Use or Drop sheet at frame {}", h.machine.frame_count())?;
    h.key(Key::Confirm)?;
    writeln!(
        out,
        "radar: the use is over at frame {}, curtain {:?}",
        h.machine.frame_count(),
        h.driver.curtain()
    )?;
    h.frames(SETTLE)?;
    writeln!(out, "radar: the radar's screen at frame {}", h.machine.frame_count())?;
    Ok(())
}

/// A healing item on the second fighter, as a recording: the use-on list, the fighter, the use.
fn flow_heal(h: &mut Harness, out: &mut String) -> anyhow::Result<()> {
    h.frames(30)?;
    open_items(h)?;
    h.key(Key::Confirm)?;
    writeln!(out, "heal: the Use or Drop sheet at frame {}", h.machine.frame_count())?;
    h.key(Key::Confirm)?;
    writeln!(out, "heal: the use started at frame {}", h.machine.frame_count())?;
    h.until_idle()?;
    h.frames(SETTLE)?;
    writeln!(
        out,
        "heal: the menu is {:?} at frame {}",
        h.driver.curtain(),
        h.machine.frame_count()
    )?;
    Ok(())
}

/// Records the Enhanced sessions the UI snapshots of the Use flow play back (`--snapshots DIR`): a Dragon Radar's use
/// and a healing item on the second fighter, each from the Continue of a crafted save. The sessions are recorded
/// unhooked (a recorder needs that) and played with the window's hooks, as the window plays them. Writes
/// `radar.ssdreplay` and `heal.ssdreplay` in `dir` and prints the frame of each step, so that a snapshot can be taken
/// at it (`ssd-recompiled --mode enhanced --replay FILE --headless-frames N --hd-scale 4 --ui-size 1488x956`).
/// A recorded Use flow: it plays the item menu's keys on the harness and writes its notes.
type Flow = fn(&mut Harness, &mut String) -> anyhow::Result<()>;

fn record_snapshots(rom: &[u8], base: &[u8], dir: &Path, out: &mut String) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let radar = craft_save(rom, base, &[23], None)?;
    let heal = craft_save_with_hp(rom, base, &[1, 1, 1], 0, 10)?;
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    let labels = ssd_catalogue::menu_labels(&catalogue);
    let flows: [(&str, Vec<u8>, Flow); 2] =
        [("radar", radar, flow_radar), ("heal", heal, flow_heal)];
    for (name, crafted, flow) in flows {
        let mut saves = MemorySaveStore::default();
        saves.0.insert("sav".to_owned(), crafted);
        let machine = Machine::new(rom.to_vec(), Box::new(saves))?;
        let recorder = Recorder::start(&machine)?.in_mode(MODE_ENHANCED);
        let mut h = Harness {
            machine,
            driver: Driver::with_bag(labels.clone(), Rules::default(), Sidecar::empty())
                .with_cheats(ssd_menus::cheats::CheatSidecar::empty()),
            tape: None,
            recorder: Some(recorder),
        };
        let continued =
            scripts::parse_timeline(&save::continue_prefix(0)).map_err(anyhow::Error::msg)?;
        for pad in continued {
            h.frame(pad)?;
        }
        flow(&mut h, out)?;
        let replay = h.recorder.take().context("the recording")?.into_replay();
        let path = dir.join(format!("{name}.ssdreplay"));
        std::fs::write(&path, replay.to_bytes(CORE_VERSION))?;
        writeln!(out, "recorded {} ({} frames)", path.display(), replay.frames())?;
    }
    Ok(())
}

/// The object whose help text has a run-time slot (`$FC`, `$43`): the original's message fills it in, so the menu shows
/// an ellipsis there until that message is traced (`docs/re/data/menus.md`, "Help text").
const RUNTIME_HELP: [u8; 4] = [10, 11, 27, 28];
/// The objects with no price: the original's price box holds a kanji mark there, which the menu does not draw yet.
const NO_PRICE: [u8; 11] = [23, 24, 26, 32, 33, 34, 35, 36, 37, 38, 39];
/// The characters whose corner face is not measured yet (Nail, Tsumuri and Maima: no party seen with them).
const UNSEEN_FACES: [usize; 3] = [8, 9, 10];

/// The pictures, prices and help the item and corner menus show, checked against the ROM: every object has its
/// picture and a help text with no placeholder in it, and every character with a measured face has its corner
/// picture. The named gaps are printed; any other gap fails the check.
fn pictures_check(rom: &[u8], out: &mut String) -> anyhow::Result<bool> {
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    let labels = ssd_catalogue::menu_labels(&catalogue);
    let view = ssd_catalogue::RomView::new(rom)?;
    let mut report = Report::default();
    for (id, object) in (0_u8..).zip(&labels.objects) {
        let name = &object.name;
        report.check(
            ssd_catalogue::object_portrait(view, id).is_ok(),
            &format!("object {id} ({name}): a picture"),
        );
        report.check(!object.help.is_empty(), &format!("object {id} ({name}): a help text"));
        let ellipsis = object.help.iter().any(|line| line.contains('\u{2026}'));
        report.check(
            !ellipsis || RUNTIME_HELP.contains(&id),
            &format!("object {id} ({name}): help text with a placeholder"),
        );
        let no_price = NO_PRICE.contains(&id);
        report.check(
            object.price.is_some() != no_price,
            &format!("object {id} ({name}): a price figure"),
        );
        if no_price {
            writeln!(
                out,
                "gap: object {id} ({name}): no price figure (the original's kanji mark is not drawn)"
            )?;
        }
        if RUNTIME_HELP.contains(&id) {
            writeln!(
                out,
                "gap: object {id} ({name}): help with a run-time slot shows an ellipsis"
            )?;
        }
    }
    for (index, character) in labels.characters.iter().enumerate() {
        let name = &character.name;
        match character.face {
            Some(face) => report.check(
                ssd_catalogue::face_portrait(view, face).is_ok(),
                &format!("character {index} ({name}): its corner face {face:02X}"),
            ),
            None => {
                report.check(
                    UNSEEN_FACES.contains(&index),
                    &format!("character {index} ({name}): a corner face"),
                );
                if UNSEEN_FACES.contains(&index) {
                    writeln!(out, "gap: character {index} ({name}): corner face not measured yet")?;
                }
            }
        }
    }
    let ok = report.passed();
    writeln!(out, "{} pictures", if ok { "ok  " } else { "FAIL" })?;
    for (passed, what) in &report.lines {
        if !passed {
            writeln!(out, "       - {what}")?;
        }
    }
    Ok(ok)
}

/// The bag's scenarios from the base save: each builds its own crafted start.
type BagScenario = fn(&mut Harness, &mut Report, &[u8], &[u8]) -> anyhow::Result<()>;

const BAG_SCENARIOS: [(&str, BagScenario); 14] = [
    ("original-items", |h, r, _, _| original_items(h, r)),
    ("use-bulma", |h, r, _, _| bag_use(h, r)),
    ("use-second", |h, r, _, _| bag_use_second(h, r)),
    ("radar", |h, r, _, _| bag_radar(h, r)),
    ("heal", |h, r, _, _| bag_heal(h, r)),
    ("ally", |h, r, _, _| bag_ally(h, r)),
    ("dead-input", |h, r, _, _| bag_dead_input(h, r)),
    ("kinds", |h, r, rom, base| bag_kinds(h, r, rom, base)),
    ("pickup", |h, r, rom, _| bag_pickup(h, r, rom)),
    ("shop", |h, r, rom, _| bag_shop(h, r, rom)),
    ("drop", |h, r, _, _| bag_drop(h, r)),
    ("save-continue", |h, r, rom, _| bag_save_continue(h, r, rom)),
    ("classic-rebuild", |h, r, rom, base| bag_classic_rebuild(h, r, rom, base)),
    ("bag-full", |h, r, rom, _| bag_full(h, r, rom)),
];

/// The items suite: each bag scenario from its crafted start, with its checks as a report.
fn items_suite(rom: &[u8], base: &[u8], only: &[String], out: &mut String) -> anyhow::Result<bool> {
    let mut all_ok = true;
    let mut ran = 0;
    if only.is_empty() || only.iter().any(|o| o == "round-trip") {
        ran += 1;
        let ok = match round_trip(rom, base, out) {
            Ok(ok) => ok,
            Err(error) => {
                writeln!(out, "FAIL round-trip: {error:#}")?;
                false
            }
        };
        all_ok &= ok;
    }
    if only.is_empty() || only.iter().any(|o| o == "pictures") {
        ran += 1;
        all_ok &= pictures_check(rom, out)?;
    }
    for &(name, scenario) in &BAG_SCENARIOS {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        ran += 1;
        let mut report = Report::default();
        let result = bag_scenario(rom, base, name, scenario, &mut report);
        if let Err(error) = &result {
            report.check(false, &format!("the scenario failed: {error:#}"));
        }
        let ok = report.passed();
        all_ok &= ok;
        writeln!(out, "{} {name}", if ok { "ok  " } else { "FAIL" })?;
        for (passed, what) in &report.lines {
            if !passed {
                writeln!(out, "       - {what}")?;
            }
        }
    }
    writeln!(
        out,
        "menu-test items: {ran} scenarios, {}",
        if all_ok { "all ok" } else { "FAILED" }
    )?;
    Ok(all_ok)
}

/// One bag scenario: its start is the base save with the items the scenario needs.
fn bag_scenario(
    rom: &[u8],
    base: &[u8],
    name: &str,
    scenario: BagScenario,
    report: &mut Report,
) -> anyhow::Result<()> {
    let ids: Vec<u8> = match name {
        "use-bulma" => vec![2, 2, 2, 2, 2],
        "use-second" => vec![1, 2],
        "ally" => vec![19],
        "heal" => vec![1, 1, 1],
        "radar" => vec![23],
        "dead-input" => vec![16, 16],
        "original-items" => vec![21, 22, 23],
        "pickup" | "save-continue" | "classic-rebuild" | "bag-full" => (0..21).collect(),
        "shop" => (0..20).collect(),
        "drop" => vec![2, 2, 16],
        _ => Vec::new(),
    };
    let hp = matches!(name, "use-bulma" | "use-second").then_some(30);
    let starts = bag_start(rom, base, name, &ids, hp)?;
    let sidecar = if name == "bag-full" { full_sidecar(rom, base)? } else { Sidecar::empty() };
    let mut h = bag_harness(rom, &starts, &sidecar)?;
    scenario(&mut h, report, rom, base)
}

/// A hash of the state the replay must reproduce: work RAM and the bag (FNV-1a, 64 bits).
fn state_hash(wram: &[u8], bag: &[u8]) -> u64 {
    wram.iter().chain(bag).fold(0xCBF2_9CE4_8422_2325_u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

/// The bag's bytes as the replay keeps them (an empty list when the bag is not bound).
fn bag_bytes(driver: &Driver) -> Vec<u8> {
    driver.bag().map_or_else(Vec::new, Bag::to_bytes)
}

/// Records an Enhanced session from power-on (the crafted save in the cartridge, its Continue, the
/// bag's Use of Bulma), then plays the replay file back: the final state hash must be the same. The
/// replay is format 2: its mode and the host events of each frame.
fn round_trip(rom: &[u8], base: &[u8], out: &mut String) -> anyhow::Result<bool> {
    let crafted = craft_save(rom, base, &[2, 2, 2, 2, 2], Some(30))?;
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    let labels = ssd_catalogue::menu_labels(&catalogue);
    let mut saves = MemorySaveStore::default();
    saves.0.insert("sav".to_owned(), crafted.clone());
    let machine = Machine::new(rom.to_vec(), Box::new(saves))?;
    let recorder = Recorder::start(&machine)?.in_mode(MODE_ENHANCED);
    let mut h = Harness {
        machine,
        driver: Driver::with_bag(labels.clone(), Rules::default(), Sidecar::empty()),
        tape: None,
        recorder: Some(recorder),
    };
    let continued =
        scripts::parse_timeline(&save::continue_prefix(0)).map_err(anyhow::Error::msg)?;
    for pad in continued {
        h.frame(pad)?;
    }
    h.frames(30)?;
    open_items(&mut h)?;
    h.key(Key::Confirm)?;
    h.key(Key::Confirm)?;
    h.key(Key::Confirm)?;
    h.key(Key::Right)?;
    h.key(Key::Confirm)?;
    h.until_idle()?;
    h.frames(SETTLE)?;
    let recorded = state_hash(h.machine.wram(), &bag_bytes(&h.driver));
    let replay = h.recorder.take().context("the recording")?.into_replay();
    let events: usize = replay.events.iter().map(Vec::len).sum();
    let bytes = replay.to_bytes(CORE_VERSION);
    let path = scratch_dir().join("round-trip.ssdreplay");
    std::fs::create_dir_all(scratch_dir())?;
    std::fs::write(&path, &bytes)?;

    // The file, read back, on a machine of its own, with the bag bound by the replay's events.
    let read = Replay::from_bytes(&std::fs::read(&path)?, ssd_rom::crc32(rom), CORE_VERSION)?;
    let mut played = read.power_on(rom.to_vec())?;
    let mut driver = Driver::with_bag(labels, Rules::default(), Sidecar::empty());
    driver.set_binding(Binding::Recorded);
    for frame in 0..read.frames() {
        let _ = driver.frame(played.wram(), played.sram(), Joypad::default());
        apply_events(&mut played, read.events_at(frame))?;
        for event in read.events_at(frame) {
            if let HostEvent::App { tag, payload } = event {
                driver.replay_event(*tag, payload, played.wram()).map_err(anyhow::Error::msg)?;
            }
        }
        played.run_frame(read.input(frame), &mut NullAudio)?;
    }
    let replayed = state_hash(played.wram(), &bag_bytes(&driver));
    let same = recorded == replayed && read.mode == MODE_ENHANCED && events > 0;
    writeln!(
        out,
        "{} round trip: format 2, mode {}, {} frames, {events} host events; the final state {}",
        if same { "ok  " } else { "FAIL" },
        read.mode,
        read.frames(),
        if same { "is the same" } else { "differs" },
    )?;
    Ok(same)
}

/// Bubbles, Escape and the Dragon Radar are used through the original's own effects, one unit each, from the
/// bag. Bubbles and Escape are consumed and the menu comes back to the field by itself. The Radar is a keepsake
/// (it cannot be dropped and its effect never removes its entry): its own screen stays up for the player, who
/// leaves it with B, and the Radar stays in the bag.
fn original_items(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    // The bag binds on the first frame of the start.
    h.frames(1)?;
    for name in ["Bubbles", "Escape", "D.Radar"] {
        let before = bag_summary(h).0.len();
        open_items(h)?;
        // Use or Drop, then Use: none of the three asks for a member, so the use is taken at once.
        h.key(Key::Confirm)?;
        h.key(Key::Confirm)?;
        h.until_idle()?;
        let kept = name == "D.Radar";
        if !kept {
            h.answer()?;
        }
        h.frames(SETTLE)?;
        if kept {
            r.check(
                h.driver.curtain() == Curtain::Lifted,
                "the radar's own screen is up for the player: the curtain lifts for it",
            );
            h.press(b())?;
            h.frames(SETTLE)?;
        }
        let after = bag_summary(h).0.len();
        r.check(
            (if kept { after == before } else { after + 1 == before })
                && h.byte(d::FIELD_STATE) == 0
                && matches!(h.driver.last_outcome(), Some(Outcome::Reached) | None),
            &format!(
                "{name} is used and {}: {:?} (field {} screen {} step {} busy {} cursor {} row0 {:02x}/{:02x} sheet {:?} bag {:?})",
                if kept { "stays in the bag" } else { "leaves the bag" },
                h.driver.last_outcome(), h.byte(d::FIELD_STATE), h.byte(d::MENU_SCREEN), h.byte(d::MENU_STEP),
                h.byte(d::MENU_BUSY), h.byte(d::MAP_ENTRY_VALUE), h.byte(d::ITEM_LIST), h.byte(d::ITEM_LIST + 1),
                h.driver.nav().sheet(), bag_summary(h).0
            ),
        );
    }
    Ok(())
}

// --- the battle's Items screen (suite `battle`): a battle of a replay, the bag bound to crafted stacks ----------------

/// Frames a use may take from the HD's Use to the plan's end. The original ignores a press that comes within three
/// frames of the one before, so a plan that presses back to back loses presses and waits 150 frames for each: a use from
/// a list of two stacks took 327 frames (a visible freeze on a phone), and one that keeps its gaps takes about 60.
const BATTLE_PLAN_LIMIT: u32 = 120;

/// The first frame of the replay `path` (within `limit` frames) where the battle's command window is up and idle. The
/// replay's own presses are not played after it: the window waits for the scenario's.
fn battle_frame(rom: &[u8], path: &Path, limit: u64) -> anyhow::Result<u64> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let replay = Replay::from_bytes(&bytes, ssd_rom::crc32(rom), CORE_VERSION)?;
    let mut machine = replay.power_on(rom.to_vec())?;
    let labels = Labels::default();
    for frame in 0..limit {
        machine.run_frame(replay.input(frame), &mut NullAudio)?;
        if ssd_game::menu::MenuProbe::new(machine.wram(), &labels).battle_command() {
            return Ok(frame + 1);
        }
    }
    bail!("no idle battle command window in the first {limit} frames of {}", path.display())
}

/// A battle scenario: the harness is in the battle with the bag bound to the scenario's objects (one unit each).
type BattleScenario = fn(&mut Harness, &mut Report) -> anyhow::Result<()>;

/// The battle scenarios by name, with the objects of the bag each starts with.
const BATTLE_SCENARIOS: [(&str, &[u8], BattleScenario); 5] = [
    ("self", &[6, 6, 6], battle_self),
    ("long-bag", &[2, 2, 2, 6, 6, 6, 18, 18], battle_long_bag),
    ("ally", &[2, 2], battle_ally),
    ("card", &[13, 13], battle_card),
    ("gohan", &[26], battle_gohan),
];

fn battle_suite(
    rom: &[u8],
    start: &Start,
    only: &[String],
    out: &mut String,
) -> anyhow::Result<bool> {
    let Start::Replay { path, frames } = start else {
        bail!(
            "menu-test battle takes --replay <a replay with a battle> --frames <the most frames to scan>"
        );
    };
    let at = battle_frame(rom, path, *frames)?;
    let starts = start_of(rom, &Start::Replay { path: path.clone(), frames: at })?;
    let catalogue = ssd_catalogue::Catalogue::from_rom(rom)?;
    let labels = ssd_catalogue::menu_labels(&catalogue);
    {
        let mut h = harness(rom, &starts)?;
        h.frames(1)?;
        writeln!(
            out,
            "the battle's command window is idle at frame {at}: group {}, party of {}",
            h.byte(d::ENCOUNTER_GROUP_INDEX),
            party_len(&h)
        )?;
    }
    let mut all_ok = true;
    let mut ran = 0;
    for &(name, objects, scenario) in &BATTLE_SCENARIOS {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        ran += 1;
        let mut report = Report::default();
        let result = battle_harness(&starts, &labels, objects)
            .and_then(|mut h| scenario(&mut h, &mut report));
        if let Err(error) = &result {
            report.check(false, &format!("the scenario failed: {error:#}"));
        }
        let ok = report.passed();
        all_ok &= ok;
        writeln!(out, "{} {name}", if ok { "ok  " } else { "FAIL" })?;
        for (passed, what) in &report.lines {
            if !passed {
                writeln!(out, "       - {what}")?;
            }
        }
    }
    writeln!(
        out,
        "menu-test battle: {ran} scenarios, {}",
        if all_ok { "all ok" } else { "FAILED" }
    )?;
    Ok(all_ok)
}

/// A harness in the battle of the start, with the bag bound (as a recorded bind) to one unit of each of `objects`.
fn battle_harness(starts: &Starts, labels: &Labels, objects: &[u8]) -> anyhow::Result<Harness> {
    let mut machine = (starts.fresh)()?;
    machine.load_state(&starts.state)?;
    enhanced_hooks(&mut machine);
    let mut driver = Driver::with_bag(labels.clone(), Rules::default(), Sidecar::empty())
        .with_cheats(ssd_menus::cheats::CheatSidecar::empty());
    let mut bag = Bag::new(Rules::default());
    for &object in objects {
        let story = labels.objects.get(usize::from(object)).is_some_and(|label| label.story);
        bag.add(object, 0, Class::of(object, story)).map_err(|_| anyhow::anyhow!("bag full"))?;
    }
    let events = driver
        .replay_event(ssd_game::menu::keeper::BIND_TAG, &bag.to_bytes(), machine.wram())
        .map_err(anyhow::Error::msg)?;
    apply_events(&mut machine, &events)?;
    let mut h = Harness::new(machine, driver);
    h.frames(20)?;
    Ok(h)
}

/// The player's Item on the battle's command window: Down to the row and A. The HD's Items screen goes over the list.
fn open_battle_items(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    r.check(h.driver.curtain() == Curtain::Down, "the battle's command window is the game's");
    while h.byte(d::MAP_ENTRY_VALUE) != 1 {
        h.press(Joypad { down: true, ..Joypad::default() })?;
        h.frames(10)?;
    }
    h.press(a())?;
    h.frames(60)?;
    r.check(
        h.driver.curtain() == Curtain::Up && h.driver.nav().screen() == ssd_menus::Screen::Items,
        &format!(
            "the HD's Items screen is over the battle's list (curtain {:?}, screen {})",
            h.driver.curtain(),
            h.byte(d::MENU_SCREEN)
        ),
    );
    Ok(())
}

/// Opens the HD's Items screen in the battle, moves to the stack `pick` and uses it: Use or Drop, then Use. Checks that
/// the plan reaches its state without a lost press's wait ([`BATTLE_PLAN_LIMIT`]); from there the original's screens are
/// the player's.
fn battle_use(h: &mut Harness, r: &mut Report, pick: usize) -> anyhow::Result<()> {
    open_battle_items(h, r)?;
    for _ in 0..pick {
        h.key(Key::Down)?;
    }
    h.key(Key::Confirm)?;
    r.check(h.driver.nav().sheet() == Sheet::Choose, "Use or Drop for the stack");
    h.driver.key(Key::Confirm, h.machine.wram());
    let mut frames = 0;
    loop {
        h.frame(Joypad::default())?;
        frames += 1;
        if !h.driver.busy() || frames > CHOICE_LIMIT {
            break;
        }
    }
    r.check(
        h.driver.last_outcome() == Some(Outcome::Reached),
        &format!("the plan reached its state: {:?}", h.driver.last_outcome()),
    );
    r.check(
        frames <= BATTLE_PLAN_LIMIT,
        &format!("the plan took {frames} frames (at most {BATTLE_PLAN_LIMIT}): a press was lost"),
    );
    Ok(())
}

/// The player's answers on the original's screens to the end of a use, and the checks of its end: the battle has its
/// command window back, and one unit of `object` is gone from the bag.
fn battle_finish(h: &mut Harness, r: &mut Report, object: u8, before: u32) -> anyhow::Result<()> {
    h.answer()?;
    h.frames(SETTLE)?;
    r.check(
        h.driver.curtain() == Curtain::Down && h.byte(d::MENU_SCREEN) == 17,
        &format!(
            "the battle has its command window back (curtain {:?}, screen {})",
            h.driver.curtain(),
            h.byte(d::MENU_SCREEN)
        ),
    );
    let after = h.driver.bag().map_or(0, |bag| bag.units_of(object));
    r.check(
        after + 1 == before,
        &format!("one unit of object {object} is used: {before} to {after}"),
    );
    Ok(())
}

fn units(h: &Harness, object: u8) -> u32 {
    h.driver.bag().map_or(0, |bag| bag.units_of(object))
}

/// An object that works on the user (Guru, BP +50%), from a stack of three.
fn battle_self(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    let before = units(h, 6);
    battle_use(h, r, 0)?;
    battle_finish(h, r, 6, before)
}

/// A bag of three stacks and the last one used: the plan presses Down and Up on a list of several rows (the presses the
/// original ignores when they come too soon after each other), then A and the use.
fn battle_long_bag(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    let before = units(h, 18);
    battle_use(h, r, 2)?;
    r.check(h.byte(d::MENU_SCREEN) == 21, "Yajirobe's effect waits on the battle's action screen");
    battle_finish(h, r, 18, before)
}

/// An object used on an ally: Bulma on the party's second member, whose HP is low, heals him and not the first.
fn battle_ally(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    anyhow::ensure!(party_len(h) >= 2, "the battle's party has one member");
    set_member_hp(h, 0, 150)?;
    set_member_hp(h, 1, 30)?;
    let before = units(h, 2);
    battle_use(h, r, 0)?;
    // The original asks on whom with its party window (the cursor on the first member): Down to the second, and A.
    h.frames(20)?;
    r.check(
        h.driver.curtain() == Curtain::Lifted && h.byte(d::MENU_SCREEN) == 2,
        "the original's own screen asks on whom",
    );
    h.press(Joypad { down: true, ..Joypad::default() })?;
    h.frames(20)?;
    h.press(a())?;
    h.frames(ANSWER_GAP)?;
    battle_finish(h, r, 2, before)?;
    let (first, second) = (member_hp(h, 0), member_hp(h, 1));
    r.check(second == 55, &format!("the second member heals 25: 30 to {second}"));
    r.check(first == 150, &format!("the first member is not healed: {first}"));
    Ok(())
}

/// A card object (Oolong changes the whole hand), from a stack of two: its use is taken and the battle goes on.
fn battle_card(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    let before = units(h, 13);
    battle_use(h, r, 0)?;
    battle_finish(h, r, 13, before)
}

/// The story's Gohan, who can be called from the list of the first Raditz battle only (encounter group 5).
fn battle_gohan(h: &mut Harness, r: &mut Report) -> anyhow::Result<()> {
    if h.byte(d::ENCOUNTER_GROUP_INDEX) != 5 {
        // Another battle: the HD refuses him there (the driver's unit tests say so), and there is nothing to use.
        r.check(true, "not the first Raditz battle: skipped");
        return Ok(());
    }
    let before = units(h, 26);
    battle_use(h, r, 0)?;
    battle_finish(h, r, 26, before)
}
