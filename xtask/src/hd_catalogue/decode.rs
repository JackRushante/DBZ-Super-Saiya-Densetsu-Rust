//! Reads the battle poses and the portraits of the ROM, statically: every pose of every unit as the
//! sprites the game would draw it from, and the pictures of the faces and objects.
//!
//! The chain (docs/re/data/battle-poses.md has the routines): unit `u` has an entry of four bytes
//! in the unit table (`1E:CB06`): the 24-bit address of its 32-byte object palette and its
//! poseset. A poseset (`15:8060`, six bytes) names two tables: the graphics of each pose (one
//! compressed stream, or a list of streams, which decode back to back) and the layout of each pose
//! (a small bytecode that places 8 x 8 sprites). The sprites are drawn unflipped here; the game
//! flips them at draw time, which only changes the pose's mirror key.

use ssd_catalogue::RomView;
use ssd_catalogue::graphics::{lz_decode, regroup};

/// Units in the unit table.
pub const UNITS: u8 = 105;
/// Posesets in the poseset table.
pub const POSESETS: u8 = 33;

/// The unit table of the battle poses (palette address and poseset of each unit).
const UNIT_TABLE: u32 = 0x1E_CB06;
/// The poseset table.
const POSESET_TABLE: u32 = 0x15_8060;
/// The layout attribute that says every entry carries its own.
const PER_ENTRY: u8 = 0xFF;
/// The pieces of a cluster are at most this many pixels apart.
const CLUSTER_GAP: i32 = 12;
/// The fewest opaque pixels of a cluster that has a key of its own.
const CLUSTER_MIN_OPAQUE: usize = 40;

/// A ROM image with the reads this module needs.
#[derive(Clone, Copy)]
pub struct Rom<'a> {
    bytes: &'a [u8],
    view: RomView<'a>,
}

impl<'a> Rom<'a> {
    /// # Errors
    ///
    /// The image is not a whole-bank `LoROM` image.
    pub fn new(bytes: &'a [u8]) -> anyhow::Result<Self> {
        Ok(Self { bytes, view: RomView::new(bytes)? })
    }

    fn offset(self, bank: u8, addr: u16) -> Option<usize> {
        self.view.offset(u32::from(bank) << 16 | u32::from(addr))
    }

    fn byte(self, bank: u8, addr: u16) -> Option<u8> {
        self.bytes.get(self.offset(bank, addr)?).copied()
    }

    fn word(self, bank: u8, addr: u16) -> Option<u16> {
        Some(
            u16::from(self.byte(bank, addr)?)
                | u16::from(self.byte(bank, addr.checked_add(1)?)?) << 8,
        )
    }

    fn slice(self, address: u32, len: usize) -> Option<&'a [u8]> {
        let start = self.view.offset(address)?;
        self.bytes.get(start..start + len)
    }

    /// The decoded stream at `bank:addr`.
    fn stream(self, bank: u8, addr: u16) -> Option<Vec<u8>> {
        lz_decode(self.bytes, self.offset(bank, addr)?).map(|stream| stream.data)
    }

    /// The bytes of the ROM.
    #[must_use]
    pub fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    /// The view of the ROM.
    #[must_use]
    pub fn view(self) -> RomView<'a> {
        self.view
    }
}

/// A unit's row of the battle table: where its palette is and which poseset it draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitRow {
    /// `bank:address` of the 32-byte palette.
    pub palette: u32,
    pub poseset: u8,
}

/// The unit table's row of `unit`.
#[must_use]
pub fn unit_row(rom: Rom<'_>, unit: u8) -> Option<UnitRow> {
    let row = rom.slice(UNIT_TABLE + 4 * u32::from(unit), 4)?;
    Some(UnitRow {
        palette: u32::from(row[2]) << 16 | u32::from(row[0]) | u32::from(row[1]) << 8,
        poseset: row[3],
    })
}

/// The 16 colours of the palette of `row`, as `0BBBBBGGGGGRRRRR` words.
#[must_use]
pub fn unit_palette(rom: Rom<'_>, row: UnitRow) -> Option<[u16; 16]> {
    let bytes = rom.slice(row.palette, 32)?;
    Some(std::array::from_fn(|n| u16::from_le_bytes([bytes[2 * n], bytes[2 * n + 1]])))
}

/// The two tables of a poseset: the graphics pointers (`bank`, address) and the layout pointers.
#[derive(Debug, Clone, Copy)]
struct Poseset {
    graphics: (u8, u16),
    layouts: (u8, u16),
}

fn poseset(rom: Rom<'_>, k: u8) -> Option<Poseset> {
    let r = rom.slice(POSESET_TABLE + 6 * u32::from(k), 6)?;
    Some(Poseset {
        graphics: (r[2], u16::from(r[0]) | u16::from(r[1]) << 8),
        layouts: (r[5], u16::from(r[3]) | u16::from(r[4]) << 8),
    })
}

/// The number of poses of poseset `k`: the layout table runs until the first layout it points at.
#[must_use]
pub fn pose_count(rom: Rom<'_>, k: u8) -> usize {
    let Some(set) = poseset(rom, k) else { return 0 };
    let (bank, addr) = set.layouts;
    let mut lowest = 0xFFFF_u16;
    let mut n = 0_usize;
    while let Some(at) = u16::try_from(usize::from(addr) + 2 * n).ok().filter(|&at| at < lowest) {
        let Some(value) = rom.word(bank, at) else { break };
        lowest = lowest.min(value);
        n += 1;
    }
    n
}

/// The tile data of a pose: its streams decoded and regrouped to the SNES tile layout, back to
/// back (tile `n` is the bytes from `32 n`).
fn pose_tiles(rom: Rom<'_>, set: Poseset, pose: usize) -> Result<Vec<u8>, String> {
    let (bank, table) = set.graphics;
    let at = u16::try_from(usize::from(table) + 3 * pose).map_err(|_| "pose table address")?;
    let (entry, entry_bank) = (
        rom.word(bank, at).ok_or("entry word")?,
        rom.byte(bank, at.checked_add(2).ok_or("entry bank")?).ok_or("entry bank")?,
    );
    let first = rom.word(entry_bank, entry).ok_or("entry target")?;
    let mut addresses = Vec::new();
    if first < 0x8000 {
        if first != 0 {
            addresses.push(entry);
        }
    } else {
        let mut y = 0_u16;
        loop {
            let word = rom
                .word(entry_bank, entry.checked_add(y).ok_or("stream list")?)
                .ok_or("stream list")?;
            if word == 0 {
                break;
            }
            if word < 0x8000 {
                return Err("stream list holds a small word".into());
            }
            addresses.push(word);
            y += 2;
        }
    }
    let mut data = Vec::new();
    for address in addresses {
        let mut piece = rom.stream(entry_bank, address).ok_or("stream does not decode")?;
        regroup(&mut piece);
        data.extend(piece);
    }
    Ok(data)
}

/// One placed 8 x 8 sprite of a pose's layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    pub y: u8,
    pub tile: u8,
    pub attr: u8,
    pub x: u8,
}

/// The layout of a pose: its sprites in the order the game expands them.
fn layout(rom: Rom<'_>, set: Poseset, pose: usize) -> Result<Vec<Piece>, String> {
    let (bank, table) = set.layouts;
    let at = u16::try_from(usize::from(table) + 2 * pose).map_err(|_| "layout table address")?;
    let start = rom.word(bank, at).ok_or("layout pointer")?;
    let mut out = Vec::new();
    let mut attr = 0x30_u8;
    let mut i = 0_u16;
    let read =
        |i: u16| rom.byte(bank, start.wrapping_add(i)).ok_or_else(|| "layout byte".to_owned());
    loop {
        let code = read(i)?;
        match code {
            0x80 => break,
            0x81 => {
                attr = 0x70;
                i += 1;
            }
            0x83 => {
                attr = PER_ENTRY;
                i += 1;
            }
            0x84 => {
                attr = 0x30;
                i += 1;
            }
            0x82 => {
                let count = read(i + 1)?;
                i += 2;
                for tile in (0..=count).rev() {
                    let y = read(i)?;
                    i += 1;
                    let mut at = attr;
                    if attr == PER_ENTRY {
                        at = read(i)?;
                        i += 1;
                    }
                    let x = read(i)?;
                    i += 1;
                    out.push(Piece { y, tile, attr: at, x });
                }
            }
            _ => {
                let (y, tile) = (code, read(i + 1)?);
                i += 2;
                let mut at = attr;
                if attr == PER_ENTRY {
                    at = read(i)?;
                    i += 1;
                }
                let x = read(i)?;
                i += 1;
                out.push(Piece { y, tile, attr: at, x });
            }
        }
        if out.len() > 512 {
            return Err("layout does not end".into());
        }
    }
    Ok(out)
}

/// The 8 x 8 palette indexes of tile `n` of `data` (4 bits a pixel, SNES layout), or blank when the
/// data is short.
fn tile_pixels(data: &[u8], n: usize) -> ([u8; 64], bool) {
    let mut out = [0_u8; 64];
    let Some(tile) = data.get(32 * n..32 * n + 32) else { return (out, false) };
    for row in 0..8 {
        for col in 0..8 {
            let bit = 7 - col;
            let mut color = 0_u8;
            for plane in 0..4 {
                let byte = tile[row * 2 + (plane & 1) + if plane >= 2 { 16 } else { 0 }];
                color |= ((byte >> bit) & 1) << plane;
            }
            out[row * 8 + col] = color;
        }
    }
    (out, true)
}

/// A decoded sprite: its place (the layout's signed bytes) and pixels.
#[derive(Debug, Clone)]
pub struct Sprite {
    pub x: i32,
    pub y: i32,
    pub pixels: [u8; 64],
}

impl Sprite {
    fn opaque(&self) -> bool {
        self.pixels.iter().any(|&c| c != 0)
    }
}

/// A trimmed array of palette indexes (0 transparent) with its size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    pub w: usize,
    pub h: usize,
    pub arr: Vec<u8>,
}

impl Shape {
    /// The pose key and the key of the pose facing the other way.
    #[must_use]
    pub fn keys(&self) -> (u64, u64) {
        (
            ssd_hd::resolve::pose_key(&self.arr, self.w, self.h),
            ssd_hd::resolve::mirror_pose_key(&self.arr, self.w, self.h),
        )
    }

    /// The keys of the pose drawn upside down: its key and the key of its mirror image.
    #[must_use]
    pub fn keys_upside_down(&self) -> (u64, u64) {
        let rows: Vec<&[u8]> = self.arr.chunks(self.w).rev().collect();
        let flipped: Vec<u8> = rows.concat();
        Shape { w: self.w, h: self.h, arr: flipped }.keys()
    }

    /// The opaque pixels.
    #[must_use]
    pub fn opaque(&self) -> usize {
        self.arr.iter().filter(|&&c| c != 0).count()
    }
}

/// One pose of a poseset, decoded.
#[derive(Debug, Clone)]
pub struct Pose {
    /// Tile data short of a tile the layout names (the tile draws blank): the key is not the game's.
    pub short_tiles: bool,
    /// The whole pose, trimmed; `None` when it has no opaque pixel.
    pub shape: Option<Shape>,
    /// The keyed clusters when the pose falls apart (pieces more than 12 pixels away from the
    /// rest): the renderer keys each cluster of at least 40 opaque pixels as well as the whole.
    pub clusters: Vec<Shape>,
    /// The ROM address of the pose's layout bytecode.
    pub layout_at: u32,
}

/// Decodes pose `pose` of poseset `k`, unflipped.
///
/// # Errors
///
/// The pose's tables or streams do not decode (the reason).
pub fn decode_pose(rom: Rom<'_>, k: u8, pose: usize) -> Result<Pose, String> {
    let set = poseset(rom, k).ok_or("poseset table")?;
    let pieces = layout(rom, set, pose)?;
    let data = pose_tiles(rom, set, pose)?;
    let (bank, table) = set.layouts;
    let pointer = rom.word(bank, table.wrapping_add(2 * pose as u16)).ok_or("layout pointer")?;
    let mut short_tiles = false;
    let sprites: Vec<Sprite> = pieces
        .iter()
        .map(|piece| {
            let (mut pixels, ok) = tile_pixels(&data, usize::from(piece.tile));
            short_tiles |= !ok;
            let (flip_h, flip_v) = (piece.attr & 0x40 != 0, piece.attr & 0x80 != 0);
            if flip_h || flip_v {
                let mut flipped = [0_u8; 64];
                for r in 0..8 {
                    for c in 0..8 {
                        let (rr, cc) =
                            (if flip_v { 7 - r } else { r }, if flip_h { 7 - c } else { c });
                        flipped[rr * 8 + cc] = pixels[r * 8 + c];
                    }
                }
                pixels = flipped;
            }
            let signed = |v: u8| if v < 128 { i32::from(v) } else { i32::from(v) - 256 };
            Sprite { x: signed(piece.x), y: signed(piece.y), pixels }
        })
        .collect();
    let shape = assemble(&sprites);
    let clusters = {
        let live: Vec<&Sprite> = sprites.iter().filter(|s| s.opaque()).collect();
        let parts = components(&live, CLUSTER_GAP);
        if parts.len() > 1 {
            parts
                .iter()
                .filter_map(|part| {
                    let group: Vec<&Sprite> = part.iter().map(|&i| live[i]).collect();
                    assemble_refs(&group)
                })
                .filter(|shape| shape.opaque() >= CLUSTER_MIN_OPAQUE)
                .collect()
        } else {
            Vec::new()
        }
    };
    Ok(Pose { short_tiles, shape, clusters, layout_at: u32::from(bank) << 16 | u32::from(pointer) })
}

/// The pose's pixels as one trimmed array; the sprite listed first is drawn on top.
fn assemble(sprites: &[Sprite]) -> Option<Shape> {
    let refs: Vec<&Sprite> = sprites.iter().collect();
    assemble_refs(&refs)
}

fn assemble_refs(sprites: &[&Sprite]) -> Option<Shape> {
    let x0 = sprites.iter().map(|s| s.x).min()?;
    let y0 = sprites.iter().map(|s| s.y).min()?;
    let x1 = sprites.iter().map(|s| s.x + 8).max()?;
    let y1 = sprites.iter().map(|s| s.y + 8).max()?;
    let (gw, gh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut canvas = vec![0_u8; gw * gh];
    for sprite in sprites.iter().rev() {
        for r in 0..8_usize {
            for c in 0..8_usize {
                let color = sprite.pixels[r * 8 + c];
                if color != 0 {
                    canvas[((sprite.y - y0) as usize + r) * gw + (sprite.x - x0) as usize + c] =
                        color;
                }
            }
        }
    }
    let (mut top, mut bottom, mut left, mut right) = (gh, 0, gw, 0);
    for y in 0..gh {
        for x in 0..gw {
            if canvas[y * gw + x] != 0 {
                top = top.min(y);
                bottom = bottom.max(y + 1);
                left = left.min(x);
                right = right.max(x + 1);
            }
        }
    }
    if bottom == 0 {
        return None;
    }
    let (w, h) = (right - left, bottom - top);
    let mut arr = Vec::with_capacity(w * h);
    for y in top..bottom {
        arr.extend_from_slice(&canvas[y * gw + left..y * gw + right]);
    }
    Some(Shape { w, h, arr })
}

/// Clusters of sprites whose boxes are within `gap` pixels of each other (the renderer's rule).
fn components(sprites: &[&Sprite], gap: i32) -> Vec<Vec<usize>> {
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let n = sprites.len();
    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (sprites[i], sprites[j]);
            if a.x - gap < b.x + 8
                && b.x - gap < a.x + 8
                && a.y - gap < b.y + 8
                && b.y - gap < a.y + 8
            {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                parent[ri] = rj;
            }
        }
    }
    let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        match groups.iter_mut().find(|(r, _)| *r == root) {
            Some((_, group)) => group.push(i),
            None => groups.push((root, vec![i])),
        }
    }
    groups.into_iter().map(|(_, group)| group).collect()
}

/// A face or object picture: 32 x 32 palette indexes.
#[derive(Debug, Clone)]
pub struct Picture {
    pub half: u8,
    pub pixels: Vec<u8>,
}

/// The tile numbers of the two pictures of a set, row by row of the 4 x 4 grid.
const HALF_CELLS: [[usize; 16]; 2] = [
    [0, 1, 2, 3, 16, 17, 18, 19, 4, 5, 6, 7, 20, 21, 22, 23],
    [8, 9, 10, 11, 24, 25, 26, 27, 12, 13, 14, 15, 28, 29, 30, 31],
];

/// Both pictures of the compressed set at `stream`.
#[must_use]
pub fn set_pictures(rom: Rom<'_>, stream: u32) -> Option<[Picture; 2]> {
    let offset = rom.view.offset(stream)?;
    let mut data = lz_decode(rom.bytes, offset)?.data;
    regroup(&mut data);
    if data.len() < 32 * 32 {
        return None;
    }
    let make = |half: usize| {
        let mut pixels = vec![0_u8; 1024];
        for (cell, &tile) in HALF_CELLS[half].iter().enumerate() {
            let (tile_pixels, _) = tile_pixels(&data, tile);
            for row in 0..8 {
                for col in 0..8 {
                    pixels[((cell / 4) * 8 + row) * 32 + (cell % 4) * 8 + col] =
                        tile_pixels[row * 8 + col];
                }
            }
        }
        Picture { half: half as u8, pixels }
    };
    Some([make(0), make(1)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_catalogue::MIN_ROM_SIZE;

    fn put(rom: &mut [u8], address: u32, bytes: &[u8]) {
        let at = RomView::new(rom).unwrap().offset(address).unwrap();
        rom[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// A ROM with poseset 0 of one pose of two sprites: tile 1 (all colour 2) at x 0, tile 0 (all colour 1)
    /// at `gap`; the pose's graphics are one literal stream of the two tiles.
    fn rom(gap: u8) -> Vec<u8> {
        let mut rom = vec![0_u8; MIN_ROM_SIZE];
        put(&mut rom, 0x15_8060, &[0x00, 0x80, 0x16, 0x00, 0x90, 0x16]);
        put(&mut rom, 0x16_8000, &[0x00, 0x81, 0x16]);
        // Two tiles of 32 bytes in the planes' layout: tile 0 has plane 0 set, tile 1 has plane 1 set
        let mut data = [0_u8; 64];
        data[..8].fill(0xFF);
        data[40..48].fill(0xFF);
        let mut stream = vec![64, 0];
        for group in data.chunks(8) {
            stream.push(0xFF);
            stream.extend_from_slice(group);
        }
        put(&mut rom, 0x16_8100, &stream);
        // The layout table holds one pointer and the layout follows it
        put(&mut rom, 0x16_9000, &[0x02, 0x90, 0x82, 0x01, 0x00, 0x00, 0x00, gap, 0x80]);
        // Unit 1: its palette and poseset 0
        put(&mut rom, 0x1E_CB06 + 4, &[0x00, 0xA0, 0x16, 0x00]);
        rom
    }

    #[test]
    fn a_pose_is_its_layout_drawn_from_its_tiles() {
        let bytes = rom(8);
        let rom = Rom::new(&bytes).unwrap();
        assert_eq!(unit_row(rom, 1), Some(UnitRow { palette: 0x16_A000, poseset: 0 }));
        assert_eq!(pose_count(rom, 0), 1);
        let pose = decode_pose(rom, 0, 0).unwrap();
        assert!(!pose.short_tiles);
        let shape = pose.shape.unwrap();
        assert_eq!((shape.w, shape.h), (16, 8));
        assert_eq!(shape.arr[0], 2, "the first sprite is tile 1");
        assert_eq!(shape.arr[8], 1, "the second is tile 0");
        let (key, mirror) = shape.keys();
        assert_ne!(
            key, mirror,
            "a pose that is not symmetric has another key facing the other way"
        );
        assert_eq!(key, ssd_hd::resolve::pose_key(&shape.arr, 16, 8));
        assert!(pose.clusters.is_empty(), "the sprites touch");
    }

    #[test]
    fn a_pose_in_pieces_has_the_keys_of_its_clusters() {
        let bytes = rom(60);
        let rom = Rom::new(&bytes).unwrap();
        let pose = decode_pose(rom, 0, 0).unwrap();
        assert_eq!(pose.clusters.len(), 2);
        assert!(pose.clusters.iter().all(|cluster| cluster.opaque() == 64));
    }

    #[test]
    fn a_tile_the_data_lacks_draws_blank_and_says_so() {
        let mut bytes = rom(8);
        // The layout names tile 5, which the 64 bytes of data do not hold
        put(&mut bytes, 0x16_9002, &[0x00, 0x05, 0x00, 0x80]);
        let rom = Rom::new(&bytes).unwrap();
        let pose = decode_pose(rom, 0, 0).unwrap();
        assert!(pose.short_tiles);
        assert!(pose.shape.is_none());
    }

    #[test]
    fn a_portrait_set_has_two_halves_of_sixteen_tiles() {
        let mut bytes = vec![0_u8; MIN_ROM_SIZE];
        // A literal stream of 32 tiles, tile n all colour (n % 15) + 1 in plane 0 and 1 terms: use plane 0 only
        let mut data = vec![0_u8; 1024];
        for tile in 0..32 {
            data[tile * 32..tile * 32 + 8].fill(0xFF);
        }
        let mut stream = vec![0x00, 0x04];
        for group in data.chunks(8) {
            stream.push(0xFF);
            stream.extend_from_slice(group);
        }
        // A stream cannot cross a bank: put it where it fits
        put(&mut bytes, 0x16_8000, &stream[..0x7000.min(stream.len())]);
        let rom = Rom::new(&bytes).unwrap();
        let halves = set_pictures(rom, 0x16_8000).unwrap();
        assert!(
            halves[0].pixels.iter().all(|&c| c == 1) && halves[1].pixels.iter().all(|&c| c == 1)
        );
        assert_eq!((halves[0].half, halves[1].half), (0, 1));
    }
}
