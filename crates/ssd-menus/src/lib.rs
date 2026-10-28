//! The HD menus (`Enhanced` mode, milestone M4): the layout and palette of the approved design, the
//! draw lists the screens build, and the view they show. Pure Rust: no emulator, no egui, no ROM.
//!
//! The data flows one way. `ssd-game`'s `MenuProbe` reads the menu state from the game's memory
//! and joins it with [`Labels`] into a [`MenuView`]; [`nav`] keeps the HD cursors and says what a
//! key chose; [`screens::scene`] turns the view into a [`DrawList`]; `ssd-app` paints the list with
//! egui. The menu logic stays the original's: this crate only draws what the game's state says and
//! moves the HD cursors.

pub mod bag;
pub mod cheats;
mod controls_page;
pub mod draw;
pub mod language;
pub mod layout;
pub mod mods_state;
pub mod nav;
pub mod options;
pub mod packs;
pub mod screens;
pub mod setup;
pub mod style;
pub mod ui_text;
pub mod view;

pub use draw::{Align, CardPart, Color, DrawList, Op, Rect};
pub use language::Language;
pub use nav::{Action, Entry, Key, Nav};
pub use style::Style;
pub use view::{
    CharacterLabel, CommandLabel, Labels, Member, MenuView, ObjectLabel, Picture, Screen, Selected,
    Slot, StatusLabels,
};
