//! The data families a pack can fill, and the directory format the game reads them by.
//!
//! The directory sits at `$20:8000` in the image: the magic `SSDP`, the version (1), the number of
//! families, two zero bytes; then one 8-byte descriptor per family present (the family, a zero, the
//! entry count as `u16`, and the bus address of its entry array as `u32`, top byte zero); then the
//! entry arrays, each entry 6 bytes (`u16` id, `u24` bus address, a zero byte), sorted by id.

/// The directory's magic, at `$20:8000`.
pub(crate) const DIRECTORY_MAGIC: &[u8; 4] = b"SSDP";
/// The directory's version.
pub(crate) const DIRECTORY_VERSION: u8 = 1;

/// A data family. The number is the byte the directory stores.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Family {
    /// A scene's script: the scene id selects the script the scene starter runs.
    Script = 1,
    /// A face portrait: the face index selects the picture the portrait routine draws.
    Face = 2,
    /// A message's list: the message number selects the list the message engine reads (`D0nn`, a
    /// string index, `FFFF`). Written by a pack's `[[messages]]` file, never by `[[data]]`.
    Message = 3,
    /// A string of table 2, the dialogue: the index selects the string the engine writes. Assigned
    /// by the image builder, never by `[[data]]`.
    String2 = 4,
    /// A string of table 3, the narration: as [`Family::String2`].
    String3 = 5,
    /// A pack map's header: its size, its music and palette maps and its start (`crate::map`). Written by `[[map]]`.
    MapHeader = 6,
    /// A pack map's grid of block ids, padded to 256 bytes, then its new block definitions. Written by `[[map]]`.
    MapBlocks = 7,
    /// A pack map's objects. Written by `[[map]]`.
    MapObjects = 8,
    /// A pack map's encounter zones. Written by `[[map]]`.
    MapZones = 9,
    /// The triggers of the whole set: one item in slot 0, compiled from every pack's `[[triggers]]` (`crate::trigger`).
    Triggers = 10,
    /// A unit's stats row (`units.md`, 14 bytes) and the unit whose sprite it borrows (one byte, `$FF` for none). Written by
    /// `[[tables]]` (`crate::tables`), never by `[[data]]`.
    Unit = 11,
    /// A unit's name: a length byte and its tiles. Written by `[[tables]]`.
    UnitName = 12,
    /// A unit's command list (`kaioken-bp.md`, section 3). Written by `[[tables]]`.
    CommandList = 13,
    /// An attack: its KI cost, the attack whose effect it uses, and its name. Written by `[[tables]]`.
    Attack = 14,
    /// An encounter group: five unit ids, `$FF` for an empty slot. Written by `[[tables]]`.
    Group = 15,
    /// A character's level rows: 100 rows of 11 bytes (`characters.md`). Written by `[[tables]]`.
    LevelRows = 16,
    /// A character's level cap, one byte. Written by `[[tables]]`.
    LevelCap = 17,
    /// A named feature's parameters, one byte each (`crate::feature`). Written by `[[feature]]`, never by `[[data]]`.
    Feature = 18,
    /// Commands to add to an original unit's command list (`commands_append` of `[[unit]]`): the image build reads the
    /// original list from the ROM and writes the joined list as the unit's [`Family::CommandList`]. Never in the directory.
    CommandAppend = 19,
}

impl Family {
    /// The name a manifest gives this family (`[[data]] family = "script"`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Face => "face",
            Self::Message => "message",
            Self::String2 => "string2",
            Self::String3 => "string3",
            Self::MapHeader => "map_header",
            Self::MapBlocks => "map_blocks",
            Self::MapObjects => "map_objects",
            Self::MapZones => "map_zones",
            Self::Triggers => "triggers",
            Self::Unit => "unit",
            Self::UnitName => "unit_name",
            Self::CommandList => "command_list",
            Self::Attack => "attack",
            Self::Group => "group",
            Self::LevelRows => "level_rows",
            Self::LevelCap => "level_cap",
            Self::Feature => "feature",
            Self::CommandAppend => "command_append",
        }
    }

    /// The family a manifest names, `None` for an unknown name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Script, Self::Face, Self::Message, Self::String2, Self::String3]
            .into_iter()
            .find(|family| family.name() == name)
    }

    /// Whether the family holds text, which only `[[messages]]` files fill.
    #[must_use]
    pub fn is_text(self) -> bool {
        matches!(self, Self::Message | Self::String2 | Self::String3)
    }
}

/// The bus address of the directory (`$20:8000`).
const DIRECTORY_BUS: u32 = crate::image::DIRECTORY_BUS;
/// The lengths of the directory's header, a family descriptor and an entry.
const HEADER_LEN: u32 = crate::image::HEADER_LEN as u32;
const DESCRIPTOR_LEN: u32 = crate::image::DESCRIPTOR_LEN as u32;
const ENTRY_LEN: u32 = crate::image::ENTRY_LEN as u32;

/// The bus address of the data of slot `id` of `family`, as the directory of the image that `read`
/// reads says it is (`read` is a bus reader: the game reads its own memory through it). `None` when
/// the image has no packs (no `SSDP` magic at `$20:8000`, or another version: a plain ROM mirrors
/// its bank `$00` there, a zero-padded image has zeros), or when the slot has no data.
///
/// The families are scanned for the one asked for; its entries are sorted by id, so the slot is
/// found by a binary search.
#[must_use]
pub fn find(read: impl Fn(u32) -> u8, family: Family, id: u16) -> Option<u32> {
    let at = |address: u32| read(address & 0xFF_FFFF);
    let word = |address: u32| u16::from_le_bytes([at(address), at(address.wrapping_add(1))]);
    let bus = |address: u32| {
        u32::from(at(address)) | u32::from(at(address + 1)) << 8 | u32::from(at(address + 2)) << 16
    };
    let magic = (0..4).all(|k| at(DIRECTORY_BUS + k) == DIRECTORY_MAGIC[k as usize]);
    if !magic || at(DIRECTORY_BUS + 4) != DIRECTORY_VERSION {
        return None;
    }
    let families = u32::from(at(DIRECTORY_BUS + 5));
    let descriptor = (0..families)
        .map(|index| DIRECTORY_BUS + HEADER_LEN + DESCRIPTOR_LEN * index)
        .find(|&descriptor| at(descriptor) == family as u8)?;
    let count = u32::from(word(descriptor + 2));
    let array = bus(descriptor + 4);
    let (mut low, mut high) = (0, count);
    while low < high {
        let middle = low + (high - low) / 2;
        let entry = array + ENTRY_LEN * middle;
        match word(entry).cmp(&id) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => return Some(bus(entry + 2)),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{pack, rom};
    use crate::{Pack, build, padded};

    /// A bus reader of `image` as the machine maps a `LoROM` cartridge of that size: `$8000-$FFFF` of
    /// every bank, mirrored to the image's length (the map of `Machine::peek`), zero elsewhere.
    fn lorom(image: &[u8]) -> impl Fn(u32) -> u8 + '_ {
        move |address| {
            if (address & 0xFFFF) < 0x8000 {
                return 0;
            }
            let offset = ((address & 0x7F_0000) >> 1) | (address & 0x7FFF);
            image[offset as usize & (image.len() - 1)]
        }
    }

    fn sample() -> Vec<Pack> {
        vec![
            pack("alpha.pack", "1.0.0", &[("script", 82, &[1; 300]), ("face", 3, &[2; 40])]),
            pack("beta.pack", "1.0.0", &[("script", 5, &[3; 70]), ("face", 0, &[4; 9])]),
        ]
    }

    #[test]
    fn find_reads_back_what_build_wrote_for_every_slot() {
        let rom = rom();
        let image = build(&rom, &sample()).unwrap();
        let read = lorom(&image.bytes);
        for placement in &image.placements {
            let found = find(&read, placement.family, placement.id);
            assert_eq!(found, Some(placement.address), "{placement:?}");
        }
        let script = find(&read, Family::Script, 82).unwrap();
        assert_eq!(read(script), 1, "the first byte of the data is there");
        assert_eq!(find(&read, Family::Script, 83), None, "a slot without data");
        assert_eq!(find(&read, Family::Face, 82), None, "the families are apart");
    }

    #[test]
    fn a_directory_with_many_slots_is_searched_by_id() {
        let rom = rom();
        let slots: Vec<(&str, u16, Vec<u8>)> =
            (0..40).map(|k| ("script", k * 7 + 1, vec![k as u8; 3])).collect();
        let entries: Vec<(&str, u16, &[u8])> =
            slots.iter().map(|(family, id, bytes)| (*family, *id, bytes.as_slice())).collect();
        let image = build(&rom, &[pack("many.pack", "1.0.0", &entries)]).unwrap();
        let read = lorom(&image.bytes);
        for (family, id, _) in &slots {
            assert!(find(&read, Family::from_name(family).unwrap(), *id).is_some(), "slot {id}");
        }
        assert_eq!(find(&read, Family::Script, 0), None, "below the first id");
        assert_eq!(find(&read, Family::Script, 2), None, "between two ids");
        assert_eq!(find(&read, Family::Script, 280), None, "above the last id");
    }

    #[test]
    fn a_plain_rom_and_a_zero_padded_image_have_no_packs() {
        let rom = rom();
        let plain = rom.clone();
        assert_eq!(find(lorom(&plain), Family::Script, 82), None, "the plain ROM");
        let padded = padded(&rom, 0x20_0000);
        assert_eq!(find(lorom(&padded), Family::Script, 82), None, "zero padding");
        let image = build(&rom, &sample()).unwrap();
        let mut other_version = image.bytes.clone();
        other_version[0x10_0000 + 4] = 2;
        assert_eq!(find(lorom(&other_version), Family::Script, 82), None, "another version");
        let mut no_magic = image.bytes.clone();
        no_magic[0x10_0000] = b'X';
        assert_eq!(find(lorom(&no_magic), Family::Script, 82), None, "no magic");
    }

    #[test]
    fn a_level_cap_and_its_rows_are_found_and_read_back_from_the_image() {
        use crate::TablesEntry;
        let rom = rom();
        let mut levels = pack("levels.pack", "1.0.0", &[]);
        levels.manifest.tables.push(TablesEntry { file: "tables.toml".into() });
        let text = "[[character]]
id = 0
cap = 48
[character.curve]
                    hp = { a = 80, b = 12, c = 1 }
ki = { a = 4, b = 0, c = 1 }
                    bp = { a = 0, b = 100, c = 1 }
exp = { a = -10, b = 10, c = 2 }
";
        levels.files.insert("tables.toml".into(), text.as_bytes().to_vec());
        let image = build(&rom, &[levels]).unwrap();
        let read = lorom(&image.bytes);
        let cap = find(&read, Family::LevelCap, 0).expect("the cap is in the directory");
        assert_eq!(read(cap), 48, "the cap byte");
        let rows = find(&read, Family::LevelRows, 0).expect("the rows are in the directory");
        assert_eq!(read(rows + 6), 92, "the HP byte of level 1 (80 + 12)");
        assert_eq!(
            read(rows + 11 * 47 + 6),
            0x90,
            "the HP byte of level 48 is 80 + 12 * 48 = 656 (0x90)"
        );
    }

    #[test]
    fn the_text_families_have_their_numbers_names_and_are_not_data() {
        assert_eq!(
            [Family::Message as u8, Family::String2 as u8, Family::String3 as u8],
            [3, 4, 5]
        );
        assert_eq!(Family::from_name("message"), Some(Family::Message));
        assert_eq!(Family::from_name("string2"), Some(Family::String2));
        assert_eq!(Family::from_name("string3"), Some(Family::String3));
        assert!(
            Family::Message.is_text() && Family::String2.is_text() && Family::String3.is_text()
        );
        assert!(!Family::Script.is_text() && !Family::Face.is_text());
    }
}
