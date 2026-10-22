//! A pack map: a field map of the game on its own id (`64` to `127`), built from the original field art. The
//! original's tile set and metatile table stay as they are, so a pack map is its block grid, its block definitions
//! (new ones are made of existing metatiles), its objects (16 at most), its encounter zones, its music and palette
//! (borrowed from an original map) and its start (`docs/re/data/maps.md`, sections 1 to 3 and 6 to 8).
//!
//! [`PackMap::compile`] writes the four families of a map (see [`crate::Family`]): the header (`MapHeader`, 8 bytes), the
//! grid and the new block definitions (`MapBlocks`), the objects (`MapObjects`) and the zones (`MapZones`). Each is
//! read by a redirector of `ssd-game` through the directory of the image; nothing in it is a pointer.

use std::fmt;

/// The first and the last id of a pack map: the original's header table has 64 words (`06:F2DE`), and a block
/// id of 7 bits of the original's arithmetic reaches 127.
pub const FIRST_ID: u8 = 64;
pub const LAST_ID: u8 = 127;
/// The first block id a pack map defines (`$78`: the original defines `$00` to `$77`, 120 blocks).
pub const NEW_BLOCK_BASE: u8 = 0x78;
/// The new block definitions a map may hold: ids `$78` to `$FF`.
pub const MAX_BLOCKS: usize = 0x100 - NEW_BLOCK_BASE as usize;
/// The bytes of a block definition: 256 cells of one word each.
pub const BLOCK_LEN: usize = 512;
/// The objects of a map: the object loader keeps 16 slots (`$1180`, 7 bytes each).
pub const MAX_OBJECTS: usize = 16;
/// The object records of the original (`02:EB67`, 72 records of 5 bytes): an object's type comes from its record.
pub const OBJECT_RECORDS: u8 = 72;
/// The encounter groups of the original (112 groups of 5 units).
pub const GROUPS: u8 = 112;
/// The most zones a map has: the zone's number is stored as a byte (1-based).
pub const MAX_ZONES: usize = 255;
/// The sides of a map in blocks: a block coordinate is a byte of the camera (`$34`, `$36`).
pub const MAX_SIDE: u32 = 128;
/// The original's music and palette maps: the ids `0` to `47`.
pub const LIKE_LIMIT: u8 = 48;
/// The metatiles the table addresses (its masked cell word reaches `$3FDE`).
pub const METATILES: u16 = 4096;
/// The bits of a cell word the game keeps (`$3FDE`): a cell's metatile is `row * $40 + column * 2` in the table.
pub const CELL_BITS: u16 = 0x3FDE;

/// A pack map as the packer reads it (see the module documentation for each field).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackMap {
    /// The map's id, `64` to `127`.
    pub id: u8,
    /// The width and height in blocks of 256 x 256 pixels, each `1` to `128`.
    pub width: u8,
    pub height: u8,
    /// The block id of each block, row by row (`width * height` bytes).
    pub grid: Vec<u8>,
    /// The new block definitions: block `NEW_BLOCK_BASE + k` is `blocks[k]`.
    pub blocks: Vec<BlockDef>,
    /// The original map whose music byte the pack map plays (`02:E41B`).
    pub music_like: u8,
    /// The original map whose palette indexes and palette sets the pack map uses (`02:8484`, `06:F876`).
    pub palette_like: u8,
    /// The start position for the mode 6 init (`00:933A`), in pixels.
    pub start: (u16, u16),
    /// The objects (NPCs), at most 16.
    pub objects: Vec<MapObject>,
    /// The encounter zones, in the order the game tests them (the first that holds the camera wins).
    pub zones: Vec<Zone>,
}

/// One block definition: 256 cell words, row by row (cell `row * 16 + column` is the 16 x 16 pixels at that place).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDef {
    pub cells: Vec<u16>,
}

impl BlockDef {
    /// A block whose cells are the metatiles `numbers` (256 numbers, row by row). A metatile number `n` is the
    /// metatile at row `n / 16`, column `n % 16` of the game's metatile table, so its cell word is
    /// `(n / 16) * $40 + (n % 16) * 2`.
    ///
    /// # Errors
    ///
    /// [`MapError::BlockCells`] if there are not 256 numbers, [`MapError::Metatile`] for a number of 4096 or more.
    pub fn from_metatiles(numbers: &[u16]) -> Result<Self, MapError> {
        if numbers.len() != 256 {
            return Err(MapError::BlockCells { count: numbers.len() });
        }
        let mut cells = Vec::with_capacity(256);
        for (cell, &number) in numbers.iter().enumerate() {
            if number >= METATILES {
                return Err(MapError::Metatile { cell, number });
            }
            cells.push((number / 16) * 0x40 + (number % 16) * 2);
        }
        Ok(Self { cells })
    }
}

/// An object (an NPC): the original's object record `object` (its type comes from the record), at a position in
/// pixels, and the message it speaks (a message id of the original).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapObject {
    pub object: u8,
    pub x: u16,
    pub y: u16,
    pub message: u16,
}

/// An encounter zone: the rectangle of blocks (inclusive) the camera's block must be in, the encounter level, and
/// the encounter group of the battle (an original group, `0` to `111`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zone {
    pub x0: u8,
    pub x1: u8,
    pub y0: u8,
    pub y1: u8,
    pub level: u8,
    pub group: u8,
}

/// The bytes of a compiled map, one per family (see [`PackMap::compile`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapData {
    /// `MapHeader`: width, height, `music_like`, `palette_like`, the start x and y (`u16` each, low byte first).
    pub header: Vec<u8>,
    /// `MapBlocks`: the grid, padded with zeros to a multiple of 256 bytes, then the block definitions. A
    /// definition starts at a multiple of 256 from the start of the family, which the image builder aligns.
    pub blocks: Vec<u8>,
    /// `MapObjects`: the count, then 7 bytes an object: the object (`u8`), x and y (`u16` each), the message (`u16`).
    pub objects: Vec<u8>,
    /// `MapZones`: the count, then 6 bytes a zone: x0, x1, y0, y1, level, group.
    pub zones: Vec<u8>,
}

impl PackMap {
    /// The bytes of the map's four families.
    ///
    /// # Errors
    ///
    /// Whatever [`PackMap::check`] refuses.
    pub fn compile(&self) -> Result<MapData, MapError> {
        self.check()?;
        let (width, height) = (usize::from(self.width), usize::from(self.height));
        let header = vec![
            self.width,
            self.height,
            self.music_like,
            self.palette_like,
            self.start.0 as u8,
            (self.start.0 >> 8) as u8,
            self.start.1 as u8,
            (self.start.1 >> 8) as u8,
        ];
        let mut blocks = self.grid.clone();
        blocks.resize(round_up(width * height, 256), 0);
        for block in &self.blocks {
            for cell in &block.cells {
                blocks.extend_from_slice(&cell.to_le_bytes());
            }
        }
        let mut objects = vec![self.objects.len() as u8];
        for object in &self.objects {
            objects.push(object.object);
            objects.extend_from_slice(&object.x.to_le_bytes());
            objects.extend_from_slice(&object.y.to_le_bytes());
            objects.extend_from_slice(&object.message.to_le_bytes());
        }
        let mut zones = vec![self.zones.len() as u8];
        for zone in &self.zones {
            zones.extend_from_slice(&[zone.x0, zone.x1, zone.y0, zone.y1, zone.level, zone.group]);
        }
        Ok(MapData { header, blocks, objects, zones })
    }

    /// Checks every field against its limit: the id, the sides, the grid (each block id defined), the block
    /// definitions (their cells, whose bits must be the ones the game keeps), the music and palette maps, the
    /// start, the objects and the zones (inside the map).
    ///
    /// # Errors
    ///
    /// The first [`MapError`] found, naming the value.
    pub fn check(&self) -> Result<(), MapError> {
        if !(FIRST_ID..=LAST_ID).contains(&self.id) {
            return Err(MapError::Id { id: self.id });
        }
        let (width, height) = (u32::from(self.width), u32::from(self.height));
        if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
            return Err(MapError::Size { width, height });
        }
        let cells = usize::from(self.width) * usize::from(self.height);
        if self.grid.len() != cells {
            return Err(MapError::GridLength { expected: cells, found: self.grid.len() });
        }
        if self.blocks.len() > MAX_BLOCKS {
            return Err(MapError::TooManyBlocks { count: self.blocks.len() });
        }
        for (index, &block) in self.grid.iter().enumerate() {
            let defined =
                block < NEW_BLOCK_BASE || usize::from(block - NEW_BLOCK_BASE) < self.blocks.len();
            if !defined {
                return Err(MapError::GridBlock { cell: index, block, blocks: self.blocks.len() });
            }
        }
        for (index, block) in self.blocks.iter().enumerate() {
            if block.cells.len() != 256 {
                return Err(MapError::BlockCells { count: block.cells.len() });
            }
            for (cell, &word) in block.cells.iter().enumerate() {
                if word & !CELL_BITS != 0 {
                    return Err(MapError::CellBits { block: index, cell, word });
                }
            }
        }
        for (name, value) in [("music_like", self.music_like), ("palette_like", self.palette_like)]
        {
            if value >= LIKE_LIMIT {
                return Err(MapError::Like { name, value });
            }
        }
        let (width_px, height_px) = (width * 256, height * 256);
        if u32::from(self.start.0) >= width_px || u32::from(self.start.1) >= height_px {
            return Err(MapError::Start {
                x: self.start.0,
                y: self.start.1,
                width: width_px,
                height: height_px,
            });
        }
        if self.objects.len() > MAX_OBJECTS {
            return Err(MapError::TooManyObjects { count: self.objects.len() });
        }
        for (index, object) in self.objects.iter().enumerate() {
            if object.object >= OBJECT_RECORDS {
                return Err(MapError::ObjectKind { index, object: object.object });
            }
            if u32::from(object.x) >= width_px || u32::from(object.y) >= height_px {
                return Err(MapError::Object { index, x: object.x, y: object.y });
            }
        }
        if self.zones.len() > MAX_ZONES {
            return Err(MapError::TooManyZones { count: self.zones.len() });
        }
        for (index, zone) in self.zones.iter().enumerate() {
            let inside = zone.x0 <= zone.x1
                && zone.y0 <= zone.y1
                && u32::from(zone.x1) < width
                && u32::from(zone.y1) < height;
            if !inside {
                return Err(MapError::Zone { index, zone: *zone, width, height });
            }
            if zone.group >= GROUPS {
                return Err(MapError::ZoneGroup { index, group: zone.group });
            }
        }
        Ok(())
    }
}

/// `value` rounded up to a multiple of `unit`.
fn round_up(value: usize, unit: usize) -> usize {
    value.div_ceil(unit) * unit
}

/// Why a pack map is refused. Each names the value and its limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    /// The id is not `64` to `127`.
    Id { id: u8 },
    /// The width or the height is not 1 to 128 blocks.
    Size { width: u32, height: u32 },
    /// The grid does not hold width times height block ids.
    GridLength { expected: usize, found: usize },
    /// A grid cell names a block that is neither original (`$00`-`$77`) nor defined in the map.
    GridBlock { cell: usize, block: u8, blocks: usize },
    /// More new block definitions than the ids `$78` to `$FF` hold.
    TooManyBlocks { count: usize },
    /// A block definition does not have 256 cells (or a metatile list does not have 256 numbers).
    BlockCells { count: usize },
    /// A cell word has bits the game does not keep (`$3FDE`): the cell is not a metatile's word.
    CellBits { block: usize, cell: usize, word: u16 },
    /// A metatile number is 4096 or more.
    Metatile { cell: usize, number: u16 },
    /// `music_like` or `palette_like` is 48 or more.
    Like { name: &'static str, value: u8 },
    /// The start is outside the map (in pixels, the sides are `width` and `height`).
    Start { x: u16, y: u16, width: u32, height: u32 },
    /// More objects than the object loader's 16 slots.
    TooManyObjects { count: usize },
    /// An object names a record of the original that is not there (`0` to `71`).
    ObjectKind { index: usize, object: u8 },
    /// An object is outside the map.
    Object { index: usize, x: u16, y: u16 },
    /// More zones than a map may have (255).
    TooManyZones { count: usize },
    /// A zone's rectangle is not inside the map (blocks, inclusive).
    Zone { index: usize, zone: Zone, width: u32, height: u32 },
    /// A zone's group is not an original group (`0` to `111`).
    ZoneGroup { index: usize, group: u8 },
    /// A Tiled map file that does not parse or does not describe a pack map (the message names the file's part).
    Tiled(String),
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id { id } => write!(f, "map id {id} is not in {FIRST_ID} to {LAST_ID}"),
            Self::Size { width, height } => {
                write!(f, "the map is {width} x {height} blocks: each side is 1 to {MAX_SIDE}")
            }
            Self::GridLength { expected, found } => {
                write!(f, "the block grid holds {found} blocks, the map's size needs {expected}")
            }
            Self::GridBlock { cell, block, blocks } => write!(
                f,
                "the block {block} at grid cell {cell} is neither an original block (0 to {}) nor one of the map's {blocks} new blocks",
                NEW_BLOCK_BASE - 1
            ),
            Self::TooManyBlocks { count } => write!(
                f,
                "{count} new block definitions: at most {MAX_BLOCKS} (block ids {NEW_BLOCK_BASE} to 255)"
            ),
            Self::BlockCells { count } => {
                write!(f, "a block has {count} cells or metatile numbers: it must have 256")
            }
            Self::CellBits { block, cell, word } => write!(
                f,
                "cell {cell} of new block {} is {word:#06X}: only the bits of {CELL_BITS:#06X} are a metatile's",
                NEW_BLOCK_BASE as usize + block
            ),
            Self::Metatile { cell, number } => {
                write!(f, "metatile {number} at cell {cell} is past the {METATILES} metatiles")
            }
            Self::Like { name, value } => {
                write!(f, "{name} is {value}: it names an original map, 0 to {}", LIKE_LIMIT - 1)
            }
            Self::Start { x, y, width, height } => {
                write!(f, "the start ({x}, {y}) is outside the map of {width} x {height} pixels")
            }
            Self::TooManyObjects { count } => {
                write!(f, "{count} objects: the object loader has {MAX_OBJECTS} slots")
            }
            Self::ObjectKind { index, object } => write!(
                f,
                "object {index} names the object record {object}: the records are 0 to {}",
                OBJECT_RECORDS - 1
            ),
            Self::Object { index, x, y } => {
                write!(f, "object {index} at ({x}, {y}) is outside the map")
            }
            Self::TooManyZones { count } => write!(f, "{count} zones: at most {MAX_ZONES}"),
            Self::Zone { index, zone, width, height } => write!(
                f,
                "zone {index} (blocks x {}..={}, y {}..={}) is not inside the {width} x {height} blocks of the map",
                zone.x0, zone.x1, zone.y0, zone.y1
            ),
            Self::ZoneGroup { index, group } => {
                write!(
                    f,
                    "zone {index} names the group {group}: the groups are 0 to {}",
                    GROUPS - 1
                )
            }
            Self::Tiled(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for MapError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 x 2 map of original block 1 and 2 with one new block (`$78`) made of the metatile 17 everywhere.
    fn two_by_two() -> PackMap {
        PackMap {
            id: 64,
            width: 2,
            height: 2,
            grid: vec![1, 2, 0x78, 2],
            blocks: vec![BlockDef::from_metatiles(&[17; 256]).unwrap()],
            music_like: 34,
            palette_like: 34,
            start: (256, 128),
            objects: vec![MapObject { object: 5, x: 300, y: 200, message: 0x122 }],
            zones: vec![Zone { x0: 0, x1: 1, y0: 0, y1: 1, level: 7, group: 3 }],
        }
    }

    #[test]
    fn a_two_by_two_map_compiles_to_the_bytes_of_its_header_grid_and_records() {
        let data = two_by_two().compile().unwrap();
        // The header: width, height, music, palette, the start x (256 = $0100) and y (128 = $0080), low byte first.
        assert_eq!(data.header, [2, 2, 34, 34, 0x00, 0x01, 0x80, 0x00]);
        // The grid is four bytes, padded to 256; the new block follows at offset 256: its cells are metatile 17
        // (row 1, column 1), so the word is $40 + $02 = $42, low byte first.
        assert_eq!(&data.blocks[..4], [1, 2, 0x78, 2]);
        assert_eq!(data.blocks.len(), 256 + 512);
        assert!(data.blocks[4..256].iter().all(|&byte| byte == 0));
        assert_eq!(&data.blocks[256..258], [0x42, 0x00]);
        assert_eq!(data.blocks[256 + 510..], [0x42, 0x00]);
        // The objects: the count, then object 5, x 300 ($012C), y 200 ($00C8), message $0122.
        assert_eq!(data.objects, [1, 5, 0x2C, 0x01, 0xC8, 0x00, 0x22, 0x01]);
        // The zones: the count, then x0 x1 y0 y1 level group.
        assert_eq!(data.zones, [1, 0, 1, 0, 1, 7, 3]);
    }

    #[test]
    fn a_metatile_number_is_a_row_and_a_column_of_the_table() {
        let numbers: Vec<u16> = (0..256).map(|k| [0_u16, 1, 15, 16, 4095][k % 5]).collect();
        let cells = BlockDef::from_metatiles(&numbers).unwrap().cells;
        assert_eq!(cells[0], 0);
        assert_eq!(cells[1], 2, "column 1 of row 0");
        assert_eq!(cells[2], 0x1E, "column 15 of row 0");
        assert_eq!(cells[3], 0x40, "column 0 of row 1");
        assert_eq!(cells[4], 0x3FDE, "metatile 4095 (row 255, column 15) is the whole mask");
        assert!(cells.iter().all(|&word| word & !CELL_BITS == 0));
    }

    #[test]
    fn each_refusal_names_its_value() {
        let mut map = two_by_two();
        map.id = 63;
        assert_eq!(map.check(), Err(MapError::Id { id: 63 }));
        let mut map = two_by_two();
        map.grid = vec![1, 2, 0x79, 2];
        assert!(matches!(map.check(), Err(MapError::GridBlock { cell: 2, block: 0x79, .. })));
        let mut map = two_by_two();
        map.blocks[0].cells[3] = 0x0001;
        assert!(matches!(map.check(), Err(MapError::CellBits { block: 0, cell: 3, .. })));
        let mut map = two_by_two();
        map.music_like = 48;
        assert!(matches!(map.check(), Err(MapError::Like { name: "music_like", value: 48 })));
        let mut map = two_by_two();
        map.start = (512, 0);
        assert!(matches!(map.check(), Err(MapError::Start { .. })));
        let mut map = two_by_two();
        map.zones[0].x1 = 2;
        assert!(matches!(map.check(), Err(MapError::Zone { index: 0, .. })));
        let mut map = two_by_two();
        map.zones[0].group = 112;
        assert!(matches!(map.check(), Err(MapError::ZoneGroup { index: 0, group: 112 })));
        let mut map = two_by_two();
        map.objects = vec![map.objects[0]; 17];
        assert_eq!(map.check(), Err(MapError::TooManyObjects { count: 17 }));
        assert!(BlockDef::from_metatiles(&[0; 255]).is_err());
        assert!(matches!(
            BlockDef::from_metatiles(&[4096; 256]),
            Err(MapError::Metatile { cell: 0, number: 4096 })
        ));
    }
}
