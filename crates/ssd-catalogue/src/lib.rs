//! The typed game catalogue (spec §5.7): characters, enemies, encounters, cards, attacks and
//! items, read at runtime from the user's own ROM image. Nothing here holds game data: the
//! structs are filled by [`Catalogue::from_rom`] and live in memory only.
//!
//! The addresses of the tables are the `[[table]]` entries of `re/symbols.toml`
//! ([`ssd_game::data`]); `docs/re/data/` describes each table's layout. Fields whose meaning is
//! not known yet are kept raw and named `unknown_<offset>`, the offset being that of the byte in
//! its row, in hex.

pub mod art;
mod cards;
mod characters;
mod encounters;
mod enemies;
pub mod engine;
pub mod game;
pub mod graphics;
pub mod menus;
mod names;
mod rom;
mod text;
pub mod translation;

pub use art::{Portrait, face_portrait, lz_decode, object_portrait};
pub use cards::{Card, CardRules, CardValue};
pub use characters::{Character, LevelRow};
pub use encounters::{Encounter, Slot};
pub use enemies::Enemy;
pub use game::Game;
pub use menus::{labels as menu_labels, plain as menu_text, translated as translated_labels};
pub use names::{Attack, Item};
pub use rom::{MIN_ROM_SIZE, RomView};
pub use text::{Name, decode, decode_tile, is_latin};

/// Why a catalogue could not be read.
#[derive(Debug, thiserror::Error)]
pub enum CatalogueError {
    /// The image is not a headerless `LoROM` image of at least 1 MiB.
    #[error(
        "ROM image of {0} bytes: expected a headerless image of whole 32 KiB banks, at least 1 MiB"
    )]
    RomTooSmall(usize),
    /// A table or a pointer leads outside the image, or across a bank boundary.
    #[error("{len} bytes at ${address:06X} are not inside one bank of the ROM image")]
    OutOfRom { address: u32, len: usize },
    /// A table does not have the layout this crate knows: another ROM, or a wrong layout note.
    #[error("{table}: {message}")]
    Malformed { table: &'static str, message: String },
}

/// Everything the extractor knows about the game's data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalogue {
    /// The playable characters, by character id.
    pub characters: Vec<Character>,
    /// The units (enemies and other fighters), by unit id.
    pub enemies: Vec<Enemy>,
    /// The groups of enemies a battle can start with, by group index.
    pub encounters: Vec<Encounter>,
    /// How cards are dealt.
    pub cards: CardRules,
    /// The special attacks, by attack id.
    pub attacks: Vec<Attack>,
    /// The named people and objects of the story (key items among them), by object id.
    pub items: Vec<Item>,
}

impl Catalogue {
    /// Reads the catalogue from a ROM image: the canonical headerless Klepto v1.02 image
    /// (`ssd_rom::normalize`; the cache holds it).
    ///
    /// # Errors
    ///
    /// [`CatalogueError::RomTooSmall`] for an image that is not whole banks of at least 1 MiB,
    /// [`CatalogueError::OutOfRom`] and [`CatalogueError::Malformed`] when a table does not have
    /// the expected shape (a ROM other than the canonical one).
    pub fn from_rom(rom: &[u8]) -> Result<Self, CatalogueError> {
        let rom = RomView::new(rom)?;
        Ok(Self {
            characters: characters::read(rom)?,
            enemies: enemies::read(rom)?,
            encounters: encounters::read(rom)?,
            cards: cards::read(rom)?,
            attacks: names::read_attacks(rom)?,
            items: names::read_items(rom)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ROM image of zeros with the bytes `at` the physical addresses.
    pub(crate) fn rom_with(bytes: &[(u32, &[u8])]) -> Vec<u8> {
        let mut rom = vec![0_u8; MIN_ROM_SIZE];
        let view = RomView::new(&rom).unwrap();
        let offsets: Vec<usize> = bytes.iter().map(|&(a, _)| view.offset(a).unwrap()).collect();
        for (&(_, data), offset) in bytes.iter().zip(offsets) {
            rom[offset..offset + data.len()].copy_from_slice(data);
        }
        rom
    }

    #[test]
    fn an_all_zero_image_is_not_the_game() {
        let error = Catalogue::from_rom(&rom_with(&[])).unwrap_err();
        assert!(matches!(error, CatalogueError::Malformed { .. }), "{error}");
    }

    #[test]
    fn a_short_image_is_refused_with_its_size() {
        let error = Catalogue::from_rom(&[0; 1024]).unwrap_err();
        assert!(error.to_string().contains("1024"), "{error}");
    }
}
