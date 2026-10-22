//! The game reimplemented in Rust (spec §5.1 `ssd-game`): routines that run in place of the
//! original 65816 code through `ssd-oracle`'s hooks, and the hook table that installs them.
//!
//! Routines are named in `re/symbols.toml`, which holds their addresses and hook settings;
//! [`registry`] maps those names to the Rust functions. The same file names the data they use,
//! and [`data`] has its addresses as constants generated from it. [`scripts`] holds the scripted
//! play-throughs of the real-ROM tests and the reference replays.
//!
//! [`mode`] is the rules a run plays with (Classic or Enhanced). [`menu`] serves the HD menus of
//! Enhanced mode: it reads the state of the original menu ([`menu::MenuProbe`]), plays the original's
//! own input for each choice of the HD ([`menu::presses`]), and holds the curtain and the HD cursors
//! ([`menu::driver::Driver`]).

pub mod cheats;
pub mod data;
pub mod gameplay;
pub mod menu;
pub mod mode;
pub mod mods;
mod registry;
mod routines;
pub mod scripts;

pub use menu::MenuProbe;
pub use mode::{Mode, Ruleset};
#[cfg(feature = "recomp")]
pub use registry::recompiled_table;
pub use registry::{GameError, RegistryEntry, hook_table, missing_cycles_warning, registry};
