//! The talk to a map object on a pack map (M10, Task 6, `docs/mods/triggers.md`). The original's talk routine (`03:9AF6`)
//! reads the talk key of the object in its slot (`$1181,X`: the object's type plus `$30`) and walks the talk program of that
//! key, a word per key in the table at `03:9DF1`: it tests and sets story flags, and ends with a message (`00:89AC`). A pack
//! map's objects carry a message of their own (`ssd_mod::map`, the objects of the pack), which the table does not know, so on
//! a pack map the object speaks the message of its record instead.
//!
//! The hook is a Patch at `03:9AFC`, the `LDA $1181,X` that follows the routine's call to `03:9C22` (the object's own action,
//! which runs as before) and its `PLX`, so `X` is the slot's offset: seven times the slot's number. On a pack map, when the
//! slot holds the record of the same number (its key is that record's object plus `$30`, as the loader writes it), the
//! routine writes what `00:89AC` writes for the message (`$072C` the message, `$071B` `$80`, `$072A` 0), leaves the width at
//! 8 bits as the original's message exit does (its `SEP #$20`), and returns at `03:9B7B`, the RTL after the original's
//! `JSL 00:89AC`: the table walk is skipped. Any other slot, and any original map, declines.

use super::maps::{self, OBJECT_SLOTS};
use super::redirect::overlay;
use super::scene_ext::{Bus, Cpu};
use crate::data as d;
use ssd_mod::Family;
use ssd_mod::map::MAX_OBJECTS;
use ssd_oracle::Ctx;

/// `03:9AFC`: the `LDA $1181,X` of the talk routine (see the module documentation).
pub(super) const TALK_KEY: u32 = 0x03_9AFC;
/// `03:9B7B`: the RTL after the message start of the talk routine, where a pack talk returns.
const TALK_RETURN: u32 = 0x03_9B7B;
/// A slot of the object table is seven bytes, the talk key is its second byte.
const SLOT_LEN: u16 = 7;
/// A record of the pack's objects: the object's type (a byte), x and y (words), the message (a word): seven bytes after the
/// count, and the message at its fifth byte.
const RECORD_LEN: u32 = 7;
const RECORD_MESSAGE: u32 = 5;
/// The talk key of an object is its type plus this.
const KEY_BASE: u8 = 0x30;
/// The state `00:89AC` gives a message (`$071B`): a message is up.
const MESSAGE_UP: u8 = 0x80;

/// The little-endian word at `address`.
fn word(bus: &impl Bus, address: u32) -> u16 {
    u16::from_le_bytes([bus.read(address), bus.read(address + 1)])
}

/// The message of the pack object in the slot at `slot_offset`: the record of the same number in the objects of the pack at
/// `list`, when the slot's talk key is that record's object plus `$30`. `None` for any other slot.
fn talk_message(bus: &impl Bus, list: u32, slot_offset: u16) -> Option<u16> {
    if !slot_offset.is_multiple_of(SLOT_LEN) {
        return None;
    }
    let index = u32::from(slot_offset / SLOT_LEN);
    if index >= u32::from(bus.read(list)).min(MAX_OBJECTS as u32) {
        return None;
    }
    let record = list + 1 + RECORD_LEN * index;
    let key = bus.read(OBJECT_SLOTS + u32::from(slot_offset) + 1);
    (key == bus.read(record).wrapping_add(KEY_BASE)).then(|| word(bus, record + RECORD_MESSAGE))
}

/// `03:9AFC`: the talk to a map object (see the module documentation).
pub(super) fn talk_wrapper(ctx: &mut Ctx<'_>) {
    let map = ctx.peek8(d::MAP_ID);
    let list =
        maps::is_pack(map).then(|| overlay(ctx, Family::MapObjects, maps::slot(map))).flatten();
    let slot = ctx.x();
    let message = match list {
        Some(list) => talk_message(&Cpu(&mut *ctx), list, slot),
        None => None,
    };
    let Some(message) = message else {
        ctx.resume_at(TALK_KEY);
        return;
    };
    ctx.write16(d::MESSAGE_NUMBER, message);
    ctx.write8(d::MESSAGE_STATE, MESSAGE_UP);
    ctx.write8(d::MESSAGE_WORD, 0);
    ctx.set_m8(true);
    ctx.resume_at(TALK_RETURN);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A bus of sparse bytes: an unwritten address reads zero.
    #[derive(Default)]
    struct Memory(BTreeMap<u32, u8>);

    impl Bus for Memory {
        fn read(&self, address: u32) -> u8 {
            self.0.get(&address).copied().unwrap_or(0)
        }

        fn write(&mut self, address: u32, value: u8) {
            self.0.insert(address, value);
        }
    }

    const LIST: u32 = 0x20_9000;

    /// Two records (object 5 with message 402, object 9 with message 403) in the objects at `LIST`, and the slots of
    /// `$1180` holding them: slot 0 with key `$35`, slot 1 (offset 7) with key `$39`.
    fn memory() -> Memory {
        let mut memory = Memory::default();
        let mut bytes = vec![2];
        bytes.extend_from_slice(&[5, 10, 0, 20, 0, 0x92, 0x01]);
        bytes.extend_from_slice(&[9, 30, 0, 40, 0, 0x93, 0x01]);
        for (k, byte) in bytes.into_iter().enumerate() {
            memory.write(LIST + k as u32, byte);
        }
        memory.write(OBJECT_SLOTS + 1, 0x35);
        memory.write(OBJECT_SLOTS + 7 + 1, 0x39);
        memory
    }

    #[test]
    fn a_slot_speaks_the_message_of_the_record_of_its_number() {
        let memory = memory();
        assert_eq!(talk_message(&memory, LIST, 0), Some(402));
        assert_eq!(talk_message(&memory, LIST, 7), Some(403));
    }

    #[test]
    fn a_slot_with_another_key_or_no_record_declines() {
        let mut memory = memory();
        memory.write(OBJECT_SLOTS + 7 + 1, 0x35);
        assert_eq!(talk_message(&memory, LIST, 7), None, "the key is not the record's object");
        assert_eq!(talk_message(&memory, LIST, 14), None, "no record of number 2");
        assert_eq!(talk_message(&memory, LIST, 3), None, "not the start of a slot");
    }

    #[test]
    fn the_count_limits_the_records_and_the_address_is_the_documented_one() {
        let mut memory = memory();
        memory.write(LIST, 1);
        assert_eq!(talk_message(&memory, LIST, 7), None, "record 2 is past the count");
        assert_eq!(TALK_KEY, 0x03_9AFC);
        assert_eq!(TALK_RETURN, 0x03_9B7B);
        assert_eq!(KEY_BASE, 0x30);
    }
}
