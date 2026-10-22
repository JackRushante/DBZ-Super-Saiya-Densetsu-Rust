//! The message language of the packs (M10, Task 4): a pack's `[[messages]]` file is compiled to the strings and the
//! lists the message engine reads, in the original's own format, so no new string table is made.
//!
//! [`messages`] compiles a file: the tiles of each line ([`encode`] is the inverse of the English font's table, with
//! the tokens for the name and number slots and the portrait), the budgets of the windows, the terminators of the
//! pages and the lists. The format and the budgets are in `docs/mods/text.md`; the engine's in
//! `docs/re/data/text-messages.md`.

mod encode;
pub mod messages;

pub use messages::{
    Compiled, FIRST_DIALOGUE, FIRST_NARRATION, MAX_ID, MAX_INDEX, MessageError, Window, compile,
};
// The line encoder, which the tables of a pack (`crate::tables`) write their names with.
pub(crate) use encode::{LineError, NAME_CELLS, Piece, line};
