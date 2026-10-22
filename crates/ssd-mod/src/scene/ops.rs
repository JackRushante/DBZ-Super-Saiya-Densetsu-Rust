//! The scene commands: one table that the assembler and the decompiler both use.
//!
//! Opcodes `$00` to `$32` are the original engine's 51 commands (`docs/re/data/scenes.md`). The keyword
//! of each is its name in `tools/re/scenes.py`, except three that share a name there: `$08`, `$09` and
//! `$0A` are `nop`, `nop9` and `nopa`, and `$32` is `start2`. Opcodes `$33` to `$35` are the extension
//! commands of the packs (`extension` is set): the original engine does not have them, and the game runs
//! them from a Rust wrapper (`ssd_game::mods`). The operand kinds are the ones `scenes.md` gives a
//! command; a command whose operands it does not name is given plain bytes or words.

use Kind::{
    Address, Byte, Character, ExtFlag, Flag, Group, Label, Map, Message, Object, Sound, Word,
};

/// What an operand is: its size, its largest value and what it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A byte of plain value (a count, a timer, a number the engine reads as it is).
    Byte,
    /// A word of plain value (a coordinate).
    Word,
    /// A story flag: one byte; flag `n` is bit `n % 8` of byte `n / 8` of the 16 bytes at `$0170`.
    Flag,
    /// An extended flag (the `flagx.*` commands): a word, 0 to 2047 (see `ssd_game::mods`).
    ExtFlag,
    /// A script address in the scene's bank: a label name, or a number taken as an offset from the
    /// scene's start (the same as the label at that offset).
    Label,
    /// A message of the map's table: a word.
    Message,
    /// A map object's slot or id: one byte.
    Object,
    /// A map id: one byte.
    Map,
    /// An encounter group: one byte.
    Group,
    /// A character id: one byte.
    Character,
    /// A sound or music number: one byte.
    Sound,
    /// A bank 0 work-RAM address, as `poke`, `zero` and `inc` take it: a word.
    Address,
}

impl Kind {
    /// The size of an operand of this kind, in bytes.
    #[must_use]
    pub const fn size(self) -> usize {
        match self {
            Self::Byte
            | Self::Flag
            | Self::Object
            | Self::Map
            | Self::Group
            | Self::Character
            | Self::Sound => 1,
            Self::Word | Self::ExtFlag | Self::Label | Self::Message | Self::Address => 2,
        }
    }

    /// The largest value an operand of this kind takes.
    #[must_use]
    pub const fn max(self) -> u32 {
        match self {
            Self::ExtFlag => 2047,
            _ if self.size() == 1 => 255,
            _ => 65535,
        }
    }

    /// The kind as a phrase for an error message.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Byte => "a byte",
            Self::Word => "a word",
            Self::Flag => "a flag",
            Self::ExtFlag => "an extended flag",
            Self::Label => "a label",
            Self::Message => "a message",
            Self::Object => "an object",
            Self::Map => "a map",
            Self::Group => "a group",
            Self::Character => "a character",
            Self::Sound => "a sound",
            Self::Address => "an address",
        }
    }
}

/// One command: its opcode, its keyword, its operands in order, and whether it is an extension command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Op {
    /// The opcode byte.
    pub code: u8,
    /// The keyword the text uses.
    pub keyword: &'static str,
    /// The kinds of the operands, in the order they follow the opcode.
    pub operands: &'static [Kind],
    /// Whether the command is an extension (`$33` and up), which the original engine does not have.
    pub extension: bool,
}

impl Op {
    /// The length of the command in bytes: the opcode and its operands.
    #[must_use]
    pub fn length(&self) -> usize {
        1 + self.operands.iter().map(|kind| kind.size()).sum::<usize>()
    }
}

const fn op(code: u8, keyword: &'static str, operands: &'static [Kind]) -> Op {
    Op { code, keyword, operands, extension: false }
}

const fn extension(code: u8, keyword: &'static str, operands: &'static [Kind]) -> Op {
    Op { code, keyword, operands, extension: true }
}

/// Every command, in opcode order: `$00` to `$35`.
pub const OPS: &[Op] = &[
    op(0x00, "start", &[]),
    op(0x01, "end", &[]),
    op(0x02, "picture", &[Byte]),
    op(0x03, "object", &[Object, Word, Word]),
    op(0x04, "move", &[Byte, Byte]),
    op(0x05, "text", &[Message]),
    op(0x06, "sound", &[Sound]),
    op(0x07, "music", &[Sound]),
    op(0x08, "nop", &[]),
    op(0x09, "nop9", &[]),
    op(0x0A, "nopa", &[]),
    op(0x0B, "ifflag", &[Flag, Label]),
    op(0x0C, "setflag", &[Flag]),
    op(0x0D, "clrflag", &[Flag]),
    op(0x0E, "sprite", &[Byte, Byte, Word, Word]),
    op(0x0F, "move2", &[Byte, Byte]),
    op(0x10, "wait", &[Byte]),
    op(0x11, "battle", &[Group, Byte, Label]),
    op(0x12, "jump", &[Label]),
    op(0x13, "poke", &[Address, Byte]),
    op(0x14, "screen14", &[]),
    op(0x15, "copyobj", &[Object, Byte]),
    op(0x16, "redraw", &[]),
    op(0x17, "camera", &[Byte]),
    op(0x18, "screen18", &[]),
    op(0x19, "place", &[Word, Word]),
    op(0x1A, "zero", &[Address]),
    op(0x1B, "effect", &[Byte]),
    op(0x1C, "addmember", &[Character, Byte]),
    op(0x1D, "portrait", &[Byte, Byte, Byte]),
    op(0x1E, "fxsetup", &[Byte, Byte, Byte]),
    op(0x1F, "call", &[Label]),
    op(0x20, "return", &[]),
    op(0x21, "mode", &[Byte]),
    op(0x22, "savepos", &[]),
    op(0x23, "restpos", &[]),
    op(0x24, "fx", &[Byte, Byte]),
    op(0x25, "call2", &[Label]),
    op(0x26, "return2", &[]),
    op(0x27, "camera2", &[Byte]),
    op(0x28, "fade", &[Byte]),
    op(0x29, "tomode", &[Byte]),
    op(0x2A, "rmobj", &[Object]),
    op(0x2B, "join", &[Character]),
    op(0x2C, "gomap", &[Byte, Map, Word, Word]),
    op(0x2D, "screen2d", &[]),
    op(0x2E, "scene", &[Byte]),
    op(0x2F, "inc", &[Address]),
    op(0x30, "battle2", &[Label]),
    op(0x31, "party", &[]),
    op(0x32, "start2", &[]),
    extension(0x33, "flagx.set", &[ExtFlag]),
    extension(0x34, "flagx.clear", &[ExtFlag]),
    extension(0x35, "flagx.if", &[ExtFlag, Label]),
];

/// The command with opcode `code`, `None` above `$35`.
#[must_use]
pub fn by_code(code: u8) -> Option<&'static Op> {
    OPS.iter().find(|op| op.code == code)
}

/// The command with keyword `keyword`, `None` for any other word.
#[must_use]
pub fn by_keyword(keyword: &str) -> Option<&'static Op> {
    OPS.iter().find(|op| op.keyword == keyword)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The operand bytes of each opcode `$00` to `$32` in `tools/re/scenes.py` (its `OPS` table).
    const SCENES_PY_OPERAND_BYTES: [usize; 51] = [
        0, 0, 1, 5, 2, 2, 1, 1, 0, 0, 0, 3, 1, 1, 6, 2, 1, 4, 2, 3, 0, 2, 0, 1, 0, 4, 2, 1, 2, 3,
        3, 2, 0, 1, 0, 0, 2, 2, 0, 1, 1, 1, 1, 1, 6, 0, 1, 2, 2, 0, 0,
    ];

    #[test]
    fn every_opcode_is_in_the_table_once_and_the_extensions_come_last() {
        for code in 0..=0x35_u8 {
            let op = by_code(code).unwrap_or_else(|| panic!("opcode {code:02X}"));
            assert_eq!(op.code, code);
            assert_eq!(op.extension, code >= 0x33, "opcode {code:02X}");
        }
        assert!(by_code(0x36).is_none());
        assert_eq!(OPS.len(), 0x36);
    }

    #[test]
    fn the_original_commands_have_the_lengths_of_the_scripts_decompiler() {
        assert_eq!(SCENES_PY_OPERAND_BYTES.len(), 51);
        for (code, &bytes) in SCENES_PY_OPERAND_BYTES.iter().enumerate() {
            let op = by_code(code as u8).unwrap();
            assert_eq!(op.length(), 1 + bytes, "opcode {code:02X} ({})", op.keyword);
        }
    }

    #[test]
    fn every_keyword_names_one_command_and_the_extensions_have_three_lengths() {
        let mut keywords: Vec<&str> = OPS.iter().map(|op| op.keyword).collect();
        keywords.sort_unstable();
        keywords.dedup();
        assert_eq!(keywords.len(), OPS.len(), "a keyword is shared");
        assert_eq!(by_keyword("nop9").map(|op| op.code), Some(0x09));
        assert_eq!(by_keyword("start2").map(|op| op.code), Some(0x32));
        assert_eq!(by_keyword("start").map(|op| op.code), Some(0x00));
        assert_eq!(by_keyword("flagx.if").map(Op::length), Some(5));
        assert_eq!(by_keyword("flagx.set").map(Op::length), Some(3));
        assert_eq!(by_keyword("no such word"), None);
    }

    #[test]
    fn the_battle_commands_end_in_their_won_label() {
        assert_eq!(by_code(0x11).unwrap().operands, [Group, Byte, Label]);
        assert_eq!(by_code(0x30).unwrap().operands, [Label]);
        assert_eq!(by_code(0x0B).unwrap().operands, [Flag, Label]);
    }
}
