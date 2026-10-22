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
    /// `menu_step`.
    pub step: u8,
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
    /// The item list the game holds (object and state word of each entry), as the keeper last wrote it.
    pub list: Vec<(u8, u16)>,
    /// Frames until the timed change of the state (opening, take-off, the Item list).
    timer: u32,
    /// The joypad of the last frame, for the presses.
    last: Joypad,
}

impl FakeMenu {
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
        put(d::TRAVEL_MODE, if self.flying { 2 } else { 1 });
        put(d::EFFECT_PENDING, if self.effect { 0x80 } else { 0 });
        put(d::ITEM_COUNT, if self.list.is_empty() { 3 } else { self.list.len() as u8 });
        if self.battle {
            put(d::CONTEXT_61, 1);
        }
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
        let down = pad.down && !self.last.down;
        let a = pad.a && !self.last.a;
        let b = pad.b && !self.last.b;
        self.last = pad;
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
            1 | 5 => self.menu(down, a, b),
            2 if self.battle => self.battle_menu(down, a, b),
            _ => {}
        }
    }

    /// The presses on a battle: its command window (screen 17: Atk., Item, Run, Menu), and its Item list (screen 2),
    /// where A takes the row under the cursor to its Use or Drop choice (step 2) and B goes back to the command window.
    fn battle_menu(&mut self, down: bool, a: bool, b: bool) {
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
                if down {
                    self.cursor = (self.cursor + 1) % self.list.len().max(1) as u8;
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
                    self.cursor = 0;
                }
            }
            6..=10 if b => {
                self.screen = 4;
                self.cursor = 0;
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

/// The joypad of `button` held, for the tests.
pub(crate) fn held(button: Button) -> Joypad {
    let mut pad = Joypad::default();
    match button {
        Button::Down => pad.down = true,
        Button::A => pad.a = true,
        Button::B => pad.b = true,
    }
    pad
}
