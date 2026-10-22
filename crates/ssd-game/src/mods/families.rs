//! The families the redirectors serve (`docs/re/data/tables.md`, rows 1 and 22): the scripts and the faces first,
//! then the messages and the strings (`docs/re/data/text-messages.md`).
//!
//! **Scripts** (row 1, note N1): the scene starter `03:ACB2` loads the script word of the scene from the
//! table at `06:D109` and stores it at `$A5` with `STA $A5` at `03:ACCA` (A is 16 bits there, and the
//! bank byte is already at `$A7`, written at `03:ACB6`). The redirector sends the scene's pointer to the
//! pack's data: A is the low word of the bus address and `$A7` its bank byte. The direct page is taken
//! to be 0 there: the routine's operands are direct-page ones, the listing of `xtask disasm` assumes
//! `D = 0`, and the execution traces of the reference replays have `D = 0000` at every instruction they
//! run (none of them runs this routine). `$00:00A7` is the work-RAM byte `$7E:00A7`.
//!
//! **Faces** (row 22, note N9): the portrait routine `01:C9B0` loads the 3-byte pointer of face `X / 3`
//! from `02:A3B4` into `$0D77` (the word) and `$0D79` (the bank). Its last store of the face path is at
//! `01:CA08`; `01:CA0B` (`REP #$20`, the next instruction) is the first point after it, and X still holds
//! `3 x` the face index there (the `TAX` that changes it is at `01:CA12`). In the execution traces of the
//! reference replays `01:CA0B` runs with `M = 8`, `X = 16` and `D = 0000`, `DBR = 00`.
//!
//! **Messages** and **strings** (`docs/re/data/text-messages.md`, section 1): the message engine reads a message's
//! list and a string through the 24-bit pointer at `$00-$02`, which the original stores at `01:D3C0` (the list) and at
//! `01:CF3A` (the string). [`message_wrapper`] stores a pack list's address there at `01:D3D5`, the instruction after
//! the list's store, and [`string_wrapper`] a pack string's address at `01:CF63`, the instruction after the string's
//! store, for a string of table 2 or 3. Both keys read M = 16-bit, X = 16-bit and `D = 0` in every trace run of `.work/re`.

use super::redirect::overlay;
use crate::data as d;
use ssd_mod::Family;
use ssd_oracle::Ctx;

/// `03:ACCA`, the `STA $A5` of the scene starter (A 16-bit).
pub(super) const SCRIPT_KEY: u32 = 0x03_ACCA;
/// `$7E:00A7`, the bank byte of the script pointer (`$A7` with `D = 0`).
const SCRIPT_BANK: u32 = 0x7E_00A7;
/// `01:CA0B`, the first instruction after the face pointer's three stores (8-bit A, see the module
/// documentation).
pub(super) const FACE_KEY: u32 = 0x01_CA0B;
/// `$7E:0D77` (the pointer's low word; `$0D78` its high byte) and `$7E:0D79` (its bank byte).
const FACE_POINTER: u32 = 0x7E_0D77;
const FACE_BANK: u32 = 0x7E_0D79;
/// The length of a face's row in `face_portrait_pointers`: X is the face index times this.
const FACE_ROW: u16 = 3;

/// `01:D3D5`, the `LDA $072A` that follows the store of a message's list pointer (`01:D3C0`: `$00-$01` the
/// pointer, `$02` its bank, A 16-bit from `REP #$20` at `01:D3C5`). The traces of `.work/re` (eight runs,
/// every row) read M = 0 (16-bit), X = 0 (16-bit), `D = 0000`, `DBR = 00` at this address.
pub(super) const MESSAGE_KEY: u32 = 0x01_D3D5;
/// `01:CF63`, the `SEP #$20` that follows the store of a string's pointer (`01:CF3A`: `$00-$01` the pointer, `$02`
/// its bank, A 16-bit from `REP #$20` at `01:CF58`). The traces read M = 0, X = 0, `D = 0000`, `DBR = 00` here.
pub(super) const STRING_KEY: u32 = 0x01_CF63;
/// `$7E:0000`: the 24-bit pointer the engine reads a message's list and a string through, with `D = 0` (the low
/// word at `$00`, the bank at `$02`).
const POINTER: u32 = 0x7E_0000;

/// `message_lists` (`$7E:0738-$073A`): the 24-bit base of the message lists the engine reads.
const MESSAGE_LISTS: u32 = 0x7E_0738;
/// The field's message lists (`07:8000`), the table the pack's message numbers extend.
const FIELD_MESSAGE_LISTS: u32 = 0x07_8000;

/// The message engine's list pointer ([`MESSAGE_KEY`]): a message with a pack list reads its words from the pack's
/// data, the list the original read is not used. The message is the one of `$072C` (`message_number`).
pub(super) fn message_wrapper(ctx: &mut Ctx<'_>) {
    let number = ctx.peek16(d::MESSAGE_NUMBER);
    // Battles read their own lists (`07:D79E`) with the same numbers: only the field's lists are the pack's.
    let lists =
        u32::from(ctx.peek16(MESSAGE_LISTS)) | u32::from(ctx.peek8(MESSAGE_LISTS + 2)) << 16;
    if lists != FIELD_MESSAGE_LISTS {
        ctx.resume_at(MESSAGE_KEY);
        return;
    }
    if let Some(address) = overlay(ctx, Family::Message, number) {
        store_pointer(ctx, address);
    }
    ctx.resume_at(MESSAGE_KEY);
}

/// The string address routine's store of the string pointer ([`STRING_KEY`]): a string of table 2 or 3 with a
/// pack string (the table is `message_table`, `$0733`, the index `message_string`, `$0723`) reads its tiles from
/// the pack's data. Every other string is left to the original.
pub(super) fn string_wrapper(ctx: &mut Ctx<'_>) {
    let table = ctx.peek8(d::MESSAGE_TABLE);
    let string = ctx.peek16(d::MESSAGE_STRING);
    if let Some(address) = string_family(table).and_then(|family| overlay(ctx, family, string)) {
        store_pointer(ctx, address);
    }
    ctx.resume_at(STRING_KEY);
}

/// The family of the strings of table `table` that the packs fill: 2 (dialogue) and 3 (narration).
fn string_family(table: u8) -> Option<Family> {
    match table {
        2 => Some(Family::String2),
        3 => Some(Family::String3),
        _ => None,
    }
}

/// The three bytes of a 24-bit pointer, the low byte first.
fn pointer_bytes(address: u32) -> [u8; 3] {
    [address as u8, (address >> 8) as u8, (address >> 16) as u8]
}

/// Stores the pointer `address` where the engine reads its list or its string through ([`POINTER`]).
fn store_pointer(ctx: &mut Ctx<'_>, address: u32) {
    for (k, byte) in pointer_bytes(address).into_iter().enumerate() {
        ctx.write8(POINTER + k as u32, byte);
    }
}

/// The scene starter's store of the script pointer ([`SCRIPT_KEY`]): a scene with a pack script gets the
/// script's bus address in A and its bank in `$A7`; any other scene is left to the original.
pub(super) fn script_wrapper(ctx: &mut Ctx<'_>) {
    let scene = ctx.read8(d::SCENE_ID);
    if let Some(address) = overlay(ctx, Family::Script, u16::from(scene)) {
        ctx.set_a((address & 0xFFFF) as u16);
        ctx.write8(SCRIPT_BANK, (address >> 16) as u8);
    }
    ctx.resume_at(SCRIPT_KEY);
}

/// The face path's store of the portrait pointer ([`FACE_KEY`]): a face with a pack picture gets the
/// picture's bus address in the three bytes the routine has just stored.
pub(super) fn face_wrapper(ctx: &mut Ctx<'_>) {
    let index = ctx.x() / FACE_ROW;
    if let Some(address) = overlay(ctx, Family::Face, index) {
        ctx.write8(FACE_POINTER, (address & 0xFF) as u8);
        ctx.write8(FACE_POINTER + 1, ((address >> 8) & 0xFF) as u8);
        ctx.write8(FACE_BANK, (address >> 16) as u8);
    }
    ctx.resume_at(FACE_KEY);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_is_low_byte_first_and_the_bank_last() {
        assert_eq!(pointer_bytes(0x20_9000), [0x00, 0x90, 0x20]);
        assert_eq!(pointer_bytes(0x7E_0000), [0x00, 0x00, 0x7E]);
    }

    #[test]
    fn the_strings_of_tables_2_and_3_are_the_pack_families_and_the_others_are_not() {
        assert_eq!(string_family(2), Some(Family::String2));
        assert_eq!(string_family(3), Some(Family::String3));
        for table in [0, 1, 4, 5, 6, 7, 8, 0xFF] {
            assert_eq!(string_family(table), None, "table {table}");
        }
    }

    #[test]
    fn the_keys_are_the_reads_of_the_list_and_the_string_pointers() {
        assert_eq!(MESSAGE_KEY, 0x01_D3D5);
        assert_eq!(STRING_KEY, 0x01_CF63);
        assert_eq!(POINTER, 0x7E_0000, "the direct page is 0 at both keys");
    }
}
