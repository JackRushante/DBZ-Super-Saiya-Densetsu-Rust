//! A pack in memory: its manifest and the bytes of the files its `[[data]]`, `[[scene]]`, `[[messages]]`, `[[map]]`,
//! `[[triggers]]` and `[[face]]` entries name, and of the tiles of its `[[map_art]]` entries, read from a folder
//! (development) or a `.ssdpack` zip (the same layout, `pack.toml` at the root).

use crate::PackError;
use crate::chapter::{self, Chapter};
use crate::feature;
use crate::manifest::{
    ChaptersEntry, Manifest, MapArtEntry, MapEntry, MessagesEntry, SceneEntry, TablesEntry,
    TriggersEntry,
};
use crate::map::{MapError, PackMap};
use crate::pose;
use crate::scene::{Assembled, SceneError, assemble};
use crate::tables;
use crate::text::{self, Compiled, MessageError};
use crate::trigger::{self, Trigger};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

/// The manifest's file name, at the root of a folder or a zip.
pub(crate) const MANIFEST: &str = "pack.toml";

/// A pack: its manifest, and the bytes of each file its `[[data]]` and `[[scene]]` entries name (keyed by
/// the name the manifest gives). Only those files are kept; a file the manifest names but the pack does not
/// hold is absent here, and [`crate::check_set`] refuses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    pub manifest: Manifest,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Pack {
    /// The pack in the folder `path` (with `pack.toml` in it). Its scene files are assembled to check them.
    ///
    /// # Errors
    ///
    /// [`PackError::Read`] if the folder or its manifest cannot be read, [`PackError::Manifest`] if the
    /// manifest is not valid, [`PackError::Scene`] if a scene file does not assemble.
    pub fn from_dir(path: &Path) -> Result<Self, PackError> {
        let shown = path.display().to_string();
        let text = std::fs::read_to_string(path.join(MANIFEST))
            .map_err(|e| PackError::Read { path: shown.clone(), message: e.to_string() })?;
        let manifest = parse_manifest(&shown, &text)?;
        let mut files = BTreeMap::new();
        for file in named_files(&manifest) {
            match std::fs::read(path.join(file)) {
                Ok(bytes) => {
                    files.insert(file.to_owned(), bytes);
                }
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(PackError::Read {
                        path: path.join(file).display().to_string(),
                        message: e.to_string(),
                    });
                }
            }
        }
        let pack =
            Self { manifest, files }.with_map_art(|name| match std::fs::read(path.join(name)) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
                Err(e) => Err(PackError::Read {
                    path: path.join(name).display().to_string(),
                    message: e.to_string(),
                }),
            })?;
        pack.check_scenes()?;
        pack.compiled_messages()?;
        pack.check_maps()?;
        pack.check_triggers()?;
        pack.check_tables()?;
        pack.check_features()?;
        pack.check_pose_variants()?;
        pack.check_chapters()?;
        Ok(pack)
    }

    /// The pack in the `.ssdpack` zip `path`: `pack.toml` at the root, the data and scene files where the
    /// manifest names them. Its scene files are assembled to check them.
    ///
    /// # Errors
    ///
    /// [`PackError::Read`] if the zip or its manifest cannot be read, [`PackError::Manifest`] if the
    /// manifest is not valid, [`PackError::Scene`] if a scene file does not assemble.
    pub fn from_zip(path: &Path) -> Result<Self, PackError> {
        let shown = path.display().to_string();
        let read_error = |message: String| PackError::Read { path: shown.clone(), message };
        let file = std::fs::File::open(path).map_err(|e| read_error(e.to_string()))?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| read_error(e.to_string()))?;
        let mut text = String::new();
        archive
            .by_name(MANIFEST)
            .and_then(|mut entry| Ok(entry.read_to_string(&mut text)?))
            .map_err(|e| read_error(format!("{MANIFEST}: {e}")))?;
        let manifest = parse_manifest(&shown, &text)?;
        let mut files = BTreeMap::new();
        for name in named_files(&manifest) {
            let read = archive.by_name(name).and_then(|mut file| {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                Ok(bytes)
            });
            match read {
                Ok(bytes) => {
                    files.insert(name.to_owned(), bytes);
                }
                Err(zip::result::ZipError::FileNotFound) => {}
                Err(e) => return Err(read_error(format!("{name}: {e}"))),
            }
        }
        let pack = Self { manifest, files }.with_map_art(|name| {
            let read = archive.by_name(name).and_then(|mut file| {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                Ok(bytes)
            });
            match read {
                Ok(bytes) => Ok(Some(bytes)),
                Err(zip::result::ZipError::FileNotFound) => Ok(None),
                Err(e) => Err(read_error(format!("{name}: {e}"))),
            }
        })?;
        pack.check_scenes()?;
        pack.compiled_messages()?;
        pack.check_maps()?;
        pack.check_triggers()?;
        pack.check_tables()?;
        pack.check_features()?;
        pack.check_pose_variants()?;
        pack.check_chapters()?;
        Ok(pack)
    }

    /// The pack at `path`: a folder with `pack.toml`, or a `.ssdpack` zip.
    ///
    /// # Errors
    ///
    /// As [`Pack::from_dir`] or [`Pack::from_zip`].
    pub fn open(path: &Path) -> Result<Self, PackError> {
        if path.is_dir() { Self::from_dir(path) } else { Self::from_zip(path) }
    }

    /// Writes the pack as a zip at `path` (`pack.toml` and the data files), the `.ssdpack` form.
    ///
    /// # Errors
    ///
    /// [`PackError::Read`] (naming the file) if the zip cannot be written.
    pub fn write_zip(&self, path: &Path) -> Result<(), PackError> {
        let shown = path.display().to_string();
        let write_error = |message: String| PackError::Read { path: shown.clone(), message };
        let manifest = toml::to_string(&self.manifest).map_err(|e| write_error(e.to_string()))?;
        let file = std::fs::File::create(path).map_err(|e| write_error(e.to_string()))?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
            zip.start_file(name, options).map_err(|e| format!("{name}: {e}"))?;
            zip.write_all(bytes).map_err(|e| format!("{name}: {e}"))
        };
        put(MANIFEST, manifest.as_bytes()).map_err(write_error)?;
        for (name, bytes) in &self.files {
            put(name, bytes).map_err(write_error)?;
        }
        zip.finish().map_err(|e| write_error(e.to_string()))?;
        Ok(())
    }

    /// The scene script of `entry`, assembled (see [`crate::scene::assemble`]); `None` when the pack does not
    /// hold its file (the checks of [`crate::check_set`] refuse that).
    ///
    /// # Errors
    ///
    /// [`PackError::Scene`], naming the pack and the file and the line and column of the error, when the file is
    /// not UTF-8 text or does not assemble.
    pub fn scene(&self, entry: &SceneEntry) -> Result<Option<Assembled>, PackError> {
        let Some(bytes) = self.files.get(&entry.file) else { return Ok(None) };
        let refused = |error: SceneError| PackError::Scene {
            pack: self.manifest.pack.id.clone(),
            file: entry.file.clone(),
            error,
        };
        let text = std::str::from_utf8(bytes)
            .map_err(|_| refused(SceneError::whole("the file is not UTF-8 text")))?;
        assemble(text).map(Some).map_err(refused)
    }

    /// The messages of `entry`, compiled (see [`crate::text`]); `None` when the pack does not hold its file (the
    /// checks of [`crate::check_set`] refuse that).
    ///
    /// # Errors
    ///
    /// [`PackError::Message`], naming the pack and the file and the message, page and line of the error.
    pub fn messages(&self, entry: &MessagesEntry) -> Result<Option<Vec<Compiled>>, PackError> {
        let Some(bytes) = self.files.get(&entry.file) else { return Ok(None) };
        let refused = |error: MessageError| PackError::Message {
            pack: self.manifest.pack.id.clone(),
            file: entry.file.clone(),
            error,
        };
        let text = std::str::from_utf8(bytes)
            .map_err(|_| refused(MessageError::File("the file is not UTF-8 text".into())))?;
        text::compile(text).map(Some).map_err(refused)
    }

    /// Every message of the pack, in the order its `[[messages]]` files and their messages give them.
    ///
    /// # Errors
    ///
    /// As [`Pack::messages`].
    pub(crate) fn compiled_messages(&self) -> Result<Vec<Compiled>, PackError> {
        let mut all = Vec::new();
        for entry in &self.manifest.messages {
            all.extend(self.messages(entry)?.unwrap_or_default());
        }
        Ok(all)
    }

    /// The pack map of `entry`, read from its Tiled file and checked (see [`crate::tiled`] and [`crate::map`]); `None` when the
    /// pack does not hold its file (the checks of [`crate::check_set`] refuse that).
    ///
    /// # Errors
    ///
    /// [`PackError::Map`], naming the pack, the file and the part of the map that is refused.
    pub fn map(&self, entry: &MapEntry) -> Result<Option<PackMap>, PackError> {
        let Some(bytes) = self.files.get(&entry.file) else { return Ok(None) };
        let refused = |error: MapError| PackError::Map {
            pack: self.manifest.pack.id.clone(),
            file: entry.file.clone(),
            error,
        };
        let text = std::str::from_utf8(bytes)
            .map_err(|_| refused(MapError::Tiled("the file is not UTF-8 text".into())))?;
        crate::tiled::parse(text, entry.id).map(Some).map_err(refused)
    }

    /// Reads every map of the pack, so that a map that is refused is refused at load.
    fn check_maps(&self) -> Result<(), PackError> {
        for entry in &self.manifest.map {
            self.map(entry)?;
        }
        Ok(())
    }

    /// The triggers of `entry`, read and checked (see [`crate::trigger`]); `None` when the pack does not hold its file (the
    /// checks of [`crate::check_set`] refuse that).
    ///
    /// # Errors
    ///
    /// [`PackError::Trigger`], naming the pack, the file and the trigger that is refused.
    pub fn triggers(&self, entry: &TriggersEntry) -> Result<Option<Vec<Trigger>>, PackError> {
        let Some(bytes) = self.files.get(&entry.file) else { return Ok(None) };
        let refused = |error: trigger::TriggerError| PackError::Trigger {
            pack: self.manifest.pack.id.clone(),
            file: entry.file.clone(),
            error,
        };
        let text = std::str::from_utf8(bytes).map_err(|_| {
            refused(trigger::TriggerError::File("the file is not UTF-8 text".into()))
        })?;
        trigger::parse(text).map(Some).map_err(refused)
    }

    /// The triggers of every `[[triggers]]` file of the pack, in the order its manifest gives them.
    ///
    /// # Errors
    ///
    /// As [`Pack::triggers`].
    pub(crate) fn compiled_triggers(&self) -> Result<Vec<Trigger>, PackError> {
        let mut all = Vec::new();
        for entry in &self.manifest.triggers {
            all.extend(self.triggers(entry)?.unwrap_or_default());
        }
        Ok(all)
    }

    /// Reads every triggers file of the pack, so that a file that is refused is refused at load.
    fn check_triggers(&self) -> Result<(), PackError> {
        for entry in &self.manifest.triggers {
            self.triggers(entry)?;
        }
        Ok(())
    }

    /// The slots of the tables file `entry` (see [`crate::tables`]), compiled and checked; `None` when the pack does not hold
    /// its file (the checks of [`crate::check_set`] refuse that).
    ///
    /// # Errors
    ///
    /// [`PackError::Tables`], naming the pack, the file and the table row that is refused.
    pub fn tables(&self, entry: &TablesEntry) -> Result<Option<Vec<tables::Compiled>>, PackError> {
        let Some(bytes) = self.files.get(&entry.file) else { return Ok(None) };
        let refused = |error: tables::TablesError| PackError::Tables {
            pack: self.manifest.pack.id.clone(),
            file: entry.file.clone(),
            error,
        };
        let text = std::str::from_utf8(bytes).map_err(|_| {
            refused(tables::TablesError::Parse("the file is not UTF-8 text".into()))
        })?;
        tables::compile(text).map(Some).map_err(refused)
    }

    /// Compiles every tables file of the pack, so that a file that is refused is refused at load.
    fn check_tables(&self) -> Result<(), PackError> {
        for entry in &self.manifest.tables {
            self.tables(entry)?;
        }
        Ok(())
    }

    /// The feature slots of the pack (see [`crate::feature`]), checked: one per `[[feature]]` entry, its bytes its parameters.
    ///
    /// # Errors
    ///
    /// [`PackError::Feature`], naming the pack and the feature that is refused.
    pub fn features(&self) -> Result<Vec<tables::Compiled>, PackError> {
        feature::compile(&self.manifest.feature)
            .map_err(|error| PackError::Feature { pack: self.manifest.pack.id.clone(), error })
    }

    /// Checks the features of the pack, so that a feature that is refused is refused at load.
    fn check_features(&self) -> Result<(), PackError> {
        self.features().map(|_| ())
    }

    /// Checks the pose variants of the pack, so that one that is refused is refused at load (see [`crate::pose`]): each
    /// names a condition the game has and a margin of whole game pixels, and an object's variant for a condition is given once
    /// (an object may have one `always` variant and one for each specific condition).
    fn check_pose_variants(&self) -> Result<(), PackError> {
        let refused = |file: &str, detail: String| PackError::Art {
            pack: self.manifest.pack.id.clone(),
            file: file.to_owned(),
            detail,
        };
        let mut given = BTreeSet::new();
        for entry in &self.manifest.pose_variant {
            if let Some(detail) = pose::refusal(entry) {
                return Err(refused(&entry.file, detail));
            }
            if !given.insert((entry.art.as_str(), entry.when.as_str())) {
                let detail = format!(
                    "the object `{}` has a variant for `{}` already",
                    entry.art, entry.when
                );
                return Err(refused(&entry.file, detail));
            }
        }
        Ok(())
    }

    /// The chapters of `entry`, read and checked (see [`crate::chapter`]); `None` when the pack does not hold its file (the
    /// checks of [`crate::check_set`] refuse that).
    ///
    /// # Errors
    ///
    /// [`PackError::Chapter`], naming the pack, the file and the chapter that is refused.
    pub fn chapters(&self, entry: &ChaptersEntry) -> Result<Option<Vec<Chapter>>, PackError> {
        let Some(bytes) = self.files.get(&entry.file) else { return Ok(None) };
        let refused = |error: chapter::ChapterError| PackError::Chapter {
            pack: self.manifest.pack.id.clone(),
            file: entry.file.clone(),
            error,
        };
        let text = std::str::from_utf8(bytes).map_err(|_| {
            refused(chapter::ChapterError::File("the file is not UTF-8 text".into()))
        })?;
        chapter::parse(text).map(Some).map_err(refused)
    }

    /// Reads every chapters file of the pack, so that a file that is refused is refused at load.
    fn check_chapters(&self) -> Result<(), PackError> {
        for entry in &self.manifest.chapters {
            self.chapters(entry)?;
        }
        Ok(())
    }

    /// The tile files of the `[[map_art]]` entry `entry`: one per tile of its map, in rows, named by the tile's column and
    /// row. `None` when the pack has no `[[map]]` of that id or the map does not read (the checks refuse that).
    #[must_use]
    pub fn map_art_tiles(&self, entry: &MapArtEntry) -> Option<Vec<String>> {
        let map = self.manifest.map.iter().find(|map| map.id == entry.map)?;
        let Ok(Some(pack_map)) = self.map(map) else { return None };
        let (width, height) = (usize::from(pack_map.width), usize::from(pack_map.height));
        Some((0..height).flat_map(|y| (0..width).map(move |x| entry.tile_file(x, y))).collect())
    }

    /// Reads the tile files of the pack's `[[map_art]]` entries with `read` (`Ok(None)` for a file the pack does not hold),
    /// once the maps' files are read: their size says how many tiles there are.
    fn with_map_art(
        mut self,
        mut read: impl FnMut(&str) -> Result<Option<Vec<u8>>, PackError>,
    ) -> Result<Self, PackError> {
        let names: Vec<String> = self
            .manifest
            .map_art
            .iter()
            .filter_map(|entry| self.map_art_tiles(entry))
            .flatten()
            .collect();
        for name in names {
            if let Some(bytes) = read(&name)? {
                self.files.insert(name, bytes);
            }
        }
        Ok(self)
    }

    /// Assembles every scene of the pack, so that a scene that does not assemble is refused at load.
    fn check_scenes(&self) -> Result<(), PackError> {
        for entry in &self.manifest.scene {
            self.scene(entry)?;
        }
        Ok(())
    }
}

/// The files the manifest names: the data files, the scene files, the messages files, the map files, the triggers files, the
/// tables files, the face pictures and the pose variants' pictures (the tiles of the `[[map_art]]` entries are named by their maps: see [`Pack::with_map_art`]).
fn named_files(manifest: &Manifest) -> impl Iterator<Item = &str> {
    manifest
        .data
        .iter()
        .map(|entry| entry.file.as_str())
        .chain(manifest.scene.iter().map(|entry| entry.file.as_str()))
        .chain(manifest.messages.iter().map(|entry| entry.file.as_str()))
        .chain(manifest.map.iter().map(|entry| entry.file.as_str()))
        .chain(manifest.triggers.iter().map(|entry| entry.file.as_str()))
        .chain(manifest.tables.iter().map(|entry| entry.file.as_str()))
        .chain(manifest.chapters.iter().map(|entry| entry.file.as_str()))
        .chain(manifest.face.iter().map(|entry| entry.hd.as_str()))
        .chain(manifest.pose_variant.iter().map(|entry| entry.file.as_str()))
}

fn parse_manifest(shown: &str, text: &str) -> Result<Manifest, PackError> {
    toml::from_str(text)
        .map_err(|e| PackError::Manifest { path: shown.to_owned(), message: e.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{messages_pack, pack, scene_pack, triggers_pack};
    use crate::text::MessageError;

    /// A scratch folder for one test, removed by the test itself.
    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ssd-mod-{name}-{}", std::process::id()))
    }

    #[test]
    fn a_pose_variant_is_refused_at_load_when_its_condition_is_unknown_and_needs_no_file() {
        let dir = scratch("pose-variant");
        std::fs::create_dir_all(&dir).unwrap();
        let mut original = pack("example.room", "1.0.0", &[]);
        let entry = crate::PoseVariantEntry {
            art: "battle-goku-ssj-000".into(),
            when: "bloom".into(),
            file: "art/hd/variants/battle-goku-ssj-000.png".into(),
            margin: 24,
        };
        original.manifest.pose_variant.push(entry.clone());
        let zip_path = dir.join("example.ssdpack");
        original.write_zip(&zip_path).unwrap();
        let refused = Pack::open(&zip_path).unwrap_err();
        assert!(
            matches!(&refused, PackError::Art { detail, .. } if detail.contains("bloom")),
            "{refused:?}"
        );
        // Known condition, no picture in the pack: the pack is read, the variant is not drawn
        original.manifest.pose_variant =
            vec![crate::PoseVariantEntry { when: "kaioken".into(), ..entry.clone() }];
        original.write_zip(&zip_path).unwrap();
        assert_eq!(Pack::open(&zip_path).unwrap(), original);
        // Two variants of one object for one condition
        original.manifest.pose_variant = vec![entry.clone(), entry.clone()];
        original.write_zip(&zip_path).unwrap();
        assert!(Pack::open(&zip_path).is_err(), "a second variant for the same condition");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_object_may_have_one_always_variant_and_one_kaioken_variant() {
        let dir = scratch("pose-variant-always");
        std::fs::create_dir_all(&dir).unwrap();
        let mut original = pack("example.room", "1.0.0", &[]);
        let variant = |when: &str, file: &str| crate::PoseVariantEntry {
            art: "battle-goku-ssj-000".into(),
            when: when.into(),
            file: file.into(),
            margin: 24,
        };
        let always = variant("always", "art/hd/variants/always.png");
        let kaioken = variant("kaioken", "art/hd/variants/kaioken.png");
        original.manifest.pose_variant = vec![always.clone(), kaioken];
        let zip_path = dir.join("example.ssdpack");
        original.write_zip(&zip_path).unwrap();
        assert_eq!(Pack::open(&zip_path).unwrap(), original, "one for each condition");
        // A second `always` variant of the object, with another picture
        original.manifest.pose_variant.push(variant("always", "art/hd/variants/other.png"));
        original.write_zip(&zip_path).unwrap();
        let refused = Pack::open(&zip_path).unwrap_err();
        assert!(
            matches!(&refused, PackError::Art { detail, .. } if detail.contains("for `always` already")),
            "{refused:?}"
        );
        // Another object's `always` variant is its own
        original.manifest.pose_variant = vec![
            always,
            crate::PoseVariantEntry {
                art: "battle-goku-ssj-001".into(),
                ..variant("always", "b.png")
            },
        ];
        original.write_zip(&zip_path).unwrap();
        assert!(Pack::open(&zip_path).is_ok(), "two objects, one `always` each");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_zip_and_a_folder_read_back_the_pack_they_hold() {
        let dir = scratch("roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let original =
            pack("example.room", "1.0.0", &[("script", 82, b"hello"), ("face", 3, &[7; 99])]);
        let zip_path = dir.join("example.ssdpack");
        original.write_zip(&zip_path).unwrap();
        assert_eq!(Pack::open(&zip_path).unwrap(), original, "the zip");
        let folder = dir.join("folder");
        std::fs::create_dir_all(folder.join("data")).unwrap();
        std::fs::write(folder.join("pack.toml"), toml::to_string(&original.manifest).unwrap())
            .unwrap();
        for (name, bytes) in &original.files {
            std::fs::write(folder.join(name), bytes).unwrap();
        }
        assert_eq!(Pack::open(&folder).unwrap(), original, "the folder");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_data_file_the_folder_lacks_is_left_out_for_the_check_to_name() {
        let dir = scratch("missing");
        std::fs::create_dir_all(&dir).unwrap();
        let original = pack("example.room", "1.0.0", &[("script", 82, b"hello")]);
        std::fs::write(dir.join("pack.toml"), toml::to_string(&original.manifest).unwrap())
            .unwrap();
        let read = Pack::from_dir(&dir).unwrap();
        assert_eq!(read.manifest, original.manifest);
        assert!(read.files.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_tiles_of_a_map_art_entry_are_read_with_the_map_from_a_zip_and_a_folder() {
        let dir = scratch("map-art");
        std::fs::create_dir_all(&dir).unwrap();
        let tiny = r#"{ "type": "map", "orientation": "orthogonal", "width": 2, "height": 1, "tilewidth": 256, "tileheight": 256,
          "properties": [ { "name": "music_like", "type": "int", "value": 34 }, { "name": "palette_like", "type": "int", "value": 34 } ],
          "tilesets": [ { "firstgid": 1, "name": "blocks", "tilecount": 120 } ],
          "layers": [ { "type": "tilelayer", "name": "blocks", "width": 2, "height": 1, "data": [1, 1] },
            { "type": "objectgroup", "name": "objects", "objects": [] },
            { "type": "objectgroup", "name": "zones", "objects": [] },
            { "type": "objectgroup", "name": "start", "objects": [ { "name": "start", "x": 256, "y": 128 } ] } ] }"#;
        let mut original = crate::testing::map_pack("example.room", "1.0.0", 64, tiny);
        original
            .manifest
            .map_art
            .push(crate::MapArtEntry { map: 64, tiles: "art/hd/maps/64/t{x}_{y}.png".into() });
        for name in ["art/hd/maps/64/t0_0.png", "art/hd/maps/64/t1_0.png"] {
            original.files.insert(name.into(), b"a tile".to_vec());
        }
        let zip_path = dir.join("map-art.ssdpack");
        original.write_zip(&zip_path).unwrap();
        assert_eq!(Pack::open(&zip_path).unwrap(), original, "the tiles are in the zip");
        let folder = dir.join("folder");
        std::fs::create_dir_all(folder.join("maps")).unwrap();
        std::fs::write(folder.join(MANIFEST), toml::to_string(&original.manifest).unwrap())
            .unwrap();
        std::fs::write(folder.join("maps/64.tmj"), tiny).unwrap();
        for (name, bytes) in &original.files {
            if name.starts_with("art/") {
                std::fs::create_dir_all(folder.join(name).parent().unwrap()).unwrap();
                std::fs::write(folder.join(name), bytes).unwrap();
            }
        }
        // The map's own file is named by the manifest as it is written: the same pack as the zip
        let read = Pack::open(&folder).unwrap();
        assert_eq!(
            read.files.get("art/hd/maps/64/t1_0.png").map(Vec::as_slice),
            Some(&b"a tile"[..])
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_scene_file_is_read_from_a_zip_and_assembled_at_load() {
        let dir = scratch("scene-zip");
        std::fs::create_dir_all(&dir).unwrap();
        let original = scene_pack("scene.pack", "1.0.0", &[(82, "scene s:\n    end\n")]);
        let zip_path = dir.join("scene.ssdpack");
        original.write_zip(&zip_path).unwrap();
        let read = Pack::open(&zip_path).unwrap();
        assert_eq!(read, original, "the scene file is in the zip");
        let entry = &read.manifest.scene[0];
        assert_eq!(read.scene(entry).unwrap().unwrap().bytes, [0x01]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_scene_that_does_not_assemble_is_refused_at_load_with_its_line() {
        let dir = scratch("scene-bad");
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        let bad = scene_pack("scene.pack", "1.0.0", &[(82, "scene s:\n    end\n    jmp 3\n")]);
        std::fs::write(dir.join("pack.toml"), toml::to_string(&bad.manifest).unwrap()).unwrap();
        std::fs::write(dir.join("scenes/82.scene"), &bad.files["scenes/82.scene"]).unwrap();
        match Pack::from_dir(&dir) {
            Err(PackError::Scene { pack, file, error }) => {
                assert_eq!(pack, "scene.pack");
                assert_eq!(file, "scenes/82.scene");
                assert_eq!((error.line, error.column), (3, 5), "{error}");
            }
            other => panic!("{other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_messages_file_is_read_from_a_zip_and_compiled() {
        let dir = scratch("messages-zip");
        std::fs::create_dir_all(&dir).unwrap();
        let text = "[[message]]\nid = 400\nwindow = \"dialogue\"\npages = [[\"ok.\"]]\n";
        let original = messages_pack("example.room", "1.0.0", text);
        let zip_path = dir.join("messages.ssdpack");
        original.write_zip(&zip_path).unwrap();
        let read = Pack::open(&zip_path).unwrap();
        assert_eq!(read, original, "the messages file is in the zip");
        let compiled = read.compiled_messages().unwrap();
        assert_eq!(compiled.len(), 1);
        assert_eq!(
            (compiled[0].id, compiled[0].string.as_slice()),
            (400, &[0x1B, 0x17, 0x7C, 0xFF, 0xFF][..])
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_chapters_file_that_is_refused_is_refused_at_load_with_its_chapter() {
        let dir = scratch("chapters-bad");
        std::fs::create_dir_all(&dir).unwrap();
        let text = "[[chapter]]\nid = \"Bad\"\nname = \"x\"\nmap = 0\nleader = [1, 1]\nfollower = [1, 1]\nparty = [{ character = 0, level = 1 }]\n";
        let mut bad = pack("example.room", "1.0.0", &[]);
        bad.manifest.chapters.push(crate::manifest::ChaptersEntry { file: "chapters.toml".into() });
        bad.files.insert("chapters.toml".into(), text.as_bytes().to_vec());
        std::fs::write(dir.join("pack.toml"), toml::to_string(&bad.manifest).unwrap()).unwrap();
        std::fs::write(dir.join("chapters.toml"), text).unwrap();
        match Pack::from_dir(&dir) {
            Err(PackError::Chapter { pack, file, error }) => {
                assert_eq!((pack.as_str(), file.as_str()), ("example.room", "chapters.toml"));
                assert!(error.to_string().starts_with("chapter 1: the id `Bad`"), "{error}");
            }
            other => panic!("{other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_triggers_file_that_is_refused_is_refused_at_load_with_its_trigger() {
        let dir = scratch("triggers-bad");
        std::fs::create_dir_all(&dir).unwrap();
        let text = "[[trigger]]\nmap = 64\nrect = [0, 0, 8, 8]\nscene = 1\n\
                    [[trigger]]\nmap = 64\nrect = [0, 0, 8, 8]\nwhen = [\"flag 128\"]\nscene = 1\n";
        let bad = triggers_pack("example.room", "1.0.0", text);
        std::fs::write(dir.join("pack.toml"), toml::to_string(&bad.manifest).unwrap()).unwrap();
        std::fs::write(dir.join("triggers.toml"), &bad.files["triggers.toml"]).unwrap();
        match Pack::from_dir(&dir) {
            Err(PackError::Trigger { pack, file, error }) => {
                assert_eq!((pack.as_str(), file.as_str()), ("example.room", "triggers.toml"));
                assert!(
                    error.to_string().starts_with("trigger 2: the condition \"flag 128\""),
                    "{error}"
                );
            }
            other => panic!("{other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_messages_file_that_does_not_compile_is_refused_at_load_with_its_line() {
        let dir = scratch("messages-bad");
        std::fs::create_dir_all(dir.join("text")).unwrap();
        let bad = messages_pack(
            "example.room",
            "1.0.0",
            "[[message]]\nid = 5\nwindow = \"narration\"\npages = [[\"ok\", \"a;b\"]]\n",
        );
        std::fs::write(dir.join("pack.toml"), toml::to_string(&bad.manifest).unwrap()).unwrap();
        std::fs::write(dir.join("text/messages.toml"), &bad.files["text/messages.toml"]).unwrap();
        match Pack::from_dir(&dir) {
            Err(PackError::Message { pack, file, error }) => {
                assert_eq!((pack.as_str(), file.as_str()), ("example.room", "text/messages.toml"));
                assert_eq!(
                    error,
                    MessageError::Character { id: 5, page: 1, line: 2, ch: ';', column: 2 }
                );
            }
            other => panic!("{other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
