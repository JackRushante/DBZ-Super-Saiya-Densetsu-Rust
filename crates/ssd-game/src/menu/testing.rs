//! A stand-in for the original menu, for the unit tests of the presses and of the driver: the menu
//! state bytes change with the buttons as the original's do, in the few places the tests need.
//! The real game is what the ROM-gated tests and `xtask menu-test` play.

use super::presses::Button;
use crate::data as d;
use ssd_oracle::{HostEvent, Joypad};

/// The bus address of the first byte of work RAM.
const WRAM_BASE: u32 = 0x7E_0000;
/// Frames the menu takes to open, and a take-off takes.
const OPENING_FRAMES: u32 = 18;
const TAKE_OFF_FRAMES: u32 = 20;
/// Frames the Item list takes to be drawn.
const ITEMS_FRAMES: u32 = 2;

/// The original menu on the field, in the states the tests use.
#[derive(Default)]
pub(crate) struct FakeMenu {
    /// `field_state`: 0 free, `$0F` opening, 1 open.
    pub field: u8,
    /// `menu_screen`.
    pub screen: u8,
    /// `map_entry_value`.
    pub cursor: u8,
    /// `menu_busy`.
    pub busy: bool,
    /// `text_speed` (`$01D0`): the Text screen opens its cursor on it and A stores the cursor in it.
    pub text_speed: u8,
    /// `menu_step`.
    pub step: u8,
    /// The slots the original's Save wrote, in order (its A on the slot list).
    pub saved: Vec<u8>,
    /// The party's characters, in the order of the original's records (empty: the menu holds no party).
    pub party: Vec<u8>,
    /// The party list the game keeps at `party_order` (`$0190`): it is written when the game saves, so an order applied
    /// leaves it as it was.
    pub listed: Vec<u8>,
    /// The rows of the party the original's Order screen has taken so far, slot by slot.
    pub picks: Vec<u8>,
    /// `travel_mode`: 2 flying.
    pub flying: bool,
    /// `effect_pending` bit 7: an item effect runs.
    pub effect: bool,
    /// While the menu opens, `menu_screen` still holds a screen of the menu before (2).
    pub stale: bool,
    /// A message waits for A (after Talk or Look).
    pub message: bool,
    /// The Talk or Look message holds the field (`field_state` 5), as a message with a person of the map does.
    pub hold: bool,
    /// A battle runs (`context_61` 1): its command window is screen 17, and it holds the field (`field_state` 2).
    pub battle: bool,
    /// `encounter_group_index`: the encounter group of the battle.
    pub group: u8,
    /// The original ignores a press that comes within three frames of the one before, as the battle's Item list does.
    pub strict: bool,
    /// The item list the game holds (object and state word of each entry), as the keeper last wrote it.
    pub list: Vec<(u8, u16)>,
    /// Frames until the original takes a press again (see [`FakeMenu::strict`]).
    quiet: u32,
    /// Frames until the timed change of the state (opening, take-off, the Item list).
    timer: u32,
    /// The joypad of the last frame, for the presses.
    last: Joypad,
}

impl FakeMenu {
    /// A menu on the field that holds the party `party` (character ids, in the order of the original's records).
    pub(crate) fn with_party(party: &[u8]) -> Self {
        Self { party: party.to_vec(), listed: party.to_vec(), ..Self::default() }
    }

    /// The rows of the command window now: five on foot, three flying.
    fn rows(&self) -> u8 {
        if self.flying { 3 } else { 5 }
    }

    /// The row of the command window that `entry` is on, as the game has them.
    fn row_of(&self, entry: &str) -> Option<u8> {
        let rows: &[&str] = if self.flying {
            &["Land", "Item", "Menu"]
        } else {
            &["Talk", "Look", "Fly", "Item", "Menu"]
        };
        rows.iter().position(|&e| e == entry).map(|row| row as u8)
    }

    /// Work RAM with the menu's bytes in place.
    pub(crate) fn wram(&self) -> Vec<u8> {
        let mut ram = vec![0_u8; 0x2_0000];
        let mut put = |address: u32, value: u8| {
            ram[(address - WRAM_BASE) as usize] = value;
        };
        put(d::FIELD_STATE, self.field);
        put(d::MENU_SCREEN, if self.stale && self.field == 0x0F { 2 } else { self.screen });
        put(d::MENU_BUSY, u8::from(self.busy));
        put(d::MENU_STEP, self.step);
        // The window queue is empty once the timed draws are over.
        put(d::MENU_DRAW_DONE, if self.timer == 0 { 0x80 } else { 0 });
        put(d::MAP_ENTRY_VALUE, self.cursor);
        put(d::TEXT_SPEED, self.text_speed);
        if !self.party.is_empty() {
            put(0x7E_01BB, self.party.len() as u8);
            put(0x7E_0D6B, self.picks.len() as u8);
            for (row, &character) in self.party.iter().enumerate() {
                let record = d::PARTY_RECORDS + 32 * row as u32;
                // A row the Order screen has taken loses bit 7 of its flags.
                put(record, if self.picks.contains(&(row as u8)) { 0x40 } else { 0xC0 });
                put(record + 0x1C, character);
            }
            for (index, &character) in self.listed.iter().enumerate() {
                put(d::PARTY_ORDER + index as u32, character);
            }
            put(d::PARTY_ORDER + self.listed.len() as u32, 0xFF);
        }
        put(d::TRAVEL_MODE, if self.flying { 2 } else { 1 });
        put(d::EFFECT_PENDING, if self.effect { 0x80 } else { 0 });
        put(d::ITEM_COUNT, if self.list.is_empty() { 3 } else { self.list.len() as u8 });
        if self.battle {
            put(d::CONTEXT_61, 1);
        }
        put(d::ENCOUNTER_GROUP_INDEX, self.group);
        for (index, &(object, extra)) in self.list.iter().enumerate() {
            put(d::ITEM_LIST + 2 * index as u32, object);
            let [low, high] = extra.to_le_bytes();
            put(d::ITEM_EXTRA + 2 * index as u32, low);
            put(d::ITEM_EXTRA + 2 * index as u32 + 1, high);
        }
        ram
    }

    /// Takes the item list the host writes of a frame (the keeper's list: entries, state words and count).
    pub(crate) fn absorb(&mut self, events: &[HostEvent]) {
        let (mut entries, mut extras, mut count) = (None, None, None);
        for event in events {
            if let HostEvent::Wram { address, bytes } = event {
                if *address == d::ITEM_LIST {
                    entries = Some(bytes.clone());
                } else if *address == d::ITEM_EXTRA {
                    extras = Some(bytes.clone());
                } else if *address == d::ITEM_COUNT {
                    count = bytes.first().copied();
                }
            }
        }
        if let (Some(entries), Some(extras), Some(count)) = (entries, extras, count) {
            self.list = (0..usize::from(count))
                .map(|index| {
                    (
                        entries[2 * index],
                        u16::from_le_bytes([extras[2 * index], extras[2 * index + 1]]),
                    )
                })
                .collect();
        }
    }

    /// One frame with `pad` held: the presses of the frame, then the frame's timed changes.
    pub(crate) fn tick(&mut self, pad: Joypad) {
        let (mut down, mut up) = (pad.down && !self.last.down, pad.up && !self.last.up);
        let (mut a, mut b) = (pad.a && !self.last.a, pad.b && !self.last.b);
        let (mut left, mut right) = (pad.left && !self.last.left, pad.right && !self.last.right);
        self.last = pad;
        if self.strict {
            if self.quiet > 0 {
                self.quiet -= 1;
                (down, up, a, b, left, right) = (false, false, false, false, false, false);
            } else if down || up || a || b || left || right {
                self.quiet = 3;
            }
        }
        if self.timer > 0 {
            self.timer -= 1;
            if self.timer == 0 {
                self.timed_change();
            }
            return;
        }
        match self.field {
            0 => {
                if a {
                    self.field = 0x0F;
                    self.screen = 0;
                    self.cursor = 0;
                    self.busy = true;
                    self.timer = OPENING_FRAMES;
                }
            }
            1 if self.screen == 9 => self.text_screen(left, right, a),
            1 if self.screen == 8 && !self.party.is_empty() => {
                self.order_screen(Edges { down, up, a, b });
            }
            1 | 5 => self.menu(down, a, b),
            2 if self.battle => self.battle_menu(Edges { down, up, a, b }),
            _ => {}
        }
    }

    /// The presses on a battle: its command window (screen 17: Atk., Item, Run, Menu), and its Item list (screen 2),
    /// where A takes the row under the cursor to its Use or Drop choice (step 2) and B goes back to the command window.
    fn battle_menu(&mut self, edges: Edges) {
        let Edges { down, up, a, b } = edges;
        match (self.screen, self.step) {
            (17, _) => {
                if down {
                    self.cursor = (self.cursor + 1) % 4;
                }
                if a && self.cursor == 1 {
                    self.screen = 2;
                    self.step = 0;
                    self.busy = true;
                    self.timer = ITEMS_FRAMES;
                    self.cursor = 0;
                }
            }
            (2, 1) => {
                let rows = self.list.len().max(1) as u8;
                if down {
                    self.cursor = (self.cursor + 1) % rows;
                }
                if up {
                    self.cursor = (self.cursor + rows - 1) % rows;
                }
                if b {
                    self.screen = 17;
                    self.step = 0;
                    self.cursor = 1;
                    self.busy = false;
                } else if a {
                    self.step = 2;
                    self.cursor = 0;
                }
            }
            (2, 2) if b => self.step = 1,
            // The Use of the row on top: its effect runs on the battle's action screen, and the entry is gone.
            (2, 2) if a && self.cursor == 0 => {
                self.screen = 21;
                self.busy = false;
                if !self.list.is_empty() {
                    self.list.remove(0);
                }
            }
            // One A on the action screen ends its message: the battle's command window is back.
            (21, _) if a => {
                self.screen = 17;
                self.step = 0;
                self.cursor = 1;
                self.busy = false;
            }
            _ => {}
        }
    }

    /// The original's Text screen: Left and Right move the cursor along the five speeds (no wrapping), and A stores the
    /// cursor in `text_speed` and goes on to the field, closing the menu.
    fn text_screen(&mut self, left: bool, right: bool, a: bool) {
        if left {
            self.cursor = self.cursor.saturating_sub(1);
        }
        if right {
            self.cursor = (self.cursor + 1).min(4);
        }
        if a {
            self.text_speed = self.cursor;
            self.field = 0;
            self.screen = 0;
            self.busy = false;
        }
    }

    /// The original's Save: the question (step 0: A is Yes), the slot list (step 1: Down moves the cursor over the three
    /// slots, A writes the slot and shows "Completed!", step 2), and the A that returns to the field.
    fn save_screen(&mut self, down: bool, a: bool, b: bool) {
        match self.step {
            0 if a => {
                self.step = 1;
                self.cursor = 0;
            }
            1 if down => self.cursor = (self.cursor + 1) % 3,
            1 if a => {
                self.saved.push(self.cursor);
                self.step = 2;
                self.cursor = 0;
            }
            2 if a => {
                self.field = 0;
                self.screen = 0;
                self.busy = false;
            }
            _ if b && self.step < 2 => {
                self.screen = 4;
                self.cursor = 0;
            }
            _ => {}
        }
    }

    /// The original's Order screen, as the disassembly and the real game have it: step 1 takes the picks (the cursor skips
    /// the rows taken and moves to the next row left after a pick; the last row is taken by itself, and the Yes or No is
    /// asked), step 2 asks (Yes writes the order back and closes the menu, No empties the slots).
    fn order_screen(&mut self, edges: Edges) {
        let Edges { down, up, a, b } = edges;
        let size = self.party.len() as u8;
        let next = |from: u8, picks: &[u8], step: i8| {
            let mut row = from;
            for _ in 0..size {
                row = (i16::from(row) + i16::from(step)).rem_euclid(i16::from(size)) as u8;
                if !picks.contains(&row) {
                    return row;
                }
            }
            from
        };
        if self.step == 1 {
            if down {
                self.cursor = next(self.cursor, &self.picks, 1);
            }
            if up {
                self.cursor = next(self.cursor, &self.picks, -1);
            }
            if a {
                self.picks.push(self.cursor);
                if self.picks.len() + 1 >= self.party.len() {
                    // The last row is taken by itself, and the order is asked for.
                    if let Some(last) = (0..size).find(|row| !self.picks.contains(row)) {
                        self.picks.push(last);
                    }
                    self.step = 2;
                    self.cursor = 0;
                } else {
                    self.cursor = next(self.cursor, &self.picks, 1);
                }
            } else if b && self.picks.pop().is_none() {
                self.screen = 4;
                self.cursor = 2;
            }
        } else if self.step == 2 {
            if down || up {
                self.cursor = (self.cursor + 1) % 2;
            }
            if a && self.cursor == 0 {
                self.party = self.picks.iter().map(|&row| self.party[usize::from(row)]).collect();
                self.picks.clear();
                self.screen = 0;
                self.step = 0;
                self.field = 0;
                self.busy = false;
            } else if a {
                self.picks.clear();
                self.step = 1;
                self.cursor = 0;
            } else if b {
                self.picks.truncate(self.picks.len().saturating_sub(2));
                self.step = 1;
            }
        }
    }

    /// What the frames after a timer do.
    fn timed_change(&mut self) {
        match (self.field, self.screen) {
            (0x0F, _) => {
                self.field = 1;
                self.busy = false;
            }
            (1, 1 | 5) => {
                // The take-off or the landing closes the menu, and the party's mode follows.
                self.flying = self.screen == 1;
                self.field = 0;
                self.screen = 0;
                self.busy = false;
            }
            (1, 2) => {
                self.step = 1;
            }
            (2, 2) => {
                self.step = 1;
                self.busy = false;
            }
            _ => {}
        }
    }

    /// The presses on the menu screens.
    fn menu(&mut self, down: bool, a: bool, b: bool) {
        if self.message {
            if a {
                self.message = false;
                self.busy = false;
                self.field = 0;
            }
            return;
        }
        match self.screen {
            0 => {
                if down {
                    self.cursor = (self.cursor + 1) % self.rows();
                }
                if b {
                    self.field = 0;
                    self.busy = false;
                }
                if a {
                    self.command_choice();
                }
            }
            2 => {
                if b {
                    self.screen = 0;
                    self.cursor = self.row_of("Item").unwrap_or(0);
                    self.busy = false;
                }
            }
            4 => {
                if down {
                    self.cursor = (self.cursor + 1) % 5;
                }
                if b {
                    self.screen = 0;
                    self.cursor = self.row_of("Menu").unwrap_or(0);
                    self.busy = false;
                } else if a {
                    self.screen = 6 + self.cursor;
                    self.step = 0;
                    self.cursor = if self.screen == 9 { self.text_speed } else { 0 };
                    if self.screen == 8 {
                        self.step = 1;
                        self.picks.clear();
                    }
                    // The original's Status page 1 idles on step 2 with its busy flag set.
                    if self.screen == 7 {
                        self.step = 2;
                        self.busy = true;
                    }
                }
            }
            // The Cards screen takes only A, which closes the whole menu
            6 if a => {
                self.screen = 0;
                self.field = 0;
                self.busy = false;
            }
            10 => self.save_screen(down, a, b),
            6..=10 if b => {
                self.screen = 4;
                self.cursor = 0;
                self.step = 0;
                self.busy = false;
            }
            _ => {}
        }
    }

    /// A press of A on the command window: the entry under the cursor happens.
    fn command_choice(&mut self) {
        let row = self.cursor;
        if self.row_of("Talk") == Some(row) || self.row_of("Look") == Some(row) {
            self.message = true;
            self.busy = true;
            if self.hold {
                self.field = 5;
            }
        } else if self.row_of("Fly") == Some(row) {
            self.screen = 1;
            self.busy = true;
            self.timer = TAKE_OFF_FRAMES;
        } else if self.row_of("Land") == Some(row) {
            self.screen = 5;
            self.busy = true;
            self.timer = TAKE_OFF_FRAMES;
        } else if self.row_of("Item") == Some(row) {
            self.screen = 2;
            self.step = 0;
            self.busy = true;
            self.timer = ITEMS_FRAMES;
            self.cursor = 0;
        } else if self.row_of("Menu") == Some(row) {
            self.screen = 4;
            self.busy = false;
            self.cursor = 0;
        }
    }
}

/// The buttons pressed on a frame (down this frame, up the frame before).
#[derive(Debug, Clone, Copy)]
struct Edges {
    down: bool,
    up: bool,
    a: bool,
    b: bool,
}

/// The joypad of `button` held, for the tests.
pub(crate) fn held(button: Button) -> Joypad {
    let mut pad = Joypad::default();
    match button {
        Button::Up => pad.up = true,
        Button::Down => pad.down = true,
        Button::Left => pad.left = true,
        Button::Right => pad.right = true,
        Button::A => pad.a = true,
        Button::B => pad.b = true,
    }
    pad
}
