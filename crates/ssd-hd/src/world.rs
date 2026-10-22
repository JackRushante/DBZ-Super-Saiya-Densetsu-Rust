//! The field's world, for a wider view than the 256 pixels the game draws: the map lookup and
//! the map objects ported from the game, the view policy and the wide renderer built on them.
//!
//! Everything here is read-only: it takes the ROM, a copy of the few work RAM variables the
//! field uses ([`FieldState`]) and the frame's render log, and never touches the machine, so a
//! wide frame cannot change what the game does.

mod check;
mod map;
mod objects;
mod wide;

pub use check::{ObjectCheck, TileCheck, TileDifference, check_objects, check_tiles};
pub use map::{FieldMap, FieldState, Rom, is_field_mode};
pub use objects::{MapObject, Part, metasprite};
pub use wide::{
    HudPlacement, MAX_WIDTH, NotField, Rect, Scene, SharedHd, WIDTH_16_9, WideError, WideFrame,
    WideHd, WideOptions, WideRenderer, WideRun, WideStats, classify, next_serial, place_window,
};
