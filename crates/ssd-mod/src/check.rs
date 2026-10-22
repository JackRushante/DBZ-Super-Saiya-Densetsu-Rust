//! The checks of a set of packs against the ROM. Every refusal is collected: a set is
//! either accepted, or refused with the whole list of what is wrong with it.

use crate::chapter;
use crate::derive::{self, MAP_ART_TILE};
use crate::image::{self, ROM_LEN};
use crate::manifest::{Version, valid_id};
use crate::{Family, Pack, PackError};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// The largest file a pack may carry: 32 768 bytes.
pub const MAX_FILE_LEN: usize = 0x8000;
/// The length of a run of ROM bytes a pack file may not hold (the same scan as `publish-check`).
pub const RUN_LEN: usize = 16;

/// What [`check_set`] allows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CheckOptions {
    /// Lets a file hold runs of the ROM's bytes. Only for a local build, whose output is never
    /// tracked; the default refuses them.
    pub allow_rom_bytes: bool,
}

/// Checks `packs` for the ROM `rom`: the ids and versions, the game's CRC32, the file sizes, the
/// slots (within a pack and across packs), the dependencies and their cycles, the ROM bytes in the
/// files (unless `options` allows them) and whether the data fits the banks and the directory.
///
/// # Errors
///
/// Every refusal found, in the order the checks run: the ROM's size, then per pack (in the given
/// order) its identity, files and entries, then the set (duplicate ids, collisions, dependencies,
/// cycles), the ROM bytes, and the fit.
pub fn check_set(rom: &[u8], packs: &[Pack], options: CheckOptions) -> Result<(), Vec<PackError>> {
    let mut errors = Vec::new();
    if rom.len() != ROM_LEN {
        errors.push(PackError::RomSize { len: rom.len() });
    }
    let rom_crc32 = ssd_rom::crc32(rom);
    let mut first_by_id: BTreeMap<&str, usize> = BTreeMap::new();
    let mut slots: BTreeMap<(Family, u16), &str> = BTreeMap::new();
    let mut all_chapters: Vec<(String, String, chapter::Chapter)> = Vec::new();
    for (index, pack) in packs.iter().enumerate() {
        let info = &pack.manifest.pack;
        let id = info.id.as_str();
        if !valid_id(id) {
            errors.push(PackError::BadId { pack: id.to_owned(), id: id.to_owned() });
        }
        if first_by_id.insert(id, index).is_some() {
            errors.push(PackError::DuplicateId { id: id.to_owned() });
        }
        if Version::parse(&info.version).is_none() {
            errors.push(PackError::BadVersion { pack: id.to_owned(), text: info.version.clone() });
        }
        if u32::from_str_radix(&info.game, 16).ok() != Some(rom_crc32) {
            errors.push(PackError::GameMismatch {
                pack: id.to_owned(),
                game: info.game.clone(),
                rom: rom_crc32,
            });
        }
        for dependency in &pack.manifest.depends {
            if !valid_id(&dependency.id) {
                errors.push(PackError::BadId { pack: id.to_owned(), id: dependency.id.clone() });
            }
            if Version::parse(&dependency.version).is_none() {
                errors.push(PackError::BadVersion {
                    pack: id.to_owned(),
                    text: dependency.version.clone(),
                });
            }
        }
        // The slots of the pack: its data entries, then its scenes (the `script` family).
        let mut entries: Vec<(Family, u16, &str)> = Vec::new();
        for entry in &pack.manifest.data {
            match Family::from_name(&entry.family) {
                Some(family) if family.is_text() => errors.push(PackError::TextFamily {
                    pack: id.to_owned(),
                    family: entry.family.clone(),
                }),
                Some(family) => entries.push((family, entry.id, entry.file.as_str())),
                None => errors.push(PackError::UnknownFamily {
                    pack: id.to_owned(),
                    family: entry.family.clone(),
                }),
            }
        }
        for entry in &pack.manifest.scene {
            // The game keeps the scene id in one byte (`$01D8`): a larger id would load and never start.
            if entry.id > 0xFF {
                errors.push(PackError::Manifest {
                    path: format!("{id}: {}", entry.file),
                    message: format!(
                        "scene id {} is over 255, the largest scene id the game can start",
                        entry.id
                    ),
                });
                continue;
            }
            entries.push((Family::Script, entry.id, entry.file.as_str()));
        }
        // The maps: each `[[map]]` fills the map slot of its id (its four families, one slot).
        for entry in &pack.manifest.map {
            entries.push((Family::MapHeader, u16::from(entry.id), entry.file.as_str()));
        }
        let mut own: BTreeSet<(Family, u16)> = BTreeSet::new();
        for (family, slot, file) in entries {
            match pack.files.get(file) {
                None => errors
                    .push(PackError::MissingFile { pack: id.to_owned(), file: file.to_owned() }),
                Some(bytes) if bytes.len() > MAX_FILE_LEN => errors.push(PackError::FileTooLarge {
                    pack: id.to_owned(),
                    file: file.to_owned(),
                    len: bytes.len(),
                }),
                Some(_) => {}
            }
            if !own.insert((family, slot)) {
                errors.push(PackError::DuplicateEntry { pack: id.to_owned(), family, id: slot });
            }
            if let Some(first) = slots.insert((family, slot), id)
                && first != id
            {
                errors.push(PackError::Collision {
                    family,
                    id: slot,
                    first: first.to_owned(),
                    second: id.to_owned(),
                });
            }
        }
        for entry in &pack.manifest.scene {
            if let Err(error) = pack.scene(entry) {
                errors.push(error);
            }
        }
        for entry in &pack.manifest.map {
            if let Err(error) = pack.map(entry) {
                errors.push(error);
            }
        }
        // The messages: each message fills its message slot, like a data entry (the strings are placed by the image).
        for entry in &pack.manifest.messages {
            match pack.messages(entry) {
                Ok(None) => errors
                    .push(PackError::MissingFile { pack: id.to_owned(), file: entry.file.clone() }),
                Ok(Some(messages)) => {
                    for message in messages {
                        if !own.insert((Family::Message, message.id)) {
                            errors.push(PackError::DuplicateEntry {
                                pack: id.to_owned(),
                                family: Family::Message,
                                id: message.id,
                            });
                        }
                        if let Some(first) = slots.insert((Family::Message, message.id), id)
                            && first != id
                        {
                            errors.push(PackError::Collision {
                                family: Family::Message,
                                id: message.id,
                                first: first.to_owned(),
                                second: id.to_owned(),
                            });
                        }
                    }
                }
                Err(error) => errors.push(error),
            }
        }
        // The triggers: each file is read and checked; the set's triggers are counted by the fit.
        for entry in &pack.manifest.triggers {
            match pack.triggers(entry) {
                Ok(None) => errors
                    .push(PackError::MissingFile { pack: id.to_owned(), file: entry.file.clone() }),
                Ok(Some(_)) => {}
                Err(error) => errors.push(error),
            }
        }
        // The tables: each file is compiled, and each slot it fills is counted like a data slot (a unit, a group, a level cap...).
        for entry in &pack.manifest.tables {
            match pack.tables(entry) {
                Ok(None) => errors
                    .push(PackError::MissingFile { pack: id.to_owned(), file: entry.file.clone() }),
                Ok(Some(items)) => {
                    for item in items {
                        if !own.insert((item.family, item.id)) {
                            errors.push(PackError::DuplicateEntry {
                                pack: id.to_owned(),
                                family: item.family,
                                id: item.id,
                            });
                        }
                        if let Some(first) = slots.insert((item.family, item.id), id)
                            && first != id
                        {
                            errors.push(PackError::Collision {
                                family: item.family,
                                id: item.id,
                                first: first.to_owned(),
                                second: id.to_owned(),
                            });
                        }
                    }
                }
                Err(error) => errors.push(error),
            }
        }
        // The features: each `[[feature]]` is a slot of the feature family, numbered by the feature; a name the game does not have,
        // or a parameter that is wrong, is refused. Two packs that enable one feature collide like two data entries.
        match pack.features() {
            Ok(items) => {
                for item in items {
                    if !own.insert((item.family, item.id)) {
                        errors.push(PackError::DuplicateEntry {
                            pack: id.to_owned(),
                            family: item.family,
                            id: item.id,
                        });
                    }
                    if let Some(first) = slots.insert((item.family, item.id), id)
                        && first != id
                    {
                        errors.push(PackError::Collision {
                            family: item.family,
                            id: item.id,
                            first: first.to_owned(),
                            second: id.to_owned(),
                        });
                    }
                }
            }
            Err(error) => errors.push(error),
        }
        // The chapters: each file is read and checked; the set's chapters are checked together after the packs (an id given twice,
        // a map of the set).
        for entry in &pack.manifest.chapters {
            match pack.chapters(entry) {
                Ok(None) => errors
                    .push(PackError::MissingFile { pack: id.to_owned(), file: entry.file.clone() }),
                Ok(Some(chapters)) => {
                    all_chapters.extend(
                        chapters.into_iter().map(|one| (id.to_owned(), entry.file.clone(), one)),
                    );
                }
                Err(error) => errors.push(error),
            }
        }
        // The faces: each is a slot of the face family (like a data entry), and its picture must derive (the derivation
        // of the image names the picture and what is wrong with it).
        for entry in &pack.manifest.face {
            let slot = (Family::Face, u16::from(entry.id));
            if !own.insert(slot) {
                errors.push(PackError::DuplicateEntry {
                    pack: id.to_owned(),
                    family: slot.0,
                    id: slot.1,
                });
            }
            if let Some(first) = slots.insert(slot, id)
                && first != id
            {
                errors.push(PackError::Collision {
                    family: slot.0,
                    id: slot.1,
                    first: first.to_owned(),
                    second: id.to_owned(),
                });
            }
            if !pack.files.contains_key(&entry.hd) {
                errors.push(PackError::MissingFile { pack: id.to_owned(), file: entry.hd.clone() });
            }
        }
        if let Err(error) = image::faces(rom, std::slice::from_ref(pack)) {
            errors.push(error);
        }
        // The HD tiles of the pack maps: the map must be the pack's, and each tile of it must be in the pack, a PNG of a tile's size.
        for entry in &pack.manifest.map_art {
            let refused = |detail: String| PackError::Art {
                pack: id.to_owned(),
                file: entry.tiles.clone(),
                detail,
            };
            if !(entry.tiles.contains("{x}") && entry.tiles.contains("{y}")) {
                errors.push(refused("the tiles' file name needs {x} and {y}".to_owned()));
                continue;
            }
            if !pack.manifest.map.iter().any(|map| map.id == entry.map) {
                errors
                    .push(refused(format!("map {} has no [[map]] entry in this pack", entry.map)));
                continue;
            }
            // (a map that does not read is refused by its own entry above)
            for name in pack.map_art_tiles(entry).unwrap_or_default() {
                match pack.files.get(&name) {
                    None => errors.push(PackError::MissingFile { pack: id.to_owned(), file: name }),
                    Some(bytes) => match derive::png_size(bytes) {
                        Ok((width, height)) if (width, height) == (MAP_ART_TILE, MAP_ART_TILE) => {}
                        Ok((width, height)) => errors.push(refused(format!(
                            "{name} is {width} x {height} pixels: a tile is {MAP_ART_TILE} x {MAP_ART_TILE}"
                        ))),
                        Err(error) => errors.push(refused(format!("{name}: {error}"))),
                    },
                }
            }
        }
    }
    let maps: Vec<u8> =
        packs.iter().flat_map(|pack| pack.manifest.map.iter().map(|map| map.id)).collect();
    for (pack, file, error) in chapter::set_refusals(&all_chapters, &maps) {
        errors.push(PackError::Chapter { pack, file, error });
    }
    dependency_errors(packs, &first_by_id, &mut errors);
    cycle_errors(packs, &first_by_id, &mut errors);
    if !options.allow_rom_bytes && rom.len() == ROM_LEN {
        rom_byte_errors(rom, packs, &mut errors);
    }
    if let Err(error) = image::fits(rom, packs) {
        errors.push(error);
    }
    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

/// Each dependency that is not in the set, or is in it below its minimum version. A dependency whose
/// own version text is not valid is refused where that pack is checked.
fn dependency_errors(
    packs: &[Pack],
    first_by_id: &BTreeMap<&str, usize>,
    errors: &mut Vec<PackError>,
) {
    for pack in packs {
        for dependency in &pack.manifest.depends {
            let Some(wanted) = Version::parse(&dependency.version) else { continue };
            let needed = first_by_id
                .get(dependency.id.as_str())
                .map(|&index| Version::parse(&packs[index].manifest.pack.version));
            let found = match needed {
                None => None,
                Some(None) => continue,
                Some(Some(found)) if found >= wanted => continue,
                Some(Some(found)) => Some(found),
            };
            errors.push(PackError::Dependency {
                pack: pack.manifest.pack.id.clone(),
                needs: dependency.id.clone(),
                wanted,
                found,
            });
        }
    }
}

/// Each dependency cycle among the packs of the set (a back edge of a depth-first walk).
fn cycle_errors(packs: &[Pack], first_by_id: &BTreeMap<&str, usize>, errors: &mut Vec<PackError>) {
    // 0: not visited, 1: on the walk's stack, 2: done.
    let mut state = vec![0_u8; packs.len()];
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..packs.len() {
        visit(start, packs, first_by_id, &mut state, &mut stack, errors);
    }
}

fn visit(
    at: usize,
    packs: &[Pack],
    first_by_id: &BTreeMap<&str, usize>,
    state: &mut [u8],
    stack: &mut Vec<usize>,
    errors: &mut Vec<PackError>,
) {
    if state[at] != 0 {
        if state[at] == 1 {
            let from = stack.iter().position(|&pack| pack == at).unwrap_or(0);
            let cycle = stack[from..].iter().map(|&pack| packs[pack].manifest.pack.id.clone());
            errors.push(PackError::Cycle { packs: cycle.collect() });
        }
        return;
    }
    state[at] = 1;
    stack.push(at);
    for dependency in &packs[at].manifest.depends {
        if let Some(&next) = first_by_id.get(dependency.id.as_str()) {
            visit(next, packs, first_by_id, state, stack, errors);
        }
    }
    stack.pop();
    state[at] = 2;
}

/// A refusal for each file that holds a run of [`RUN_LEN`] bytes of the ROM.
fn rom_byte_errors(rom: &[u8], packs: &[Pack], errors: &mut Vec<PackError>) {
    // A window of one repeated byte (the ROM's fill of zeros or `$FF`) is not the game's data: a pack may hold it.
    let windows: HashSet<[u8; RUN_LEN]> = rom
        .windows(RUN_LEN)
        .filter(|window| !is_fill(window))
        .map(|window| {
            let mut key = [0_u8; RUN_LEN];
            key.copy_from_slice(window);
            key
        })
        .collect();
    for pack in packs {
        for (file, bytes) in &pack.files {
            if let Some((file_offset, rom_offset)) = rom_run(rom, &windows, bytes) {
                errors.push(PackError::RomBytes {
                    pack: pack.manifest.pack.id.clone(),
                    file: file.clone(),
                    file_offset,
                    rom_offset,
                });
            }
        }
        // A scene's assembled script is scanned too: its text may write ROM bytes as numbers. Its offset is
        // the offset in the script.
        for entry in &pack.manifest.scene {
            if let Ok(Some(script)) = pack.scene(entry)
                && let Some((file_offset, rom_offset)) = rom_run(rom, &windows, &script.bytes)
            {
                errors.push(PackError::RomBytes {
                    pack: pack.manifest.pack.id.clone(),
                    file: entry.file.clone(),
                    file_offset,
                    rom_offset,
                });
            }
        }
    }
}

/// Whether `window` is one byte repeated.
fn is_fill(window: &[u8]) -> bool {
    window.iter().all(|&byte| byte == window[0])
}

/// The first run of [`RUN_LEN`] bytes of `bytes` that is also in the ROM: its offset in `bytes` and
/// its offset in the ROM (the first one there). `windows` holds the ROM's windows (see
/// [`rom_byte_errors`]).
#[must_use]
pub(crate) fn rom_run(
    rom: &[u8],
    windows: &HashSet<[u8; RUN_LEN]>,
    bytes: &[u8],
) -> Option<(usize, usize)> {
    bytes.windows(RUN_LEN).enumerate().find_map(|(at, window)| {
        let mut key = [0_u8; RUN_LEN];
        key.copy_from_slice(window);
        if !windows.contains(&key) {
            return None;
        }
        let rom_at = rom.windows(RUN_LEN).position(|candidate| candidate == window)?;
        Some((at, rom_at))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{depending, messages_pack, pack, rom, scene_pack, triggers_pack};
    use crate::text::MessageError;

    fn refused(rom: &[u8], packs: &[Pack], options: CheckOptions) -> Vec<PackError> {
        check_set(rom, packs, options).expect_err("the set should be refused")
    }

    /// Bytes that are not in the counter ROM: a run of them is not a ROM run.
    const NOT_ROM: &[u8] = &[0xA5; 40];

    #[test]
    fn a_valid_set_passes() {
        let packs = [pack("example.room", "1.0.0", &[("script", 82, b"abc")])];
        assert_eq!(check_set(&rom(), &packs, CheckOptions::default()), Ok(()));
        assert_eq!(check_set(&rom(), &[], CheckOptions::default()), Ok(()), "the empty set");
    }

    #[test]
    fn a_bad_id_is_refused() {
        let packs = [pack("Bad Id", "1.0.0", &[])];
        let errors = refused(&rom(), &packs, CheckOptions::default());
        assert!(matches!(errors.as_slice(), [PackError::BadId { .. }]), "{errors:?}");
    }

    #[test]
    fn a_bad_version_is_refused() {
        let packs = [pack("example.room", "1.0", &[])];
        let errors = refused(&rom(), &packs, CheckOptions::default());
        assert!(matches!(errors.as_slice(), [PackError::BadVersion { .. }]), "{errors:?}");
    }

    #[test]
    fn a_game_that_is_not_the_rom_crc_is_refused() {
        let mut other = pack("example.room", "1.0.0", &[]);
        other.manifest.pack.game = "DEADBEEF".into();
        let errors = refused(&rom(), &[other], CheckOptions::default());
        assert!(matches!(errors.as_slice(), [PackError::GameMismatch { .. }]), "{errors:?}");
    }

    #[test]
    fn a_rom_that_is_not_one_mebibyte_is_refused() {
        let short = &rom()[..ROM_LEN - 1];
        let errors = refused(short, &[], CheckOptions::default());
        assert_eq!(errors, [PackError::RomSize { len: ROM_LEN - 1 }]);
    }

    #[test]
    fn a_missing_data_file_is_refused() {
        let mut packs = [pack("example.room", "1.0.0", &[("script", 82, b"abc")])];
        packs[0].files.clear();
        let errors = refused(&rom(), &packs, CheckOptions::default());
        assert!(
            matches!(errors.as_slice(), [PackError::MissingFile { file, .. }] if file == "data/script-82.bin"),
            "{errors:?}"
        );
    }

    #[test]
    fn a_file_over_32768_bytes_is_refused() {
        let big = vec![0x11_u8; MAX_FILE_LEN + 1];
        let packs = [pack("example.room", "1.0.0", &[("face", 1, &big)])];
        let errors = refused(&rom(), &packs, CheckOptions::default());
        assert!(
            matches!(errors.as_slice(), [PackError::FileTooLarge { len, .. }] if *len == MAX_FILE_LEN + 1),
            "{errors:?}"
        );
        let exactly = vec![0x11_u8; MAX_FILE_LEN];
        let packs = [pack("example.room", "1.0.0", &[("face", 1, &exactly)])];
        assert_eq!(check_set(&rom(), &packs, CheckOptions::default()), Ok(()), "the limit itself");
    }

    #[test]
    fn the_same_slot_twice_in_one_pack_is_refused() {
        let packs = [pack("example.room", "1.0.0", &[("script", 5, b"a"), ("script", 5, b"b")])];
        let errors = refused(&rom(), &packs, CheckOptions::default());
        assert!(
            matches!(errors.as_slice(), [PackError::DuplicateEntry { id: 5, .. }]),
            "{errors:?}"
        );
    }

    /// A tables file of one new unit, `id` (its stats are the ones of the tests of `crate::tables`).
    fn unit_tables(id: u16) -> Vec<u8> {
        format!(
            "[[unit]]
id = {id}
name = \"Guard\"
class = 1
level = 5
hp = 300
bp = 100
ki = 50
             defence = 30
tie = 7
exp = 0
sprite_like = 4
[[unit.command]]
id = 0
attr = 16
factor = 4
"
        )
        .into_bytes()
    }

    #[test]
    fn two_packs_that_give_one_unit_are_a_collision_and_a_tables_file_is_counted_as_its_slots() {
        let mut first = pack("first.pack", "1.0.0", &[]);
        first.manifest.tables.push(crate::TablesEntry { file: "tables.toml".into() });
        first.files.insert("tables.toml".into(), unit_tables(105));
        let mut second = pack("second.pack", "1.0.0", &[]);
        second.manifest.tables.push(crate::TablesEntry { file: "tables.toml".into() });
        second.files.insert("tables.toml".into(), unit_tables(105));
        let errors = refused(&rom(), &[first.clone(), second], CheckOptions::default());
        // The unit's row, its name and its command list: one collision for each family.
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors.iter().any(|error| matches!(
            error,
            PackError::Collision { family: Family::Unit, id: 105, .. }
        )));
        // A pack alone with the same file is fine, and a file of one pack that gives a unit twice is refused.
        assert!(check_set(&rom(), &[first.clone()], CheckOptions::default()).is_ok());
        let mut twice = first;
        let both = format!(
            "{}{}",
            String::from_utf8(unit_tables(105)).unwrap(),
            String::from_utf8(unit_tables(105)).unwrap()
        );
        twice.files.insert("tables.toml".into(), both.into_bytes());
        let errors = refused(&rom(), &[twice], CheckOptions::default());
        assert!(errors.iter().any(|error| matches!(error, PackError::Tables { .. })), "{errors:?}");
    }

    #[test]
    fn the_same_slot_in_two_packs_is_a_collision() {
        let packs = [
            pack("first.pack", "1.0.0", &[("script", 82, b"a")]),
            pack("second.pack", "1.0.0", &[("script", 82, b"b")]),
        ];
        let errors = refused(&rom(), &packs, CheckOptions::default());
        match errors.as_slice() {
            [PackError::Collision { family: Family::Script, id: 82, first, second }] => {
                assert_eq!((first.as_str(), second.as_str()), ("first.pack", "second.pack"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_missing_dependency_and_one_below_its_version_are_refused() {
        let needs = depending(pack("needs.this", "1.0.0", &[]), "other.pack", "1.2.0");
        let missing = refused(&rom(), std::slice::from_ref(&needs), CheckOptions::default());
        assert!(
            matches!(missing.as_slice(), [PackError::Dependency { found: None, .. }]),
            "{missing:?}"
        );
        let low = [needs, pack("other.pack", "1.1.9", &[])];
        let below = refused(&rom(), &low, CheckOptions::default());
        assert!(
            matches!(below.as_slice(), [PackError::Dependency { found: Some(found), .. }] if found.minor == 1),
            "{below:?}"
        );
        let enough = [
            depending(pack("needs.this", "1.0.0", &[]), "other.pack", "1.2.0"),
            pack("other.pack", "1.2.0", &[]),
        ];
        assert_eq!(check_set(&rom(), &enough, CheckOptions::default()), Ok(()), "at the minimum");
    }

    #[test]
    fn a_dependency_cycle_is_refused() {
        let packs = [
            depending(pack("a.pack", "1.0.0", &[]), "b.pack", "1.0.0"),
            depending(pack("b.pack", "1.0.0", &[]), "a.pack", "1.0.0"),
        ];
        let errors = refused(&rom(), &packs, CheckOptions::default());
        assert!(
            errors.iter().any(|e| matches!(e, PackError::Cycle { packs } if packs.len() == 2)),
            "{errors:?}"
        );
    }

    #[test]
    fn two_packs_with_one_id_are_refused() {
        let packs = [pack("same.pack", "1.0.0", &[]), pack("same.pack", "2.0.0", &[])];
        let errors = refused(&rom(), &packs, CheckOptions::default());
        assert!(matches!(errors.as_slice(), [PackError::DuplicateId { .. }]), "{errors:?}");
    }

    #[test]
    fn data_that_does_not_fit_is_refused() {
        // 94 files of 32 KiB: the 4 MiB image has 93 banks that take one each (bank $20 keeps the directory).
        let big = vec![0x22_u8; MAX_FILE_LEN];
        let entries: Vec<(&str, u16, &[u8])> =
            (0..94).map(|slot| ("script", slot, big.as_slice())).collect();
        let errors =
            refused(&rom(), &[pack("big.pack", "1.0.0", &entries)], CheckOptions::default());
        assert!(matches!(errors.as_slice(), [PackError::DoesNotFit { .. }]), "{errors:?}");
        // The directory holds 680 entries (8 + 8 + 6 * 680 = 4096 bytes): 700 tiny slots do not fit.
        let tiny = [0x33_u8];
        let entries: Vec<(&str, u16, &[u8])> =
            (0..700).map(|slot| ("script", slot, tiny.as_slice())).collect();
        let errors =
            refused(&rom(), &[pack("many.pack", "1.0.0", &entries)], CheckOptions::default());
        assert!(matches!(errors.as_slice(), [PackError::DoesNotFit { .. }]), "{errors:?}");
    }

    #[test]
    fn a_file_with_a_run_of_rom_bytes_is_refused_unless_allowed() {
        let rom = rom();
        let mut run = NOT_ROM[..8].to_vec();
        run.extend_from_slice(&rom[0x4321..0x4321 + 20]);
        let packs = [pack("example.room", "1.0.0", &[("face", 3, &run)])];
        let errors = refused(&rom, &packs, CheckOptions::default());
        match errors.as_slice() {
            [PackError::RomBytes { file_offset, rom_offset, .. }] => {
                assert_eq!((*file_offset, *rom_offset), (8, 0x4321));
            }
            other => panic!("{other:?}"),
        }
        let allowed = CheckOptions { allow_rom_bytes: true };
        assert_eq!(check_set(&rom, &packs, allowed), Ok(()), "a local build may hold them");
        let short = [pack("example.room", "1.0.0", &[("face", 3, &rom[0x4321..0x4321 + 15])])];
        assert_eq!(
            check_set(&rom, &short, CheckOptions::default()),
            Ok(()),
            "15 bytes are not a run"
        );
    }

    #[test]
    fn a_run_of_one_repeated_byte_is_not_rom_data() {
        let mut rom = vec![0_u8; 4096];
        rom[100..200].fill(0xFF);
        let windows: HashSet<[u8; RUN_LEN]> = rom
            .windows(RUN_LEN)
            .filter(|w| !is_fill(w))
            .map(|w| <[u8; RUN_LEN]>::try_from(w).unwrap())
            .collect();
        assert_eq!(rom_run(&rom, &windows, &[0; 64]), None);
        assert_eq!(rom_run(&rom, &windows, &[0xFF; 64]), None);
    }

    #[test]
    fn rom_run_reports_the_first_run_and_no_run_when_there_is_none() {
        let rom = rom();
        let windows: HashSet<[u8; RUN_LEN]> =
            rom.windows(RUN_LEN).map(|w| <[u8; RUN_LEN]>::try_from(w).unwrap()).collect();
        assert_eq!(rom_run(&rom, &windows, NOT_ROM), None);
        let mut bytes = NOT_ROM.to_vec();
        bytes.extend_from_slice(&rom[0x100..0x110]);
        assert_eq!(rom_run(&rom, &windows, &bytes), Some((40, 0x100)));
    }

    #[test]
    fn a_triggers_file_that_is_missing_or_refused_is_refused_with_its_trigger() {
        let text = "[[trigger]]\nmap = 64\nrect = [0, 0, 8, 8]\nscene = 1\n";
        let mut missing = triggers_pack("example.room", "1.0.0", text);
        missing.files.clear();
        let errors = refused(&rom(), &[missing], CheckOptions::default());
        assert!(
            errors.iter().any(
                |e| matches!(e, PackError::MissingFile { file, .. } if file == "triggers.toml")
            ),
            "{errors:?}"
        );
        let bad = triggers_pack(
            "example.room",
            "1.0.0",
            &format!("{text}[[trigger]]\nmap = 64\nrect = [0, 0, 8, 8]\nscene = 300\n"),
        );
        let errors = refused(&rom(), &[bad], CheckOptions::default());
        assert!(
            errors.iter().any(|e| matches!(
                e,
                PackError::Trigger {
                    error: crate::trigger::TriggerError::Trigger { index: 2, .. },
                    ..
                }
            )),
            "{errors:?}"
        );
    }

    #[test]
    fn every_refusal_of_a_set_is_reported_at_once() {
        let mut bad = pack("Bad Id", "1.0", &[("script", 1, b"x")]);
        bad.manifest.pack.game = "00000000".into();
        bad.files.clear();
        let errors = refused(&rom(), &[bad], CheckOptions::default());
        let has = |want: fn(&PackError) -> bool| errors.iter().any(want);
        assert!(has(|e| matches!(e, PackError::BadId { .. })), "{errors:?}");
        assert!(has(|e| matches!(e, PackError::BadVersion { .. })), "{errors:?}");
        assert!(has(|e| matches!(e, PackError::GameMismatch { .. })), "{errors:?}");
        assert!(has(|e| matches!(e, PackError::MissingFile { .. })), "{errors:?}");
        assert!(errors.len() >= 4, "{errors:?}");
    }

    #[test]
    fn a_scene_takes_the_script_slot_of_its_id_and_is_checked_like_a_data_slot() {
        let clash = pack("example.room", "1.0.0", &[("script", 82, b"abc")]);
        let mut both = scene_pack("example.scene", "1.0.0", &[(82, "scene s:\n    end\n")]);
        both.manifest.data = clash.manifest.data.clone();
        both.files.extend(clash.files.clone());
        let errors = refused(&rom(), &[both], CheckOptions::default());
        assert!(
            matches!(
                errors.as_slice(),
                [PackError::DuplicateEntry { family: Family::Script, id: 82, .. }]
            ),
            "{errors:?}"
        );
        let mut missing = scene_pack("example.scene", "1.0.0", &[(7, "scene s:\n    end\n")]);
        missing.files.clear();
        let errors = refused(&rom(), &[missing], CheckOptions::default());
        assert!(
            matches!(errors.as_slice(), [PackError::MissingFile { file, .. }] if file == "scenes/7.scene"),
            "{errors:?}"
        );
        let first = scene_pack("first.scene", "1.0.0", &[(9, "scene s:\n    end\n")]);
        let second = scene_pack("second.scene", "1.0.0", &[(9, "scene t:\n    end\n")]);
        let errors = refused(&rom(), &[first, second], CheckOptions::default());
        assert!(matches!(errors.as_slice(), [PackError::Collision { id: 9, .. }]), "{errors:?}");
    }

    #[test]
    fn a_scene_id_over_255_is_refused() {
        let big = scene_pack(
            "example.scene",
            "1.0.0",
            &[(
                256,
                "scene s:
    end
",
            )],
        );
        let errors = check_set(&rom(), &[big], CheckOptions::default()).unwrap_err();
        assert!(errors.iter().any(|e| e.to_string().contains("over 255")), "{errors:?}");
    }

    #[test]
    fn a_scene_that_does_not_assemble_is_refused_with_its_line_by_the_set() {
        let bad = scene_pack("example.scene", "1.0.0", &[(5, "scene s:\n    wait 999\n")]);
        let errors = refused(&rom(), &[bad], CheckOptions::default());
        match errors.as_slice() {
            [PackError::Scene { file, error, .. }] => {
                assert_eq!(file, "scenes/5.scene");
                assert_eq!((error.line, error.column), (2, 10), "{error}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_scene_script_that_holds_a_run_of_rom_bytes_is_refused_unless_allowed() {
        // A ROM whose bytes at $4321 are eight `wait $05` commands: a script that writes them is a run of the ROM.
        let mut rom = rom();
        let window: Vec<u8> = (0..8).flat_map(|_| [0x10, 0x05]).collect();
        rom[0x4321..0x4321 + 16].copy_from_slice(&window);
        let text = format!("scene s:\n{}", "    wait $05\n".repeat(8));
        let mut copied = scene_pack("example.scene", "1.0.0", &[(3, &text)]);
        copied.manifest.pack.game = format!("{:08X}", ssd_rom::crc32(&rom));
        let errors = refused(&rom, std::slice::from_ref(&copied), CheckOptions::default());
        assert!(
            matches!(errors.as_slice(), [PackError::RomBytes { file, file_offset: 0, .. }] if file == "scenes/3.scene"),
            "{errors:?}"
        );
        let allowed = CheckOptions { allow_rom_bytes: true };
        assert_eq!(check_set(&rom, &[copied], allowed), Ok(()), "a local build may hold them");
    }

    /// A message in a messages file: one line, one page, `id` in `window`.
    fn message(id: u16, window: &str, line: &str) -> String {
        format!("[[message]]\nid = {id}\nwindow = \"{window}\"\npages = [[\"{line}\"]]\n")
    }

    #[test]
    fn a_face_picture_that_does_not_derive_is_refused_with_its_size() {
        let rom = crate::testing::rom_with_face_row();
        let hd = crate::testing::png(64, 64, &crate::testing::flat(64, 64, [0, 0, 0]));
        let packs = [crate::testing::face_pack("face.pack", "1.0.0", 0, "art/hd/faces/a.png", hd)];
        let errors = check_set(&rom, &packs, CheckOptions::default()).unwrap_err();
        assert!(
            matches!(errors.as_slice(), [PackError::Art { file, detail, .. }] if file == "art/hd/faces/a.png" && detail.contains("64 x 64")),
            "{errors:?}"
        );
        let good = crate::testing::png(128, 128, &crate::testing::flat(128, 128, [0, 0, 0]));
        let packs =
            [crate::testing::face_pack("face.pack", "1.0.0", 0, "art/hd/faces/a.png", good)];
        assert!(check_set(&rom, &packs, CheckOptions::default()).is_ok());
    }

    #[test]
    fn map_art_needs_its_map_and_a_tile_per_block_of_the_right_size() {
        let rom = crate::testing::rom();
        let tiny = r#"{ "type": "map", "orientation": "orthogonal", "width": 2, "height": 1, "tilewidth": 256, "tileheight": 256,
          "properties": [ { "name": "music_like", "type": "int", "value": 34 }, { "name": "palette_like", "type": "int", "value": 34 } ],
          "tilesets": [ { "firstgid": 1, "name": "blocks", "tilecount": 120 } ],
          "layers": [ { "type": "tilelayer", "name": "blocks", "width": 2, "height": 1, "data": [1, 1] },
            { "type": "objectgroup", "name": "objects", "objects": [] },
            { "type": "objectgroup", "name": "zones", "objects": [] },
            { "type": "objectgroup", "name": "start", "objects": [ { "name": "start", "x": 256, "y": 128 } ] } ] }"#;
        let mut pack = crate::testing::map_pack("example.room", "1.0.0", 64, tiny);
        pack.manifest
            .map_art
            .push(crate::MapArtEntry { map: 64, tiles: "art/hd/maps/64/t{x}_{y}.png".into() });
        pack.manifest
            .map_art
            .push(crate::MapArtEntry { map: 65, tiles: "art/hd/maps/65/t{x}_{y}.png".into() });
        // The tile of column 0 is 16 x 16, which is not the size of a tile; the tile of column 1 is not in the pack
        pack.files.insert(
            "art/hd/maps/64/t0_0.png".into(),
            crate::testing::png(16, 16, &crate::testing::flat(16, 16, [9; 3])),
        );
        let errors = check_set(&rom, &[pack], CheckOptions { allow_rom_bytes: true }).unwrap_err();
        assert!(
            errors.iter().any(|e| matches!(e, PackError::Art { file, detail, .. }
                if file == "art/hd/maps/64/t{x}_{y}.png" && detail.contains("16 x 16"))),
            "{errors:?}"
        );
        assert!(
            errors.iter().any(|e| matches!(e, PackError::MissingFile { file, .. } if file == "art/hd/maps/64/t1_0.png")),
            "{errors:?}"
        );
        assert!(
            errors.iter().any(
                |e| matches!(e, PackError::Art { detail, .. } if detail.contains("no [[map]]"))
            ),
            "{errors:?}"
        );
    }

    #[test]
    fn a_text_family_is_not_a_data_entry() {
        let packs = [pack("example.room", "1.0.0", &[("string2", 366, b"abc")])];
        let errors = refused(&rom(), &packs, CheckOptions::default());
        assert!(
            matches!(errors.as_slice(), [PackError::TextFamily { family, .. }] if family == "string2"),
            "{errors:?}"
        );
    }

    #[test]
    fn a_message_id_fills_a_slot_that_two_packs_cannot_share() {
        let packs = [
            messages_pack("first.pack", "1.0.0", &message(400, "dialogue", "ok.")),
            messages_pack("second.pack", "1.0.0", &message(400, "dialogue", "no.")),
        ];
        let errors = refused(&rom(), &packs, CheckOptions::default());
        assert!(
            matches!(
                errors.as_slice(),
                [PackError::Collision { family: Family::Message, id: 400, .. }]
            ),
            "{errors:?}"
        );
    }

    #[test]
    fn a_message_id_twice_in_one_pack_is_refused() {
        let twice =
            format!("{}{}", message(400, "dialogue", "ok."), message(400, "narration", "no."));
        let packs = [messages_pack("example.room", "1.0.0", &twice)];
        let errors = refused(&rom(), &packs, CheckOptions::default());
        assert!(
            matches!(
                errors.as_slice(),
                [PackError::DuplicateEntry { family: Family::Message, id: 400, .. }]
            ),
            "{errors:?}"
        );
    }

    #[test]
    fn a_messages_file_that_does_not_compile_is_refused_with_its_message_page_and_line() {
        let text =
            "[[message]]\nid = 5\nwindow = \"narration\"\npages = [[\"ok\"], [\"{name:9}\"]]\n";
        let packs = [messages_pack("example.room", "1.0.0", text)];
        let errors = refused(&rom(), &packs, CheckOptions::default());
        match errors.as_slice() {
            [
                PackError::Message {
                    file,
                    error: MessageError::Token { id: 5, page: 2, line: 1, .. },
                    ..
                },
            ] => {
                assert_eq!(file, "text/messages.toml");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn chapters_are_checked_as_a_set_a_map_must_be_in_it_and_an_id_is_unique() {
        let chapter = |id: &str, map: u8| {
            format!(
                "[[chapter]]\nid = \"{id}\"\nname = \"x\"\nmap = {map}\nleader = [1, 1]\nfollower = [1, 1]\nparty = [{{ character = 0, level = 1 }}]\n"
            )
        };
        let with_chapters = |name: &str, text: String| {
            let mut found = pack(name, "1.0.0", &[]);
            found
                .manifest
                .chapters
                .push(crate::manifest::ChaptersEntry { file: "chapters.toml".into() });
            found.files.insert("chapters.toml".into(), text.into_bytes());
            found
        };
        let a = with_chapters("a.pack", chapter("androids", 64));
        let b = with_chapters("b.pack", chapter("androids", 0));
        let errors = refused(&rom(), &[a, b], CheckOptions::default());
        assert!(
            errors.iter().any(|e| matches!(
                e,
                PackError::Chapter { pack, error: chapter::ChapterError::MissingMap { id, map: 64 }, .. }
                    if pack == "a.pack" && id == "androids"
            )),
            "{errors:?}"
        );
        assert!(
            errors.iter().any(|e| matches!(
                e,
                PackError::Chapter { pack, file, error: chapter::ChapterError::Duplicate { .. } }
                    if pack == "b.pack" && file == "chapters.toml"
            )),
            "{errors:?}"
        );
    }
}
