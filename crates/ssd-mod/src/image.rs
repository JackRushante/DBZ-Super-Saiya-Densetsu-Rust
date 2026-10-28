//! The image a set of packs runs on: the 1 MiB ROM, then the pack data in the `LoROM` banks
//! above the cartridge's own, and the directory that tells the game where each slot's data lies.
//!
//! The image is built in memory and never written to disk. A set without data is the ROM padded
//! with zeros to 2 MiB (the empty set), and it has no directory.

use crate::check::MAX_FILE_LEN;
use crate::derive;
use crate::family::{DIRECTORY_MAGIC, DIRECTORY_VERSION};
use crate::manifest::Version;
use crate::scene::relocate;
use crate::text::{self, Window};
use crate::trigger;
use crate::{Family, Pack, PackError};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// The cartridge's ROM: 1 048 576 bytes.
pub const ROM_LEN: usize = 0x10_0000;
/// One `LoROM` bank's window of the image: `$8000-$FFFF`.
const BANK_LEN: usize = 0x8000;
/// The first bank of the extension (`$20`).
const FIRST_BANK: u8 = 0x20;
/// The last bank of a 2 MiB image (`$3F`) and of a 4 MiB one (`$7D`: `$7E` and `$7F` are work RAM).
const LAST_BANK_2_MIB: u8 = 0x3F;
const LAST_BANK_4_MIB: u8 = 0x7D;
/// The directory at `$20:8000`, 4 096 bytes; the data starts after it, at `$20:9000`.
pub(crate) const DIRECTORY_BUS: u32 = 0x20_8000;
pub(crate) const DIRECTORY_LEN: usize = 0x1000;
/// The directory's header (`SSDP`, the version, the family count, two zero bytes), a family's
/// descriptor, and an entry (`u16` id, `u24` address, a zero byte).
pub(crate) const HEADER_LEN: usize = 8;
pub(crate) const DESCRIPTOR_LEN: usize = 8;
pub(crate) const ENTRY_LEN: usize = 6;

/// An image built from a set of packs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// The ROM followed by the extension, padded with zeros; 2 MiB or 4 MiB, a power of two.
    pub bytes: Vec<u8>,
    /// CRC32 of the whole image.
    pub crc32: u32,
    /// The set's id: FNV-1a 64 of the packs' identities and data (see [`build`]).
    pub set_id: u64,
    /// Where each slot's data lies, sorted by family and id.
    pub placements: Vec<Placement>,
    /// The HD tiles of the pack maps (`[[map_art]]`), in the order of the packs; for the Enhanced picture only.
    pub map_art: Vec<MapArt>,
    /// The faces of the packs (`[[face]]`), in the order of the packs: the SNES art the image holds for each.
    pub faces: Vec<FaceArt>,
    /// The pose variants of the packs (`[[pose_variant]]`), in the order of the packs; for the Enhanced picture only.
    pub pose_variants: Vec<PoseArt>,
}

/// A pose variant of a pack (its `[[pose_variant]]` entry): the picture the Enhanced picture draws in place of the art of the
/// object `art` while `when` holds (see [`crate::pose`]). Nothing of it is in the image: the picture is read from the pack's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoseArt {
    /// The pack the variant is in.
    pub pack: String,
    /// The name of the object's art it replaces.
    pub art: String,
    /// The condition under which it is drawn.
    pub when: crate::pose::Condition,
    /// The margin, in art pixels on each side of the object's picture.
    pub margin: u32,
    /// The picture, relative to the pack.
    pub file: String,
}

/// The HD tiles of a pack map (its `[[map_art]]` entry), with the layout the image gives the map: its size in blocks and the
/// bus address of its grid. The Enhanced picture finds its art by that layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapArt {
    /// The pack the entry is in.
    pub pack: String,
    /// The pack map's id.
    pub map: u8,
    /// The size of the map in blocks of 256 x 256 pixels.
    pub blocks: (u8, u8),
    /// The bus address of the map's grid (`$BB:AAAA`).
    pub grid: u32,
    /// The tiles' file name, relative to the pack, with `{x}` and `{y}` (see [`crate::MapArtEntry::tile_file`]).
    pub tiles: String,
}

/// A face of a pack (its `[[face]]` entry): the HD picture and the SNES art derived from it (see [`crate::derive`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceArt {
    /// The pack the face is in.
    pub pack: String,
    /// The face index.
    pub id: u16,
    /// The HD picture, relative to the pack.
    pub picture: String,
    /// The 32 tiles of the face's record in the SNES layout: the 16 tiles of the face, in both pictures of the record.
    pub tiles: Vec<[u8; 32]>,
    /// The palette row the face was derived in (the row the game draws it in).
    pub palette: [u16; 16],
    /// The record the image holds for the face: the LZ stream the game's portrait routine decodes.
    pub record: Vec<u8>,
}

/// Where one slot's data lies in the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub family: Family,
    pub id: u16,
    /// The 24-bit bus address of the first byte (`$BB:AAAA`, in banks `$20` and up).
    pub address: u32,
    /// The length of the data in bytes.
    pub len: u32,
}

/// One slot's data, as the layout sees it: the bytes, and for a scene script the offsets of its label
/// words, which [`build`] relocates to the address the script is placed at.
struct Item<'a> {
    family: Family,
    id: u16,
    data: Cow<'a, [u8]>,
    relocs: Vec<usize>,
    /// The alignment of the data in the bank: 1, or 256 where a block definition must start a multiple of 256 (the game reads its
    /// address as a bank and a middle byte).
    align: usize,
}

/// The image of `packs` on `rom`. The packs must have passed [`crate::check_set`].
///
/// The data is sorted by size, largest first (ties: family, then id), and placed by first fit into
/// the banks in order; a file never crosses a bank. A scene script is relocated to the low 16 bits of
/// its address. The image is 2 MiB when everything fits the banks `$20-$3F`, else 4 MiB (banks up to
/// `$7D`). A face's record is derived from its HD picture in the palette row of the ROM that the game
/// draws the faces in ([`derive::face_palette`]). The set id is the FNV-1a 64 of the text
/// `"{id}@{version}:{crc32 of the pack's data in [[data]], [[scene]], [[messages]], [[map]], [[triggers]], [[tables]],
/// [[face]] and [[feature]] order};"` for each pack, sorted by id.
///
/// # Errors
///
/// [`PackError::RomSize`] if `rom` is not 1 MiB, [`PackError::Scene`] if a scene does not assemble,
/// [`PackError::DoesNotFit`] if the data does not fit the banks or the directory, [`PackError::Art`] if a
/// face's picture does not derive.
pub fn build(rom: &[u8], packs: &[Pack]) -> Result<Image, PackError> {
    if rom.len() != ROM_LEN {
        return Err(PackError::RomSize { len: rom.len() });
    }
    let faces = faces(rom, packs)?;
    let items = items(rom, packs, &faces)?;
    let (len, addresses) = layout(&items)?;
    let mut bytes = padded(rom, len);
    for (item, &address) in items.iter().zip(&addresses) {
        let at = file_offset(address);
        let end = at + item.data.len();
        bytes[at..end].copy_from_slice(&item.data);
        if !item.relocs.is_empty() {
            relocate(&mut bytes[at..end], &item.relocs, address as u16);
        }
    }
    if !items.is_empty() {
        write_directory(&mut bytes, &items, &addresses);
    }
    let mut placements: Vec<Placement> = items
        .iter()
        .zip(&addresses)
        .map(|(item, &address)| Placement {
            family: item.family,
            id: item.id,
            address,
            len: item.data.len() as u32,
        })
        .collect();
    placements.sort_by_key(|placement| (placement.family, placement.id));
    let map_art = map_arts(packs, &items, &addresses);
    Ok(Image {
        crc32: ssd_rom::crc32(&bytes),
        bytes,
        set_id: set_id(packs),
        placements,
        map_art,
        faces,
        pose_variants: pose_variants(packs),
    })
}

/// The pose variants of `packs`, in the order of the packs: each `[[pose_variant]]` entry with its condition. An entry whose
/// condition is not known is left out (the pack is refused at load).
fn pose_variants(packs: &[Pack]) -> Vec<PoseArt> {
    packs
        .iter()
        .flat_map(|pack| {
            pack.manifest.pose_variant.iter().filter_map(|entry| {
                Some(PoseArt {
                    pack: pack.manifest.pack.id.clone(),
                    art: entry.art.clone(),
                    when: crate::pose::Condition::from_name(&entry.when)?,
                    margin: entry.margin,
                    file: entry.file.clone(),
                })
            })
        })
        .collect()
}

/// The faces of `packs`: each `[[face]]` picture read and derived in the palette row of `rom` that the game draws the faces
/// in, and its record made. A face whose file the pack does not hold is left out (the checks refuse it).
///
/// # Errors
///
/// [`PackError::Art`], naming the pack and the picture, if a picture is not a PNG or not 128 x 128 pixels.
pub(crate) fn faces(rom: &[u8], packs: &[Pack]) -> Result<Vec<FaceArt>, PackError> {
    let mut faces = Vec::new();
    for pack in packs {
        for entry in &pack.manifest.face {
            let Some(bytes) = pack.files.get(&entry.hd) else { continue };
            let pack_id = pack.manifest.pack.id.clone();
            let refused = |detail: String| PackError::Art {
                pack: pack_id.clone(),
                file: entry.hd.clone(),
                detail,
            };
            let row = derive::face_palette(rom)
                .ok_or_else(|| refused("the ROM has no palette row for the faces".to_owned()))?;
            let picture = derive::read_png(bytes).map_err(|e| refused(e.to_string()))?;
            let face = derive::derive_face(&picture, row).map_err(|e| refused(e.to_string()))?;
            let record = derive::face_record(&face.tiles)
                .ok_or_else(|| refused("the face's record is empty".to_owned()))?;
            faces.push(FaceArt {
                pack: pack_id.clone(),
                id: u16::from(entry.id),
                picture: entry.hd.clone(),
                tiles: derive::record_tiles(&face.tiles),
                palette: face.palette,
                record,
            });
        }
    }
    Ok(faces)
}

/// The HD tiles of the pack maps of `packs`: for each `[[map_art]]` entry, the size and the grid address of its map (the
/// header and grid slots of the image). An entry whose map has no slots is left out (the checks refuse it).
fn map_arts(packs: &[Pack], items: &[Item<'_>], addresses: &[u32]) -> Vec<MapArt> {
    let slot = |family: Family, id: u16| {
        items
            .iter()
            .zip(addresses)
            .find(|(item, _)| item.family == family && item.id == id)
            .map(|(item, &address)| (item, address))
    };
    let mut arts = Vec::new();
    for pack in packs {
        for entry in &pack.manifest.map_art {
            let id = u16::from(entry.map);
            let header = slot(Family::MapHeader, id);
            let grid = slot(Family::MapBlocks, id);
            let (Some((header, _)), Some((_, grid))) = (header, grid) else { continue };
            let (Some(&width), Some(&height)) = (header.data.first(), header.data.get(1)) else {
                continue;
            };
            arts.push(MapArt {
                pack: pack.manifest.pack.id.clone(),
                map: entry.map,
                blocks: (width, height),
                grid,
                tiles: entry.tiles.clone(),
            });
        }
    }
    arts
}

/// `rom` followed by zeros, `len` bytes in all.
#[must_use]
pub fn padded(rom: &[u8], len: usize) -> Vec<u8> {
    let mut bytes = rom.to_vec();
    bytes.resize(len, 0);
    bytes
}

/// Whether the data of `packs` fits the banks and the directory (see [`build`]). A scene that does not
/// assemble, or a face that does not derive, is not counted here: [`crate::check_set`] refuses it, with its line.
pub(crate) fn fits(rom: &[u8], packs: &[Pack]) -> Result<(), PackError> {
    let Ok(faces) = faces(rom, packs) else { return Ok(()) };
    match items(rom, packs, &faces) {
        Ok(items) => layout(&items).map(|_| ()),
        Err(error @ PackError::DoesNotFit { .. }) => Err(error),
        Err(_) => Ok(()),
    }
}

/// The data of `packs` that the layout places: the slots whose files are in the packs and are not too
/// large (the other cases are refusals of [`crate::check_set`]), the scene scripts, assembled, the messages,
/// compiled, with their strings numbered (see [`text_items`]), the maps, the triggers, and the records of `faces`.
///
/// # Errors
///
/// [`PackError::Scene`] for a scene that does not assemble, [`PackError::Message`] for a message that does not
/// compile, [`PackError::DoesNotFit`] for a string index past [`text::MAX_INDEX`].
fn items<'a>(
    rom: &[u8],
    packs: &'a [Pack],
    faces: &'a [FaceArt],
) -> Result<Vec<Item<'a>>, PackError> {
    let mut items: Vec<Item<'_>> = Vec::new();
    for pack in packs {
        for entry in &pack.manifest.data {
            let Some(family) = Family::from_name(&entry.family) else { continue };
            if family.is_text() {
                continue;
            }
            let Some(data) = pack.files.get(&entry.file) else { continue };
            if data.len() <= MAX_FILE_LEN {
                items.push(Item {
                    family,
                    id: entry.id,
                    data: Cow::Borrowed(data),
                    relocs: Vec::new(),
                    align: 1,
                });
            }
        }
        for entry in &pack.manifest.scene {
            let Some(script) = pack.scene(entry)? else { continue };
            if script.bytes.len() <= MAX_FILE_LEN {
                items.push(Item {
                    family: Family::Script,
                    id: entry.id,
                    data: Cow::Owned(script.bytes),
                    relocs: script.relocs,
                    align: 1,
                });
            }
        }
        for entry in &pack.manifest.tables {
            let Some(slots) = pack.tables(entry)? else { continue };
            for mut slot in slots {
                if slot.family == Family::CommandAppend {
                    slot.bytes = crate::tables::appended_command_list(rom, slot.id, &slot.bytes).ok_or_else(|| {
                        PackError::Manifest {
                            path: pack.manifest.pack.id.clone(),
                            message: format!(
                                "unit {}: its original command list cannot be read, or the joined list is too long",
                                slot.id
                            ),
                        }
                    })?;
                    slot.family = Family::CommandList;
                }
                items.push(Item {
                    family: slot.family,
                    id: slot.id,
                    data: Cow::Owned(slot.bytes),
                    relocs: Vec::new(),
                    align: 1,
                });
            }
        }
        for slot in pack.features()? {
            items.push(Item {
                family: slot.family,
                id: slot.id,
                data: Cow::Owned(slot.bytes),
                relocs: Vec::new(),
                align: 1,
            });
        }
        for entry in &pack.manifest.map {
            let Some(map) = pack.map(entry)? else { continue };
            let refused = |error| PackError::Map {
                pack: pack.manifest.pack.id.clone(),
                file: entry.file.clone(),
                error,
            };
            let data = map.compile().map_err(refused)?;
            if data.blocks.len() > MAX_FILE_LEN {
                return Err(PackError::DoesNotFit {
                    detail: format!(
                        "map {} needs {} bytes of grid and block definitions: a bank holds 32768",
                        map.id,
                        data.blocks.len()
                    ),
                });
            }
            let id = u16::from(map.id);
            items.push(Item {
                family: Family::MapHeader,
                id,
                data: Cow::Owned(data.header),
                relocs: Vec::new(),
                align: 1,
            });
            items.push(Item {
                family: Family::MapBlocks,
                id,
                data: Cow::Owned(data.blocks),
                relocs: Vec::new(),
                align: 256,
            });
            items.push(Item {
                family: Family::MapObjects,
                id,
                data: Cow::Owned(data.objects),
                relocs: Vec::new(),
                align: 1,
            });
            items.push(Item {
                family: Family::MapZones,
                id,
                data: Cow::Owned(data.zones),
                relocs: Vec::new(),
                align: 1,
            });
        }
    }
    items.extend(trigger_item(packs)?);
    items.extend(text_items(packs)?);
    for face in faces {
        items.push(Item {
            family: Family::Face,
            id: face.id,
            data: Cow::Borrowed(&face.record),
            relocs: Vec::new(),
            align: 1,
        });
    }
    items.sort_by(|a, b| {
        b.data.len().cmp(&a.data.len()).then(a.family.cmp(&b.family)).then(a.id.cmp(&b.id))
    });
    Ok(items)
}

/// The item of the triggers of `packs`: the triggers of every pack, in the order of the packs' ids (so the numbers of a set do
/// not depend on the order it is loaded in), compiled by [`trigger::encode`] (sorted by map id). It is slot 0 of
/// `Family::Triggers`; a set without triggers has no item.
///
/// # Errors
///
/// [`PackError::DoesNotFit`] for more triggers than a set holds, or an item larger than a bank.
fn trigger_item(packs: &[Pack]) -> Result<Option<Item<'static>>, PackError> {
    let mut by_id: Vec<&Pack> = packs.iter().collect();
    by_id.sort_by(|a, b| a.manifest.pack.id.cmp(&b.manifest.pack.id));
    let mut triggers = Vec::new();
    for pack in by_id {
        triggers.extend(pack.compiled_triggers()?);
    }
    if triggers.is_empty() {
        return Ok(None);
    }
    let data = trigger::encode(&triggers)
        .map_err(|error| PackError::DoesNotFit { detail: error.to_string() })?;
    if data.len() > MAX_FILE_LEN {
        return Err(PackError::DoesNotFit {
            detail: format!("the triggers need {} bytes: a bank holds 32768", data.len()),
        });
    }
    Ok(Some(Item {
        family: Family::Triggers,
        id: 0,
        data: Cow::Owned(data),
        relocs: Vec::new(),
        align: 1,
    }))
}

/// The items of the messages of `packs`: each message's list (family `message`, its id) and its string (the
/// family of its table, the index). The packs are taken in the order of their ids, so the indices of a set do
/// not depend on the order it is loaded in; within a pack, the messages take the indices in the order the file
/// gives them, and each table's first index is the original's count (see [`text::FIRST_DIALOGUE`]).
///
/// # Errors
///
/// As [`items`]: the messages that do not compile, and [`PackError::DoesNotFit`] for an index past the table's
/// reach ([`text::MAX_INDEX`]).
fn text_items(packs: &[Pack]) -> Result<Vec<Item<'static>>, PackError> {
    let mut by_id: Vec<&Pack> = packs.iter().collect();
    by_id.sort_by(|a, b| a.manifest.pack.id.cmp(&b.manifest.pack.id));
    let mut next = [Window::Dialogue.first_index(), Window::Narration.first_index()];
    let mut items = Vec::new();
    for pack in by_id {
        for message in pack.compiled_messages()? {
            let slot = match message.window {
                Window::Dialogue => 0,
                Window::Narration => 1,
            };
            let index = next[slot];
            if index > text::MAX_INDEX {
                return Err(PackError::DoesNotFit {
                    detail: format!(
                        "the strings of table {} run past index ${:04X}",
                        message.window.table(),
                        text::MAX_INDEX
                    ),
                });
            }
            next[slot] += 1;
            let family =
                if message.window == Window::Dialogue { Family::String2 } else { Family::String3 };
            items.push(Item {
                family: Family::Message,
                id: message.id,
                data: Cow::Owned(message.list(index).to_vec()),
                relocs: Vec::new(),
                align: 1,
            });
            items.push(Item {
                family,
                id: index,
                data: Cow::Owned(message.string),
                relocs: Vec::new(),
                align: 1,
            });
        }
    }
    Ok(items)
}

/// The image length and the address of each item (in `items` order).
fn layout(items: &[Item<'_>]) -> Result<(usize, Vec<u32>), PackError> {
    if items.is_empty() {
        return Ok((0x20_0000, Vec::new()));
    }
    let families: std::collections::BTreeSet<Family> =
        items.iter().map(|item| item.family).collect();
    let directory = HEADER_LEN + DESCRIPTOR_LEN * families.len() + ENTRY_LEN * items.len();
    if directory > DIRECTORY_LEN {
        return Err(PackError::DoesNotFit {
            detail: format!(
                "the directory of {} entries needs {directory} bytes, the directory holds \
                 {DIRECTORY_LEN}",
                items.len()
            ),
        });
    }
    if let Some(addresses) = place(items, LAST_BANK_2_MIB) {
        return Ok((0x20_0000, addresses));
    }
    if let Some(addresses) = place(items, LAST_BANK_4_MIB) {
        return Ok((0x40_0000, addresses));
    }
    Err(PackError::DoesNotFit {
        detail: format!(
            "the data needs more than the banks ${FIRST_BANK:02X}-${LAST_BANK_4_MIB:02X} of a 4 MiB \
             image hold"
        ),
    })
}

/// First fit of `items` (sorted, largest first) into the banks `$20` to `last`, each bank's window
/// `$8000-$FFFF`, bank `$20` after the directory. `None` when one does not fit.
fn place(items: &[Item<'_>], last: u8) -> Option<Vec<u32>> {
    let banks = usize::from(last - FIRST_BANK) + 1;
    let mut used = vec![0_usize; banks];
    used[0] = DIRECTORY_LEN;
    let mut addresses = Vec::with_capacity(items.len());
    for item in items {
        let len = item.data.len();
        let aligned = |used: usize| used.div_ceil(item.align) * item.align;
        let bank = (0..banks).find(|&bank| aligned(used[bank]) + len <= BANK_LEN)?;
        let start = aligned(used[bank]);
        let bus = (u32::from(FIRST_BANK) + bank as u32) << 16 | (0x8000 + start) as u32;
        addresses.push(bus);
        used[bank] = start + len;
    }
    Some(addresses)
}

/// The directory (see [`crate::family`]): the header, one descriptor per family present, then one
/// entry array per family, sorted by id.
fn write_directory(bytes: &mut [u8], items: &[Item<'_>], addresses: &[u32]) {
    let mut families: BTreeMap<Family, Vec<(u16, u32)>> = BTreeMap::new();
    for (item, &address) in items.iter().zip(addresses) {
        families.entry(item.family).or_default().push((item.id, address));
    }
    let base = file_offset(DIRECTORY_BUS);
    bytes[base..base + DIRECTORY_LEN].fill(0);
    bytes[base..base + 4].copy_from_slice(DIRECTORY_MAGIC);
    bytes[base + 4] = DIRECTORY_VERSION;
    bytes[base + 5] = families.len() as u8;
    let mut array = DIRECTORY_BUS + (HEADER_LEN + DESCRIPTOR_LEN * families.len()) as u32;
    for (index, (family, mut entries)) in families.into_iter().enumerate() {
        entries.sort_by_key(|&(id, _)| id);
        let at = base + HEADER_LEN + DESCRIPTOR_LEN * index;
        bytes[at] = family as u8;
        bytes[at + 2..at + 4].copy_from_slice(&(entries.len() as u16).to_le_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&array.to_le_bytes());
        for (k, &(id, address)) in entries.iter().enumerate() {
            let entry = file_offset(array) + k * ENTRY_LEN;
            bytes[entry..entry + 2].copy_from_slice(&id.to_le_bytes());
            bytes[entry + 2..entry + 5].copy_from_slice(&address.to_le_bytes()[..3]);
            bytes[entry + 5] = 0;
        }
        array += (entries.len() * ENTRY_LEN) as u32;
    }
}

/// The offset in the image of a `LoROM` bus address (`$BB:8000-$FFFF`).
fn file_offset(bus: u32) -> usize {
    (bus >> 16) as usize * BANK_LEN + ((bus & 0xFFFF) as usize - 0x8000)
}

/// The set id (see [`build`]).
fn set_id(packs: &[Pack]) -> u64 {
    let mut sorted: Vec<&Pack> = packs.iter().collect();
    sorted.sort_by(|a, b| a.manifest.pack.id.cmp(&b.manifest.pack.id));
    let mut text = String::new();
    for pack in sorted {
        let mut data = Vec::new();
        let files = pack
            .manifest
            .data
            .iter()
            .map(|entry| &entry.file)
            .chain(pack.manifest.scene.iter().map(|entry| &entry.file))
            .chain(pack.manifest.messages.iter().map(|entry| &entry.file))
            .chain(pack.manifest.map.iter().map(|entry| &entry.file))
            .chain(pack.manifest.triggers.iter().map(|entry| &entry.file))
            .chain(pack.manifest.tables.iter().map(|entry| &entry.file))
            .chain(pack.manifest.face.iter().map(|entry| &entry.hd));
        for file in files {
            if let Some(bytes) = pack.files.get(file) {
                data.extend_from_slice(bytes);
            }
        }
        // A feature is no file: its name and parameters are the pack's data too (another parameter is another image).
        for entry in &pack.manifest.feature {
            data.extend_from_slice(format!("{}{:?};", entry.name, entry.params).as_bytes());
        }
        let version = Version::parse(&pack.manifest.pack.version)
            .map_or_else(|| pack.manifest.pack.version.clone(), |version| version.to_string());
        let _ = write!(text, "{}@{version}:{:08X};", pack.manifest.pack.id, ssd_rom::crc32(&data));
    }
    text.bytes().fold(0xCBF2_9CE4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{map_pack, messages_pack, pack, rom, triggers_pack};

    /// The bytes at a bus address of the image (`$BB:AAAA`, `AAAA` at least `$8000`).
    fn at(bytes: &[u8], bus: u32, len: usize) -> &[u8] {
        let start = file_offset(bus);
        &bytes[start..start + len]
    }

    fn sample_packs() -> Vec<Pack> {
        vec![
            pack("alpha.pack", "1.0.0", &[("script", 1, &[1; 5000]), ("face", 9, &[2; 300])]),
            pack("beta.pack", "2.1.0", &[("script", 7, &[3; 20_000]), ("script", 8, &[4; 9000])]),
        ]
    }

    #[test]
    fn placement_never_crosses_a_bank_and_is_deterministic() {
        let rom = rom();
        let first = build(&rom, &sample_packs()).unwrap();
        let second = build(&rom, &sample_packs()).unwrap();
        assert_eq!(first, second, "the same packs give the same image");
        assert_eq!(first.placements.len(), 4);
        for placement in &first.placements {
            let bank = placement.address >> 16;
            let start = placement.address & 0xFFFF;
            assert!((u32::from(FIRST_BANK)..=0x7D).contains(&bank), "bank {bank:02X}");
            assert!(start >= 0x8000, "data is in the bank's LoROM window");
            assert!(start + placement.len <= 0x1_0000, "{placement:?} crosses its bank");
        }
        let script7 = first.placements.iter().find(|p| p.id == 7).unwrap();
        assert_eq!(at(&first.bytes, script7.address, 20_000), vec![3_u8; 20_000].as_slice());
        let face9 = first.placements.iter().find(|p| p.id == 9).unwrap();
        assert_eq!(at(&first.bytes, face9.address, 300), vec![2_u8; 300].as_slice());
    }

    #[test]
    fn the_image_is_the_rom_then_the_directory_and_the_data() {
        let rom = rom();
        let image = build(&rom, &sample_packs()).unwrap();
        assert_eq!(&image.bytes[..ROM_LEN], &rom[..], "the ROM first");
        assert_eq!(at(&image.bytes, DIRECTORY_BUS, 4), b"SSDP", "the directory at $20:8000");
        assert_eq!(image.crc32, ssd_rom::crc32(&image.bytes));
        assert_eq!(image.bytes.len(), 0x20_0000);
    }

    #[test]
    fn two_mebibytes_when_the_data_fits_banks_20_to_3f_and_four_when_it_needs_more() {
        let rom = rom();
        // Bank $20 keeps the directory (0x1000 bytes), so 31 files of 32 KiB fit banks $20 to $3F
        // only in $21-$3F (31 banks); a 32nd does not, and goes to bank $40.
        let fits: Vec<Pack> = vec![pack(
            "two.mib",
            "1.0.0",
            &(0..31).map(|s| ("script", s as u16, &[9_u8; 0x8000][..])).collect::<Vec<_>>(),
        )];
        let image = build(&rom, &fits).unwrap();
        assert_eq!(image.bytes.len(), 0x20_0000, "two MiB");
        assert!(image.placements.iter().all(|p| (p.address >> 16) <= 0x3F));
        let more: Vec<Pack> = vec![pack(
            "four.mib",
            "1.0.0",
            &(0..32).map(|s| ("script", s as u16, &[9_u8; 0x8000][..])).collect::<Vec<_>>(),
        )];
        let image = build(&rom, &more).unwrap();
        assert_eq!(image.bytes.len(), 0x40_0000, "four MiB");
        assert!(image.placements.iter().any(|p| (p.address >> 16) >= 0x40));
    }

    #[test]
    fn the_empty_set_is_the_rom_padded_to_two_mebibytes_without_a_directory() {
        let rom = rom();
        let image = build(&rom, &[]).unwrap();
        assert_eq!(image.bytes, padded(&rom, 0x20_0000));
        assert_eq!(image.placements, []);
        assert_eq!(
            image.set_id, 0xCBF2_9CE4_8422_2325,
            "FNV-1a of the empty text is its offset basis"
        );
        assert_eq!(&at(&image.bytes, DIRECTORY_BUS, 4), &[0_u8; 4], "no directory");
    }

    #[test]
    fn the_set_id_changes_with_a_version_and_with_a_data_byte() {
        let rom = rom();
        let base = build(&rom, &sample_packs()).unwrap().set_id;
        assert_eq!(build(&rom, &sample_packs()).unwrap().set_id, base, "stable");
        let mut bumped = sample_packs();
        bumped[1].manifest.pack.version = "2.1.1".into();
        assert_ne!(build(&rom, &bumped).unwrap().set_id, base, "a version");
        let mut changed = sample_packs();
        changed[0].files.get_mut("data/face-9.bin").unwrap()[17] ^= 1;
        assert_ne!(build(&rom, &changed).unwrap().set_id, base, "one data byte");
    }

    #[test]
    fn a_scene_whose_jump_lands_on_its_own_label_holds_the_relocated_address() {
        let rom = rom();
        let text = "scene loop_once:\ntop:\n    wait $02\n    jump top\n";
        let packs = vec![crate::testing::scene_pack("scene.pack", "1.0.0", &[(82, text)])];
        let image = build(&rom, &packs).unwrap();
        let placement = image
            .placements
            .iter()
            .find(|p| p.family == Family::Script && p.id == 82)
            .expect("the scene is placed");
        assert!(placement.address >> 16 >= 0x20, "a pack bank");
        let script = at(&image.bytes, placement.address, placement.len as usize);
        let low = (placement.address & 0xFFFF) as u16;
        assert_eq!(
            &script[..5],
            &[0x10, 0x02, 0x12, low as u8, (low >> 8) as u8],
            "the jump holds the script's own address"
        );
        let broken =
            crate::testing::scene_pack("scene.pack", "1.0.0", &[(82, "scene s:\n    jmp 1\n")]);
        assert!(matches!(build(&rom, &[broken]), Err(PackError::Scene { .. })));
    }

    #[test]
    fn a_feature_is_its_parameters_in_the_image_and_its_parameters_change_the_set_id() {
        use crate::feature::SSJ_KAIOKEN;
        let rom = rom();
        let text = "[[feature]]\nname = \"ssj_kaioken\"\nmax_factor = 10\nhp_cost_percent = 10\n";
        let pack_with = |text: &str| {
            let mut features = pack("kaioken.pack", "1.0.0", &[]);
            features.manifest = toml::from_str(&format!(
                "[pack]\nid = \"kaioken.pack\"\nname = \"K\"\nversion = \"1.0.0\"\ngame = \"{}\"\n{text}",
                crate::testing::game()
            ))
            .unwrap();
            features
        };
        let image = build(&rom, &[pack_with(text)]).unwrap();
        let slot = image
            .placements
            .iter()
            .find(|placement| placement.family == Family::Feature && placement.id == SSJ_KAIOKEN)
            .expect("the feature's slot is placed");
        assert_eq!(slot.len, 2, "two bytes: max_factor and hp_cost_percent");
        assert_eq!(at(&image.bytes, slot.address, 2), [10, 10], "the parameters, one byte each");
        let other =
            build(&rom, &[pack_with(&text.replace("max_factor = 10", "max_factor = 9"))]).unwrap();
        assert_ne!(image.set_id, other.set_id, "another parameter is another set");
        let plain = build(&rom, &[pack("plain.pack", "1.0.0", &[])]).unwrap();
        assert!(
            plain.placements.iter().all(|placement| placement.family != Family::Feature),
            "a pack without features places none"
        );
    }

    #[test]
    fn a_rom_that_is_not_one_mebibyte_is_refused_by_build() {
        let short = &rom()[..ROM_LEN - 2];
        assert_eq!(build(short, &[]).unwrap_err(), PackError::RomSize { len: ROM_LEN - 2 });
    }

    /// A messages file of one message in `window`, one line `line`, one page.
    #[test]
    fn a_tables_file_places_its_unit_row_name_and_command_list_at_their_addresses() {
        let rom = rom();
        let text = "[[unit]]
id = 105
name = \"Guard\"
class = 1
level = 5
hp = 300
bp = 100000
ki = 50
                    defence = 30
tie = 7
exp = 1234
sprite_like = 4
[[unit.command]]
id = 0
attr = 16
factor = 4
";
        let mut tables = pack("tables.pack", "1.0.0", &[]);
        tables.manifest.tables.push(crate::TablesEntry { file: "tables.toml".into() });
        tables.files.insert("tables.toml".into(), text.as_bytes().to_vec());
        let image = build(&rom, &[tables]).unwrap();
        let place = |family: Family| {
            image.placements.iter().find(|p| p.family == family && p.id == 105).unwrap()
        };
        let row = place(Family::Unit);
        assert_eq!(row.len, 15, "the 14 bytes of the row and the sprite");
        assert_eq!(
            at(&image.bytes, row.address, 15),
            [1, 5, 0x2C, 0x01, 0xA0, 0x86, 0x01, 0x32, 0x00, 30, 7, 0xD2, 0x04, 0x00, 4]
        );
        let commands = place(Family::CommandList);
        assert_eq!(at(&image.bytes, commands.address, 5), [1, 0, 16, 4, 0], "one slot");
        let name = place(Family::UnitName);
        assert_eq!(at(&image.bytes, name.address, 6)[0], 5, "a length of five tiles");
    }

    fn message(id: u16, window: &str, line: &str) -> String {
        format!("[[message]]\nid = {id}\nwindow = \"{window}\"\npages = [[\"{line}\"]]\n")
    }

    #[test]
    fn a_message_takes_its_list_and_its_string_at_the_index_the_set_gives() {
        let rom = rom();
        // The packs are numbered in the order of their ids: alpha's dialogue (402) comes first in table 2.
        let packs = vec![
            messages_pack(
                "beta.pack",
                "1.0.0",
                &format!("{}{}", message(400, "dialogue", "ok."), message(401, "narration", "ok.")),
            ),
            messages_pack("alpha.pack", "1.0.0", &message(402, "dialogue", "ok.")),
        ];
        let image = build(&rom, &packs).unwrap();
        let place = |family: Family, id: u16| {
            *image
                .placements
                .iter()
                .find(|p| p.family == family && p.id == id)
                .unwrap_or_else(|| panic!("{family:?} {id} is placed"))
        };
        // `ok.` is `o`, `k`, `.` and the two FF of the string's end.
        let string = [0x1B, 0x17, 0x7C, 0xFF, 0xFF];
        assert_eq!(
            at(&image.bytes, place(Family::Message, 402).address, 6).to_vec(),
            [0x02, 0xD0, 0x6E, 0x01, 0xFF, 0xFF]
        );
        assert_eq!(at(&image.bytes, place(Family::String2, 366).address, 5).to_vec(), string);
        assert_eq!(
            at(&image.bytes, place(Family::Message, 400).address, 6).to_vec(),
            [0x02, 0xD0, 0x6F, 0x01, 0xFF, 0xFF]
        );
        assert_eq!(at(&image.bytes, place(Family::String2, 367).address, 5).to_vec(), string);
        assert_eq!(
            at(&image.bytes, place(Family::Message, 401).address, 6).to_vec(),
            [0x03, 0xD0, 0xA7, 0x00, 0xFF, 0xFF]
        );
        assert_eq!(at(&image.bytes, place(Family::String3, 167).address, 5).to_vec(), string);
    }

    #[test]
    fn the_set_id_changes_with_the_text_of_a_message() {
        let rom = rom();
        let base = build(
            &rom,
            &[messages_pack("example.room", "1.0.0", &message(400, "dialogue", "ok."))],
        )
        .unwrap()
        .set_id;
        let changed = build(
            &rom,
            &[messages_pack("example.room", "1.0.0", &message(400, "dialogue", "no."))],
        )
        .unwrap()
        .set_id;
        assert_ne!(base, changed, "a messages file is data of the set");
    }

    #[test]
    fn a_message_that_does_not_compile_stops_the_build_with_its_message() {
        let rom = rom();
        let bad = messages_pack("example.room", "1.0.0", &message(400, "dialogue", "a;b"));
        assert!(matches!(
            build(&rom, &[bad]),
            Err(PackError::Message {
                error: crate::text::MessageError::Character { id: 400, .. },
                ..
            })
        ));
    }

    /// A Tiled map of two cells: the original block 1, then the new block (`$78`) of metatile 1 everywhere.
    fn tiny_map() -> String {
        let ones = vec!["1"; 256].join(",");
        r#"{ "type": "map", "orientation": "orthogonal", "width": 2, "height": 1, "tilewidth": 256, "tileheight": 256,
  "properties": [ { "name": "music_like", "type": "int", "value": 34 }, { "name": "palette_like", "type": "int", "value": 34 } ],
  "tilesets": [ { "firstgid": 1, "name": "blocks", "tilecount": 120 },
    { "firstgid": 121, "name": "new", "tilecount": 1, "tiles": [ { "id": 0, "properties": [ { "name": "metatiles", "type": "string", "value": "ONES" } ] } ] } ],
  "layers": [ { "type": "tilelayer", "name": "blocks", "width": 2, "height": 1, "data": [1, 121] },
    { "type": "objectgroup", "name": "objects", "objects": [] },
    { "type": "objectgroup", "name": "zones", "objects": [] },
    { "type": "objectgroup", "name": "start", "objects": [ { "name": "start", "x": 256, "y": 128 } ] } ] }"#
            .replace("ONES", &ones)
    }

    #[test]
    fn a_pack_map_is_four_families_and_its_block_definitions_are_aligned() {
        let rom = rom();
        let image = build(&rom, &[map_pack("example.room", "1.0.0", 64, &tiny_map())]).unwrap();
        let place = |family: Family| {
            *image
                .placements
                .iter()
                .find(|p| p.family == family && p.id == 64)
                .unwrap_or_else(|| panic!("{family:?} 64 is placed"))
        };
        let header = place(Family::MapHeader);
        assert_eq!(at(&image.bytes, header.address, 8), [2, 1, 34, 34, 0, 1, 128, 0]);
        let blocks = place(Family::MapBlocks);
        assert_eq!(blocks.address & 0xFF, 0, "the block definitions start a multiple of 256");
        assert_eq!(blocks.len, 256 + 512, "the grid padded to 256, one definition");
        assert_eq!(
            at(&image.bytes, blocks.address, 2),
            [0, 0x78],
            "the grid: block 0, then the new block"
        );
        assert_eq!(at(&image.bytes, blocks.address + 256, 2), [0x02, 0x00], "metatile 1 is word 2");
        assert_eq!(place(Family::MapObjects).len, 1, "a count of zero objects");
        assert_eq!(place(Family::MapZones).len, 1, "a count of zero zones");
    }

    #[test]
    fn the_set_id_changes_with_a_map_file() {
        let rom = rom();
        let base =
            build(&rom, &[map_pack("example.room", "1.0.0", 64, &tiny_map())]).unwrap().set_id;
        let changed = tiny_map().replace("\"x\": 256", "\"x\": 257");
        let other = build(&rom, &[map_pack("example.room", "1.0.0", 64, &changed)]).unwrap().set_id;
        assert_ne!(base, other, "a map file is data of the set");
    }

    #[test]
    fn the_triggers_of_a_set_are_one_item_in_slot_0_sorted_by_map_and_the_set_id_follows_them() {
        let rom = rom();
        let one = triggers_pack(
            "a.room",
            "1.0.0",
            "[[trigger]]\nmap = 70\nrect = [0, 0, 8, 8]\nscene = 3\n",
        );
        let two = triggers_pack(
            "b.room",
            "1.0.0",
            "[[trigger]]\nmap = 64\nrect = [8, 8, 16, 16]\nwhen = [\"flag 3\"]\nscene = 82\n\
             [[trigger]]\nmap = 70\nrect = [1, 1, 1, 1]\nscene = 4\n",
        );
        let image = build(&rom, &[two.clone(), one]).unwrap();
        let placed: Vec<&Placement> =
            image.placements.iter().filter(|p| p.family == Family::Triggers).collect();
        assert_eq!(placed.len(), 1, "one item for the set");
        assert_eq!(placed[0].id, 0);
        let bytes = at(&image.bytes, placed[0].address, placed[0].len as usize);
        let triggers = crate::trigger::decode(bytes).expect("a whole item");
        let order: Vec<(u8, u8)> = triggers.iter().map(|t| (t.map, t.scene)).collect();
        assert_eq!(order, [(64, 82), (70, 3), (70, 4)], "sorted by map; packs by id");
        let other = triggers_pack(
            "a.room",
            "1.0.0",
            "[[trigger]]\nmap = 70\nrect = [0, 0, 8, 8]\nscene = 5\n",
        );
        let changed = build(&rom, &[two, other]).unwrap();
        assert_ne!(image.set_id, changed.set_id, "the set id follows a triggers file");
        let none = build(&rom, &[pack("c.room", "1.0.0", &[])]).unwrap();
        assert!(
            none.placements.iter().all(|p| p.family != Family::Triggers),
            "no triggers, no item"
        );
    }

    #[test]
    fn a_face_is_derived_at_the_row_of_the_rom_and_its_record_is_placed() {
        let rom = crate::testing::rom_with_face_row();
        let hd = crate::testing::png(128, 128, &crate::testing::flat(128, 128, [200, 30, 30]));
        let packs = [crate::testing::face_pack("face.pack", "1.0.0", 3, "art/hd/faces/a.png", hd)];
        let image = build(&rom, &packs).unwrap();
        assert_eq!(image.faces.len(), 1);
        let face = &image.faces[0];
        assert_eq!(
            (face.pack.as_str(), face.id, face.picture.as_str()),
            ("face.pack", 3, "art/hd/faces/a.png")
        );
        assert_eq!(face.palette[1], 0x1234, "the row of the ROM the game draws the faces in");
        assert_eq!(face.tiles.len(), 32, "the 16 tiles in both pictures of the record");
        let placed =
            image.placements.iter().find(|p| p.family == Family::Face && p.id == 3).unwrap();
        assert_eq!(at(&image.bytes, placed.address, placed.len as usize), face.record.as_slice());
        assert_eq!(&face.record[..2], &[0, 4], "the record decodes to 1024 bytes, 32 tiles");
    }

    #[test]
    fn a_map_art_entry_is_listed_with_the_size_and_the_grid_of_its_map() {
        let rom = rom();
        let mut pack = map_pack("example.room", "1.0.0", 64, &tiny_map());
        pack.manifest
            .map_art
            .push(crate::MapArtEntry { map: 64, tiles: "art/hd/maps/64/t{x}_{y}.png".into() });
        let image = build(&rom, &[pack]).unwrap();
        let grid = image
            .placements
            .iter()
            .find(|p| p.family == Family::MapBlocks && p.id == 64)
            .unwrap()
            .address;
        assert_eq!(
            image.map_art,
            [MapArt {
                pack: "example.room".into(),
                map: 64,
                blocks: (2, 1),
                grid,
                tiles: "art/hd/maps/64/t{x}_{y}.png".into(),
            }]
        );
    }

    #[test]
    fn a_pose_variant_is_listed_with_its_condition_and_its_picture_is_not_needed_by_the_build() {
        let rom = rom();
        let mut pack = pack("example.room", "1.0.0", &[]);
        pack.manifest.pose_variant.push(crate::PoseVariantEntry {
            art: "battle-goku-ssj-000".into(),
            when: "kaioken".into(),
            file: "art/hd/variants/battle-goku-ssj-000.png".into(),
            margin: 24,
        });
        let packs = [pack];
        let options = crate::CheckOptions { allow_rom_bytes: true };
        assert!(crate::check_set(&rom, &packs, options).is_ok(), "a variant whose file is missing");
        let image = build(&rom, &packs).unwrap();
        assert_eq!(
            image.pose_variants,
            [PoseArt {
                pack: "example.room".into(),
                art: "battle-goku-ssj-000".into(),
                when: crate::pose::Condition::Kaioken,
                margin: 24,
                file: "art/hd/variants/battle-goku-ssj-000.png".into(),
            }]
        );
    }

    #[test]
    fn an_always_variant_is_listed_beside_the_kaioken_one_of_its_object() {
        let rom = rom();
        let mut pack = pack("example.room", "1.0.0", &[]);
        for when in ["always", "kaioken"] {
            pack.manifest.pose_variant.push(crate::PoseVariantEntry {
                art: "battle-goku-ssj-000".into(),
                when: when.into(),
                file: format!("art/hd/variants/{when}.png"),
                margin: 8,
            });
        }
        let packs = [pack];
        let options = crate::CheckOptions { allow_rom_bytes: true };
        assert!(crate::check_set(&rom, &packs, options).is_ok());
        let image = build(&rom, &packs).unwrap();
        let conditions: Vec<_> = image.pose_variants.iter().map(|variant| variant.when).collect();
        assert_eq!(conditions, [crate::pose::Condition::Always, crate::pose::Condition::Kaioken]);
    }

    #[test]
    fn a_map_that_is_not_valid_stops_the_build_with_its_part() {
        let rom = rom();
        let refused = build(&rom, &[map_pack("example.room", "1.0.0", 63, &tiny_map())]);
        assert!(matches!(
            refused,
            Err(PackError::Map { error: crate::map::MapError::Id { id: 63 }, .. })
        ));
    }
}
