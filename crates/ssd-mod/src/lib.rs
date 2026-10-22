//! Data packs (M10): a pack is data that adds or replaces the game's data in Enhanced mode (a
//! `pack.toml` with its `[[data]]` files, or the same as a `.ssdpack` zip). This crate holds the
//! format and the checks, the image the packs make on the ROM, and the directory the game reads
//! them by. It is a pure library: it runs no emulator and writes no game file.
//!
//! The modules: [`manifest`] (`pack.toml`), [`pack`] (a pack in memory, folder or zip), [`check`]
//! (the refusals of a set, all at once), [`image`] (the ROM and the pack data in the banks `$20` and
//! up, with the set id), [`family`] (the families and the directory's format, read by [`find`]) and
//! [`scene`] (the scene-script language, which a pack's `[[scene]]` files are written in) and [`text`] (the message
//! language, which its `[[messages]]` files are written in) and [`trigger`] (the triggers, which its `[[triggers]]`
//! file is written in) and [`tables`] (the units, attacks, groups and characters' levels, which its `[[tables]]` file is
//! written in).

pub mod chapter;
mod check;
pub mod derive;
mod family;
pub mod feature;
mod image;
pub mod lz;
mod manifest;
pub mod map;
mod pack;
pub mod pose;
pub mod scene;
pub mod set;
pub mod tables;
pub mod text;
pub mod tiled;
pub mod trigger;

use std::fmt;

pub use check::{CheckOptions, MAX_FILE_LEN, RUN_LEN, check_set};
pub use family::{Family, find};
pub use image::{FaceArt, Image, MapArt, Placement, PoseArt, ROM_LEN, build, padded};
pub use manifest::{
    ChaptersEntry, DataEntry, Dependency, FaceEntry, FeatureEntry, Manifest, MapArtEntry, MapEntry,
    MessagesEntry, PackInfo, ParamValue, PoseVariantEntry, SceneEntry, TablesEntry, TriggersEntry,
    Version, valid_id,
};
pub use pack::Pack;

/// Why a pack, a set of packs or its image is refused. Each names the pack, and the file or the
/// value, where there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackError {
    /// A folder, a zip or its manifest cannot be read or written.
    Read { path: String, message: String },
    /// A `pack.toml` is not a valid manifest (an unknown key is one of the ways).
    Manifest { path: String, message: String },
    /// A pack id, or a dependency's id, is not 3 to 64 characters of `[a-z0-9._-]`.
    BadId { pack: String, id: String },
    /// A version is not `major.minor.patch` with each part a `u16`.
    BadVersion { pack: String, text: String },
    /// The pack's `game` is not the CRC32 of the ROM.
    GameMismatch { pack: String, game: String, rom: u32 },
    /// The ROM is not exactly 1 048 576 bytes.
    RomSize { len: usize },
    /// A `[[data]]` file the pack does not hold.
    MissingFile { pack: String, file: String },
    /// A `[[data]]` file larger than 32 768 bytes.
    FileTooLarge { pack: String, file: String, len: usize },
    /// A `[[data]]` entry names a family the game does not have.
    UnknownFamily { pack: String, family: String },
    /// Two `[[data]]` entries of one pack name the same family and id.
    DuplicateEntry { pack: String, family: Family, id: u16 },
    /// Two packs fill the same family and id.
    Collision { family: Family, id: u16, first: String, second: String },
    /// Two packs have the same id.
    DuplicateId { id: String },
    /// A dependency is not in the set (`found` is `None`), or is in it below its minimum version.
    Dependency { pack: String, needs: String, wanted: Version, found: Option<Version> },
    /// Packs depend on each other in a cycle (the ids in order, back to the first).
    Cycle { packs: Vec<String> },
    /// A file holds a run of 16 or more bytes of the ROM: where in the file, and where in the ROM.
    RomBytes { pack: String, file: String, file_offset: usize, rom_offset: usize },
    /// The data does not fit the banks of the image, or the directory.
    DoesNotFit { detail: String },
    /// A `[[scene]]` file does not assemble: the error names its line and column.
    Scene { pack: String, file: String, error: scene::SceneError },
    /// A `[[messages]]` file does not compile: the error names the message, its page and line.
    Message { pack: String, file: String, error: text::MessageError },
    /// A `[[data]]` entry names a text family (`message`, `string2`, `string3`): only `[[messages]]` fills those.
    TextFamily { pack: String, family: String },
    /// A `[[map]]` file does not read or does not describe a pack map: the error names the map and its part.
    Map { pack: String, file: String, error: map::MapError },
    /// A `[[triggers]]` file is refused: the error names the trigger and what is wrong with it.
    Trigger { pack: String, file: String, error: trigger::TriggerError },
    /// A `[[tables]]` file is refused: the error names the table, its id and the field or the rule that is wrong.
    Tables { pack: String, file: String, error: tables::TablesError },
    /// The HD art of a pack is refused: a `[[face]]` picture that does not derive, or a `[[map_art]]` that does not match its map.
    Art { pack: String, file: String, detail: String },
    /// A `[[chapters]]` file is refused, or a chapter of the set is (an id given by two chapters, or a map that is not in the set).
    Chapter { pack: String, file: String, error: chapter::ChapterError },
    /// A `[[feature]]` entry is refused: the name is not one the game has, or a parameter is wrong (see [`feature`]).
    Feature { pack: String, error: feature::FeatureError },
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, message } => write!(f, "{path}: cannot be read: {message}"),
            Self::Manifest { path, message } => {
                write!(f, "{path}: pack.toml is not a valid manifest: {message}")
            }
            Self::BadId { pack, id } => write!(
                f,
                "pack `{pack}`: the id `{id}` is not 3 to 64 characters of a-z, 0-9, `.`, `_`, `-`"
            ),
            Self::BadVersion { pack, text } => write!(
                f,
                "pack `{pack}`: the version `{text}` is not major.minor.patch with each part a number up to 65535"
            ),
            Self::GameMismatch { pack, game, rom } => write!(
                f,
                "pack `{pack}`: its game `{game}` is not the CRC32 of the ROM ({rom:08X})"
            ),
            Self::RomSize { len } => {
                write!(f, "the ROM is {len} bytes, it must be exactly 1048576 (1 MiB)")
            }
            Self::MissingFile { pack, file } => {
                write!(f, "pack `{pack}`: the data file `{file}` is not in the pack")
            }
            Self::FileTooLarge { pack, file, len } => write!(
                f,
                "pack `{pack}`: the data file `{file}` is {len} bytes, the limit is 32768"
            ),
            Self::UnknownFamily { pack, family } => write!(
                f,
                "pack `{pack}`: the family `{family}` is unknown (the families are `script` and `face`)"
            ),
            Self::DuplicateEntry { pack, family, id } => {
                write!(f, "pack `{pack}`: {} {id} is given twice", family.name())
            }
            Self::Collision { family, id, first, second } => {
                write!(f, "packs `{first}` and `{second}` both fill {} {id}", family.name())
            }
            Self::DuplicateId { id } => write!(f, "two packs have the id `{id}`"),
            Self::Dependency { pack, needs, wanted, found: None } => write!(
                f,
                "pack `{pack}` needs `{needs}` at {wanted} or later, and `{needs}` is not in the set"
            ),
            Self::Dependency { pack, needs, wanted, found: Some(found) } => write!(
                f,
                "pack `{pack}` needs `{needs}` at {wanted} or later, and the set has {found}"
            ),
            Self::Cycle { packs } => {
                write!(f, "the packs depend on each other in a cycle: {}", packs.join(" -> "))
            }
            Self::RomBytes { pack, file, file_offset, rom_offset } => write!(
                f,
                "pack `{pack}`: the file `{file}` holds 16 or more bytes of the ROM (file offset {file_offset:#X}, ROM offset {rom_offset:#X})"
            ),
            Self::DoesNotFit { detail } => write!(f, "the data does not fit: {detail}"),
            Self::Message { pack, file, error } => {
                write!(f, "pack `{pack}`: the messages `{file}`: {error}")
            }
            Self::TextFamily { pack, family } => write!(
                f,
                "pack `{pack}`: the family `{family}` is written by `[[messages]]`, not by `[[data]]`"
            ),
            Self::Scene { pack, file, error } => {
                write!(f, "pack `{pack}`: the scene `{file}`: {error}")
            }
            Self::Map { pack, file, error } => {
                write!(f, "pack `{pack}`: the map `{file}`: {error}")
            }
            Self::Trigger { pack, file, error } => {
                write!(f, "pack `{pack}`: the triggers `{file}`: {error}")
            }
            Self::Tables { pack, file, error } => {
                write!(f, "pack `{pack}`: the tables `{file}`: {error}")
            }
            Self::Art { pack, file, detail } => {
                write!(f, "pack `{pack}`: the HD art `{file}`: {detail}")
            }
            Self::Chapter { pack, file, error } => {
                write!(f, "pack `{pack}`: the chapters `{file}`: {error}")
            }
            Self::Feature { pack, error } => write!(f, "pack `{pack}`: {error}"),
        }
    }
}

impl std::error::Error for PackError {}

#[cfg(test)]
pub(crate) mod testing {
    //! Synthetic packs and a synthetic ROM for the tests: never the real ROM.
    use crate::{
        DataEntry, Dependency, Manifest, MapEntry, MessagesEntry, Pack, PackInfo, ROM_LEN,
        SceneEntry, TriggersEntry,
    };
    use std::collections::BTreeMap;

    /// A 1 MiB ROM of a counter pattern (no run of it is in any pack the tests make).
    pub fn rom() -> Vec<u8> {
        (0..ROM_LEN).map(|i| (i.wrapping_mul(7) ^ (i >> 8)) as u8).collect()
    }

    /// The CRC32 (hex) of [`rom`], the `game` of the packs the tests make.
    pub fn game() -> String {
        format!("{:08X}", ssd_rom::crc32(&rom()))
    }

    /// [`rom`] with the face palette row in it: the object table's pointer for object 28 (`02:A38E`) names bank `$09`
    /// `$8000`, and that row's second word is `$1234`.
    pub fn rom_with_face_row() -> Vec<u8> {
        let mut rom = rom();
        let table = 0x02 * 0x8000 + (0xA38E - 0x8000);
        rom[table..table + 2].copy_from_slice(&0x8000_u16.to_le_bytes());
        let row = 0x09 * 0x8000;
        rom[row + 2..row + 4].copy_from_slice(&0x1234_u16.to_le_bytes());
        rom
    }

    /// A PNG of `width` x `height` pixels of RGBA `rgba`, as the packs ship their HD pictures.
    pub fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(rgba).unwrap();
        out
    }

    /// An opaque RGBA picture of `width` x `height` pixels, every pixel the same `[r, g, b]`.
    pub fn flat(width: u32, height: u32, [r, g, b]: [u8; 3]) -> Vec<u8> {
        (0..width * height).flat_map(|_| [r, g, b, 255]).collect()
    }

    /// A pack `id` at `version` that holds face `face` with the HD picture `hd` (the bytes of a PNG), for
    /// [`rom_with_face_row`].
    pub fn face_pack(id: &str, version: &str, face: u8, hd: &str, png: Vec<u8>) -> Pack {
        let mut pack = pack(id, version, &[]);
        pack.manifest.pack.game = format!("{:08X}", ssd_rom::crc32(&rom_with_face_row()));
        pack.manifest.face.push(crate::FaceEntry { id: face, hd: hd.to_owned() });
        pack.files.insert(hd.to_owned(), png);
        pack
    }

    /// A pack `id` at `version`, with one `[[data]]` entry per `(family, slot, bytes)`; the file
    /// of each entry is `data/<family>-<slot>.bin`.
    pub fn pack(id: &str, version: &str, entries: &[(&str, u16, &[u8])]) -> Pack {
        let mut data = Vec::new();
        let mut files = BTreeMap::new();
        for &(family, slot, bytes) in entries {
            let file = format!("data/{family}-{slot}.bin");
            data.push(DataEntry { family: family.to_owned(), id: slot, file: file.clone() });
            files.insert(file, bytes.to_vec());
        }
        let manifest = Manifest {
            pack: PackInfo {
                id: id.to_owned(),
                name: format!("Pack {id}"),
                version: version.to_owned(),
                game: game(),
                author: String::new(),
                description: String::new(),
            },
            depends: Vec::new(),
            data,
            scene: Vec::new(),
            messages: Vec::new(),
            map: Vec::new(),
            triggers: Vec::new(),
            map_art: Vec::new(),
            face: Vec::new(),
            tables: Vec::new(),
            chapters: Vec::new(),
            feature: Vec::new(),
            pose_variant: Vec::new(),
        };
        Pack { manifest, files }
    }

    /// A pack `id` at `version`, with one `[[scene]]` entry per `(slot, text)`; the file of each is
    /// `scenes/<slot>.scene`, holding `text`.
    pub fn scene_pack(id: &str, version: &str, scenes: &[(u16, &str)]) -> Pack {
        let mut scene = Vec::new();
        let mut files = BTreeMap::new();
        for &(slot, text) in scenes {
            let file = format!("scenes/{slot}.scene");
            scene.push(SceneEntry { id: slot, file: file.clone() });
            files.insert(file, text.as_bytes().to_vec());
        }
        let manifest = Manifest {
            pack: PackInfo {
                id: id.to_owned(),
                name: format!("Pack {id}"),
                version: version.to_owned(),
                game: game(),
                author: String::new(),
                description: String::new(),
            },
            depends: Vec::new(),
            data: Vec::new(),
            scene,
            messages: Vec::new(),
            map: Vec::new(),
            triggers: Vec::new(),
            map_art: Vec::new(),
            face: Vec::new(),
            tables: Vec::new(),
            chapters: Vec::new(),
            feature: Vec::new(),
            pose_variant: Vec::new(),
        };
        Pack { manifest, files }
    }

    /// A pack `id` at `version` whose messages file `text/messages.toml` holds `text`.
    pub fn messages_pack(id: &str, version: &str, text: &str) -> Pack {
        let file = "text/messages.toml".to_owned();
        let manifest = Manifest {
            pack: PackInfo {
                id: id.to_owned(),
                name: format!("Pack {id}"),
                version: version.to_owned(),
                game: game(),
                author: String::new(),
                description: String::new(),
            },
            depends: Vec::new(),
            data: Vec::new(),
            scene: Vec::new(),
            messages: vec![MessagesEntry { file: file.clone() }],
            map: Vec::new(),
            triggers: Vec::new(),
            map_art: Vec::new(),
            face: Vec::new(),
            tables: Vec::new(),
            chapters: Vec::new(),
            feature: Vec::new(),
            pose_variant: Vec::new(),
        };
        let files = BTreeMap::from([(file, text.as_bytes().to_vec())]);
        Pack { manifest, files }
    }

    /// A pack `id` at `version` whose Tiled map `maps/<id>.tmj` is pack map `map_id`, the file holding `text`.
    pub fn map_pack(id: &str, version: &str, map_id: u8, text: &str) -> Pack {
        let file = format!("maps/{map_id}.tmj");
        let manifest = Manifest {
            pack: PackInfo {
                id: id.to_owned(),
                name: format!("Pack {id}"),
                version: version.to_owned(),
                game: game(),
                author: String::new(),
                description: String::new(),
            },
            depends: Vec::new(),
            data: Vec::new(),
            scene: Vec::new(),
            messages: Vec::new(),
            map: vec![MapEntry { id: map_id, file: file.clone() }],
            triggers: Vec::new(),
            map_art: Vec::new(),
            face: Vec::new(),
            tables: Vec::new(),
            chapters: Vec::new(),
            feature: Vec::new(),
            pose_variant: Vec::new(),
        };
        let files = BTreeMap::from([(file, text.as_bytes().to_vec())]);
        Pack { manifest, files }
    }

    /// A pack `id` at `version` whose triggers file `triggers.toml` holds `text`.
    pub fn triggers_pack(id: &str, version: &str, text: &str) -> Pack {
        let file = "triggers.toml".to_owned();
        let manifest = Manifest {
            pack: PackInfo {
                id: id.to_owned(),
                name: format!("Pack {id}"),
                version: version.to_owned(),
                game: game(),
                author: String::new(),
                description: String::new(),
            },
            depends: Vec::new(),
            data: Vec::new(),
            scene: Vec::new(),
            messages: Vec::new(),
            map: Vec::new(),
            triggers: vec![TriggersEntry { file: file.clone() }],
            map_art: Vec::new(),
            face: Vec::new(),
            tables: Vec::new(),
            chapters: Vec::new(),
            feature: Vec::new(),
            pose_variant: Vec::new(),
        };
        let files = BTreeMap::from([(file, text.as_bytes().to_vec())]);
        Pack { manifest, files }
    }

    /// `pack` depending on `needs` at `version`.
    pub fn depending(mut pack: Pack, needs: &str, version: &str) -> Pack {
        pack.manifest
            .depends
            .push(Dependency { id: needs.to_owned(), version: version.to_owned() });
        pack
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_names_its_pack_and_its_cause() {
        let missing =
            PackError::MissingFile { pack: "example.room".into(), file: "data/x.bin".into() };
        let text = missing.to_string();
        assert!(text.contains("example.room") && text.contains("data/x.bin"), "{text}");
        let unmet = PackError::Dependency {
            pack: "a.pack".into(),
            needs: "b.pack".into(),
            wanted: Version { major: 1, minor: 2, patch: 0 },
            found: None,
        };
        let text = unmet.to_string();
        assert!(
            text.contains("a.pack") && text.contains("b.pack") && text.contains("1.2.0"),
            "{text}"
        );
        assert_eq!(Family::from_name("face"), Some(Family::Face));
        assert_eq!(Family::from_name("tile"), None);
    }

    #[test]
    fn a_scene_refusal_names_its_pack_file_line_and_column() {
        let refusal = PackError::Scene {
            pack: "example.room".into(),
            file: "scenes/82.scene".into(),
            error: scene::SceneError::at(4, 9, "`x` is not a command"),
        };
        let text = refusal.to_string();
        assert!(text.contains("example.room") && text.contains("scenes/82.scene"), "{text}");
        assert!(text.contains("line 4, column 9"), "{text}");
    }
}
