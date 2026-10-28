//! The HD pack's index and manifest: the assets the converter found in the ROM, each as the keys
//! (content hashes) of its tiles in order, and where its art is. The index holds no ROM bytes and
//! no pixel data; the art is in the pack's image files (PNG or WebP). Format 1.

use crate::art_index::{ArtIndex, Palette, Picture};
use crate::mapart;
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

/// The version of the pack and of its index (`hdpack.toml`'s `format`).
pub const FORMAT: u32 = 1;

const MAGIC: &[u8; 8] = b"SSDHDIDX";

/// Why a pack cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    /// The manifest or the index cannot be read.
    #[error("reading the pack at {}: {message}", path.display())]
    Read { path: PathBuf, message: String },
    /// The index is not an index of this format (wrong magic, version, checksum or layout).
    #[error("index: {0}")]
    Index(&'static str),
    /// The manifest says another format or another number of assets than the index holds.
    #[error("manifest: {0}")]
    Manifest(String),
}

/// The content hash of a tile: FNV-1a over its bytes as they sit in VRAM.
#[must_use]
pub fn tile_key(bytes: &[u8]) -> u64 {
    fnv1a(bytes.iter().copied())
}

/// [`tile_key`] of the VRAM words `words` (each word's low byte first).
#[must_use]
pub fn word_key(words: &[u16]) -> u64 {
    fnv1a(words.iter().flat_map(|word| word.to_le_bytes()))
}

fn fnv1a(bytes: impl Iterator<Item = u8>) -> u64 {
    bytes.fold(0xCBF2_9CE4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3)
    })
}

/// How an asset's pixels are read: palette indexes resolved through the CGRAM row the game
/// uses (`Indexed`, so fades and palette cycles keep working), or colours as drawn (`Rgba`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Indexed,
    Rgba,
}

/// One asset of the pack: tiles of one bit depth, in the order of the stream they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// The logical id (`<class>/<source>/<n>`): a name, not data, the same in every ROM revision.
    pub id: String,
    /// Bits a pixel takes: 2 or 4.
    pub bpp: u8,
    pub mode: Mode,
    /// Tiles per row of the art image.
    pub columns: u16,
    /// The key of each tile, in order.
    pub keys: Vec<u64>,
    /// The art file, relative to the pack directory.
    pub art: String,
}

/// A tile of the index: the asset and the position of the tile in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileRef {
    pub asset: u32,
    pub tile: u32,
}

/// The index: the assets, and the tiles that each key names.
#[derive(Debug, Clone)]
pub struct Index {
    assets: Vec<Asset>,
    tiles: FxHashMap<(u8, u64), Vec<TileRef>>,
}

impl Index {
    /// The index of `assets`, in this order.
    #[must_use]
    pub fn new(assets: Vec<Asset>) -> Self {
        let mut tiles: FxHashMap<(u8, u64), Vec<TileRef>> = FxHashMap::default();
        for (asset, data) in assets.iter().enumerate() {
            for (tile, &key) in data.keys.iter().enumerate() {
                let reference = TileRef { asset: asset as u32, tile: tile as u32 };
                tiles.entry((data.bpp, key)).or_default().push(reference);
            }
        }
        Self { assets, tiles }
    }

    #[must_use]
    pub fn assets(&self) -> &[Asset] {
        &self.assets
    }

    /// The asset `index` names.
    ///
    /// # Panics
    ///
    /// `index` is not an asset of this index.
    #[must_use]
    pub fn asset(&self, index: u32) -> &Asset {
        &self.assets[index as usize]
    }

    /// Every tile of `bpp` bits whose key is `key`, in index order (none, one, or several when
    /// the same tile art is in more than one stream).
    #[must_use]
    pub fn candidates(&self, bpp: u8, key: u64) -> &[TileRef] {
        self.tiles.get(&(bpp, key)).map_or(&[], Vec::as_slice)
    }

    /// The index as its file: magic, format, the assets (id, bit depth, mode, columns, art
    /// path and keys, little-endian), then the CRC-32 of everything before it.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&FORMAT.to_le_bytes());
        out.extend_from_slice(&(self.assets.len() as u32).to_le_bytes());
        for asset in &self.assets {
            put_str(&mut out, &asset.id);
            out.push(asset.bpp);
            out.push(match asset.mode {
                Mode::Indexed => 0,
                Mode::Rgba => 1,
            });
            out.extend_from_slice(&asset.columns.to_le_bytes());
            put_str(&mut out, &asset.art);
            out.extend_from_slice(&(asset.keys.len() as u32).to_le_bytes());
            for key in &asset.keys {
                out.extend_from_slice(&key.to_le_bytes());
            }
        }
        let crc = ssd_rom::crc32(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    /// Reads an index written by [`Index::encode`].
    ///
    /// # Errors
    ///
    /// The bytes are not an index of [`FORMAT`]: the magic, the version, the checksum or a
    /// length is wrong.
    pub fn decode(bytes: &[u8]) -> Result<Self, PackError> {
        let body_len = bytes.len().checked_sub(4).ok_or(PackError::Index("truncated"))?;
        let (body, crc) = bytes.split_at(body_len);
        if ssd_rom::crc32(body).to_le_bytes() != crc {
            return Err(PackError::Index("checksum"));
        }
        let mut reader = Reader { bytes: body, at: 0 };
        if reader.take(8)? != MAGIC {
            return Err(PackError::Index("not an HD pack index"));
        }
        if reader.u32()? != FORMAT {
            return Err(PackError::Index("another format"));
        }
        let count = reader.u32()?;
        let mut assets = Vec::new();
        for _ in 0..count {
            let id = reader.string()?;
            let bpp = reader.u8()?;
            let mode = match reader.u8()? {
                0 => Mode::Indexed,
                1 => Mode::Rgba,
                _ => return Err(PackError::Index("a mode")),
            };
            let columns = reader.u16()?;
            let art = reader.string()?;
            let keys = (0..reader.u32()?).map(|_| reader.u64()).collect::<Result<Vec<_>, _>>()?;
            assets.push(Asset { id, bpp, mode, columns, keys, art });
        }
        if reader.at != body.len() {
            return Err(PackError::Index("trailing bytes"));
        }
        Ok(Self::new(assets))
    }
}

fn put_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u16).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

/// A cursor over the index bytes; every read is checked.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], PackError> {
        let end = self.at.checked_add(len).ok_or(PackError::Index("truncated"))?;
        let bytes = self.bytes.get(self.at..end).ok_or(PackError::Index("truncated"))?;
        self.at = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, PackError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, PackError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("two bytes")))
    }

    fn u32(&mut self) -> Result<u32, PackError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes")))
    }

    fn u64(&mut self) -> Result<u64, PackError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes")))
    }

    fn string(&mut self) -> Result<String, PackError> {
        let len = usize::from(self.u16()?);
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| PackError::Index("a name"))
    }
}

/// `hdpack.toml`: the pack's format, scale, converter version and the ROM it was made from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub scale: u32,
    pub converter: u32,
    /// The CRC-32 of the canonical ROM, as eight hex digits.
    pub rom_crc32: String,
    /// The assets of the index, checked against it on load.
    pub assets: usize,
}

/// A pack on disk: its manifest and its index (the art files are read by the renderer).
#[derive(Debug, Clone)]
pub struct Pack {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub index: Index,
    /// The approved art the pack draws (empty without `art.bin`).
    pub art: crate::art_index::ArtIndex,
    /// The art of the field maps (empty without `art/maps/maps.toml`).
    pub maps: crate::mapart::Index,
    /// The art of the battle backgrounds (empty without `art/bg/bg.toml`).
    pub bg: crate::bgart::Index,
    /// The art of the Mode 7 plane (empty without `art/mode7/mode7.toml`).
    pub mode7: crate::mode7art::Index,
    /// The pose variants of the data packs drawn over this pack (empty without them; see [`Pack::with_overlay`]).
    pub variants: Vec<PoseVariant>,
    /// The pack's own pose variants: art bigger than the box of the original sprite for the poses that need it (empty without
    /// `art/variants/variants.toml`). The renderer draws one only while no data pack's variant of the same art holds
    /// (`ArtState::pose_variant`).
    pub pose_art: Vec<PoseVariant>,
}

/// Where the pack lists its own pose variants.
pub const POSE_ART: &str = "art/variants/variants.toml";

/// `art/variants/variants.toml`: the pack's own pose variants, as the art import writes them and [`Pack::open`] reads them.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PoseArt {
    #[serde(default, rename = "variant")]
    pub variants: Vec<PoseArtEntry>,
}

/// One pose variant of the pack: the same fields as a data pack's `[[pose_variant]]` (`ssd_mod::PoseVariantEntry`),
/// with the picture's file relative to the pack's folder.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PoseArtEntry {
    /// The name of the object's art it replaces (its `id` in the art index).
    pub art: String,
    /// The condition under which it is drawn (`always` or `kaioken`).
    pub when: String,
    /// The margin on each side, in art pixels: a multiple of 4 up to 96 (`ssd_mod::pose::refusal`).
    pub margin: u32,
    /// The picture, relative to the pack's folder.
    pub file: String,
}

impl PoseArtEntry {
    /// The entry as the renderer's variant (the margin in game pixels), or why it is refused.
    ///
    /// # Errors
    ///
    /// The condition is not one the game has, or the margin is not a whole number of game pixels up to the limit.
    pub fn variant(&self) -> Result<PoseVariant, String> {
        let entry = ssd_mod::PoseVariantEntry {
            art: self.art.clone(),
            when: self.when.clone(),
            file: self.file.clone(),
            margin: self.margin,
        };
        if let Some(reason) = ssd_mod::pose::refusal(&entry) {
            return Err(reason);
        }
        let when = ssd_mod::pose::Condition::from_name(&self.when).ok_or("a condition")?;
        Ok(PoseVariant {
            art: self.art.clone(),
            when,
            margin: self.margin / 4,
            file: self.file.clone(),
        })
    }
}

/// The pose variants `art/variants/variants.toml` of the pack in `dir` lists: none when there is no such file. An entry that
/// is refused is left out, and the log says why.
fn read_pose_art(dir: &Path) -> Result<Vec<PoseVariant>, PackError> {
    let path = dir.join(POSE_ART);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(PackError::Read {
                path: dir.to_path_buf(),
                message: format!("{POSE_ART}: {e}"),
            });
        }
    };
    let listed: PoseArt = toml::from_str(&text).map_err(|e| PackError::Read {
        path: dir.to_path_buf(),
        message: format!("{POSE_ART}: {e}"),
    })?;
    Ok(listed
        .variants
        .iter()
        .filter_map(|entry| match entry.variant() {
            Ok(variant) => Some(variant),
            Err(reason) => {
                log::warn!(
                    "the pack's pose variant {} of {} is left out: {reason}",
                    entry.file,
                    entry.art
                );
                None
            }
        })
        .collect())
}

/// A pose variant of a data pack (its `[[pose_variant]]` entry, `ssd_mod::pose`): the picture drawn in place of the art of the
/// object `art` while `when` holds. The picture is the object's own, grown by `margin` game pixels on each side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoseVariant {
    /// The name of the object's art it replaces (the object's `id` in the art index).
    pub art: String,
    /// The condition under which it is drawn.
    pub when: ssd_mod::pose::Condition,
    /// The margin on each side, in game pixels (the picture's art pixels divided by four).
    pub margin: u32,
    /// The picture: relative to the pack's folder, absolute once it is in a [`Pack`].
    pub file: String,
}

impl Pack {
    /// The newest pack of the ROM `rom_crc32` that the converters left under `cache`
    /// (`<cache>/hd/<crc>/<converter>/`): the highest converter number whose pack opens. `None`
    /// when there is none.
    #[must_use]
    pub fn find(cache: &Path, rom_crc32: u32) -> Option<Self> {
        let folder = cache.join("hd").join(format!("{rom_crc32:08X}"));
        let mut versions: Vec<(u32, PathBuf)> = std::fs::read_dir(&folder)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| Some((entry.file_name().to_str()?.parse().ok()?, entry.path())))
            .collect();
        versions.sort_by_key(|&(version, _)| std::cmp::Reverse(version));
        versions.into_iter().find_map(|(_, dir)| Self::open(&dir).ok())
    }

    /// Opens the pack in `dir`.
    ///
    /// # Errors
    ///
    /// The manifest or the index cannot be read, or they do not agree.
    pub fn open(dir: &Path) -> Result<Self, PackError> {
        let read = |name: &str| {
            std::fs::read(dir.join(name)).map_err(|e| PackError::Read {
                path: dir.to_path_buf(),
                message: format!("{name}: {e}"),
            })
        };
        let text = String::from_utf8(read("hdpack.toml")?)
            .map_err(|_| PackError::Manifest("not UTF-8".to_owned()))?;
        let manifest: Manifest =
            toml::from_str(&text).map_err(|e| PackError::Manifest(e.to_string()))?;
        if manifest.format != FORMAT {
            return Err(PackError::Manifest(format!("format {} is not {FORMAT}", manifest.format)));
        }
        let index = Index::decode(&read("index.bin")?)?;
        if index.assets().len() != manifest.assets {
            return Err(PackError::Manifest(format!(
                "{} assets, the index has {}",
                manifest.assets,
                index.assets().len()
            )));
        }
        let art = crate::art_index::ArtIndex::open(dir)?;
        let maps = crate::mapart::Index::open(dir)
            .map_err(|e| PackError::Read { path: dir.to_path_buf(), message: e.to_string() })?;
        let bg = crate::bgart::Index::open(dir)
            .map_err(|e| PackError::Read { path: dir.to_path_buf(), message: e.to_string() })?;
        let mode7 = crate::mode7art::Index::open(dir)
            .map_err(|e| PackError::Read { path: dir.to_path_buf(), message: e.to_string() })?;
        Ok(Self {
            dir: dir.to_path_buf(),
            manifest,
            index,
            art,
            maps,
            bg,
            mode7,
            variants: Vec::new(),
            pose_art: read_pose_art(dir)?,
        })
    }
}

/// The art of one data pack, drawn over the installed pack's (see [`Pack::with_overlay`]): the layers of its pack maps, the
/// pictures of its faces and its pose variants. The files it names are relative to the folder `dir` of the pack.
#[derive(Debug, Clone, Default)]
pub struct Overlay {
    /// The folder of the pack: the overlay's files are read from it.
    pub dir: PathBuf,
    /// The layers of the pack's maps. They come before the installed layers, so they are the ones drawn.
    pub maps: Vec<mapart::Entry>,
    /// The assets of the pack's faces (one per face: the keys of the 32 tiles of its record), appended to the index.
    pub assets: Vec<Asset>,
    /// The pictures of the pack's faces, appended to the art index: the picture of each face's cells.
    pub pictures: Vec<Picture>,
    /// The pose variants of the pack (`[[pose_variant]]`), in the order of the pack.
    pub variants: Vec<PoseVariant>,
}

/// The field's metatile table (`09:EFB0`): the table a pack map's layout names.
const METATILES: &str = "09:EFB0";

impl Overlay {
    /// The overlay of the art of the data pack `pack` (its id) in `image`, whose folder is `dir`.
    #[must_use]
    pub fn of_pack(image: &ssd_mod::Image, pack: &str, dir: &Path) -> Self {
        let maps = image
            .map_art
            .iter()
            .filter(|art| art.pack == pack)
            .map(|art| mapart::Entry {
                id: format!("{pack} map {}", art.map),
                maps: vec![art.map],
                blocks: [art.blocks.0, art.blocks.1],
                grid: format!("{:02X}:{:04X}", art.grid >> 16, art.grid & 0xFFFF),
                metatiles: METATILES.to_owned(),
                source: String::new(),
                palette: String::new(),
                palette_variant: false,
                tiles: art.tiles.clone(),
            })
            .collect();
        let mut assets = Vec::new();
        let mut pictures = Vec::new();
        for face in image.faces.iter().filter(|face| face.pack == pack) {
            // The keys are the tile keys of the binder (`word_key`, the VRAM bytes of each tile), so the characters the
            // game uploads for the face bind to this asset.
            let id = format!("face/{pack}/{}", face.id);
            let keys = face.tiles.iter().map(|tile| word_key(&tile_words(tile))).collect();
            assets.push(Asset {
                id: id.clone(),
                bpp: 4,
                mode: Mode::Indexed,
                columns: 4,
                keys,
                art: face.picture.clone(),
            });
            pictures.push(Picture {
                name: Path::new(&face.picture)
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                asset: id,
                tiles: std::array::from_fn(|cell| {
                    u16::try_from(ssd_mod::derive::FIRST_PICTURE[cell]).unwrap_or(0)
                }),
                palette: picture_palette(&face.palette, &face.tiles),
                art: face.picture.clone(),
            });
        }
        let variants = image
            .pose_variants
            .iter()
            .filter(|variant| variant.pack == pack)
            .map(|variant| PoseVariant {
                art: variant.art.clone(),
                when: variant.when,
                // The margin is a whole number of game pixels (ssd-mod checks it): four art pixels to one
                margin: variant.margin / 4,
                file: variant.file.clone(),
            })
            .collect();
        Self { dir: dir.to_path_buf(), maps, assets, pictures, variants }
    }
}

/// The 16 words of a 4-bit tile in the SNES layout (the words the VRAM holds, low byte first).
fn tile_words(tile: &[u8; 32]) -> [u16; 16] {
    std::array::from_fn(|word| u16::from_le_bytes([tile[2 * word], tile[2 * word + 1]]))
}

/// The palette index of pixel `(x, y)` of a 4-bit tile in the SNES layout.
fn nibble(tile: &[u8; 32], x: usize, y: usize) -> u8 {
    (0..4).fold(0_u8, |value, plane| {
        let byte = tile[plane / 2 * 16 + 2 * y + plane % 2];
        value | ((byte >> (7 - x)) & 1) << plane
    })
}

/// The palette a face's art was made for: the colours of its row, and the indexes its tiles use (bit `i` for index `i`,
/// 0 excepted: it is transparent).
fn picture_palette(row: &[u16; 16], tiles: &[[u8; 32]]) -> Palette {
    let mut mask = 0_u16;
    for tile in tiles {
        for y in 0..8 {
            for x in 0..8 {
                mask |= 1 << nibble(tile, x, y);
            }
        }
    }
    Palette {
        mask: mask & !1,
        colors: std::array::from_fn(|at| crate::art::poses::rgb555(row[at])),
    }
}

impl Pack {
    /// This pack with `overlay` on top: the overlay's map layers come first (so they are the ones drawn), and its assets,
    /// pictures and pose variants come after the installed ones. Each file the overlay names is read from the overlay's folder.
    #[must_use]
    pub fn with_overlay(mut self, overlay: Overlay) -> Self {
        let Overlay { dir, maps, assets, pictures, variants } = overlay;
        // Absolute: the renderer joins the names below the installed pack's folder, which an absolute name replaces
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        let file = |relative: &str| dir.join(relative).to_string_lossy().into_owned();
        let mut layer: Vec<mapart::Entry> = maps
            .into_iter()
            .map(|entry| mapart::Entry { tiles: file(&entry.tiles), ..entry })
            .collect();
        layer.append(&mut self.maps.layer);
        self.maps.layer = layer;
        let mut all = self.index.assets().to_vec();
        all.extend(assets.into_iter().map(|asset| Asset { art: file(&asset.art), ..asset }));
        self.index = Index::new(all);
        let mut shown = self.art.pictures().to_vec();
        shown.extend(
            pictures.into_iter().map(|picture| Picture { art: file(&picture.art), ..picture }),
        );
        self.art = ArtIndex::new(shown, self.art.objects().to_vec());
        self.variants.extend(
            variants
                .into_iter()
                .map(|variant| PoseVariant { file: file(&variant.file), ..variant }),
        );
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::Binder;

    fn asset(id: &str, bpp: u8, keys: &[u64]) -> Asset {
        Asset {
            id: id.to_owned(),
            bpp,
            mode: Mode::Indexed,
            columns: 4,
            keys: keys.to_vec(),
            art: format!("base/{id}.png"),
        }
    }

    #[test]
    fn a_key_names_every_tile_of_its_bit_depth() {
        let index =
            Index::new(vec![asset("a", 4, &[1, 2]), asset("b", 4, &[2]), asset("c", 2, &[2])]);
        assert_eq!(
            index.candidates(4, 2),
            [TileRef { asset: 0, tile: 1 }, TileRef { asset: 1, tile: 0 }]
        );
        assert_eq!(index.candidates(2, 2), [TileRef { asset: 2, tile: 0 }]);
        assert_eq!(index.candidates(4, 9), &[] as &[TileRef]);
    }

    #[test]
    fn an_index_reads_back_as_it_was_written_and_a_damaged_one_is_refused() {
        let index =
            Index::new(vec![asset("ui/11:8036/0", 2, &[7, 8]), asset("gfx/16:8000/0", 4, &[])]);
        let bytes = index.encode();
        let back = Index::decode(&bytes).unwrap();
        assert_eq!(back.assets(), index.assets());
        assert_eq!(back.encode(), bytes, "the same index, the same bytes");

        let mut damaged = bytes.clone();
        damaged[20] ^= 1;
        assert!(matches!(Index::decode(&damaged), Err(PackError::Index("checksum"))));
        assert!(matches!(Index::decode(&bytes[..10]), Err(PackError::Index(_))));
    }

    fn write_pack(dir: &std::path::Path, converter: u32) {
        std::fs::create_dir_all(dir).unwrap();
        let manifest = Manifest {
            format: FORMAT,
            scale: 4,
            converter,
            rom_crc32: "A8CF4F2A".to_owned(),
            assets: 0,
        };
        std::fs::write(dir.join("hdpack.toml"), toml::to_string(&manifest).unwrap()).unwrap();
        std::fs::write(dir.join("index.bin"), Index::new(Vec::new()).encode()).unwrap();
    }

    #[test]
    fn the_newest_pack_of_a_rom_is_the_highest_converter_that_opens() {
        let cache = tempfile::tempdir().unwrap();
        let rom = cache.path().join("hd").join("A8CF4F2A");
        assert!(Pack::find(cache.path(), 0xA8CF_4F2A).is_none(), "no pack yet");
        write_pack(&rom.join("1"), 1);
        write_pack(&rom.join("2"), 2);
        assert_eq!(Pack::find(cache.path(), 0xA8CF_4F2A).unwrap().manifest.converter, 2);
        // A pack that does not open (damaged index) is passed over for the older one
        std::fs::write(rom.join("2").join("index.bin"), b"junk").unwrap();
        assert_eq!(Pack::find(cache.path(), 0xA8CF_4F2A).unwrap().manifest.converter, 1);
        assert!(Pack::find(cache.path(), 0x1234_5678).is_none(), "another ROM");
    }

    #[test]
    fn keys_are_fnv1a_of_the_vram_bytes_low_byte_first() {
        assert_eq!(word_key(&[0x3412]), tile_key(&[0x12, 0x34]));
        assert_eq!(tile_key(b""), 0xCBF2_9CE4_8422_2325);
    }

    /// An installed pack of one asset (`installed/0`) and one map layer, in the folder `installed`.
    fn installed_pack() -> Pack {
        Pack {
            dir: PathBuf::from("installed"),
            manifest: Manifest {
                format: FORMAT,
                scale: 4,
                converter: 1,
                rom_crc32: "A8CF4F2A".to_owned(),
                assets: 1,
            },
            index: Index::new(vec![asset("installed/0", 4, &[7])]),
            art: ArtIndex::default(),
            maps: mapart::Index {
                layer: vec![mapart::Entry {
                    id: "installed".to_owned(),
                    maps: vec![0],
                    blocks: [1, 1],
                    grid: "06:F360".to_owned(),
                    metatiles: METATILES.to_owned(),
                    source: String::new(),
                    palette: String::new(),
                    palette_variant: false,
                    tiles: "t{x}_{y}.png".to_owned(),
                }],
            },
            bg: crate::bgart::Index::default(),
            mode7: crate::mode7art::Index::default(),
            variants: Vec::new(),
            pose_art: Vec::new(),
        }
    }

    #[test]
    fn an_overlay_comes_before_the_installed_layers_and_its_files_are_read_from_its_folder() {
        let overlay = Overlay {
            dir: PathBuf::from("packs").join("room"),
            maps: vec![mapart::Entry {
                id: "room map 64".to_owned(),
                maps: vec![64],
                blocks: [8, 6],
                grid: "06:0000".to_owned(),
                metatiles: METATILES.to_owned(),
                source: String::new(),
                palette: String::new(),
                palette_variant: false,
                tiles: "art/hd/maps/64/t{x}_{y}.png".to_owned(),
            }],
            assets: vec![asset("face/room/0", 4, &[1, 2])],
            pictures: Vec::new(),
            variants: vec![PoseVariant {
                art: "battle-goku-ssj-000".to_owned(),
                when: ssd_mod::pose::Condition::Kaioken,
                margin: 6,
                file: "art/hd/variants/battle-goku-ssj-000.png".to_owned(),
            }],
        };
        let pack = installed_pack().with_overlay(overlay);
        let ids: Vec<&str> = pack.maps.layer.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(ids, ["room map 64", "installed"], "the overlay's layer is drawn first");
        let folder = std::path::absolute(PathBuf::from("packs").join("room")).unwrap();
        assert_eq!(pack.variants.len(), 1);
        assert_eq!(
            pack.variants[0].file,
            folder.join("art/hd/variants/battle-goku-ssj-000.png").to_string_lossy(),
            "a pose variant's file is absolute too"
        );
        let folder = std::path::absolute(PathBuf::from("packs").join("room")).unwrap();
        let tiles = folder.join("art/hd/maps/64/t{x}_{y}.png");
        assert_eq!(
            pack.maps.layer[0].tiles,
            tiles.to_string_lossy(),
            "absolute, the renderer joins below the pack"
        );
        assert_eq!(pack.index.assets().len(), 2, "the overlay's asset is appended");
        assert_eq!(
            pack.index.asset(0).id,
            "installed/0",
            "the installed assets keep their numbers"
        );
    }

    #[test]
    fn the_tiles_of_a_face_record_bind_to_the_face_picture_in_the_cells_they_fill() {
        // A ROM with the face palette row: object 28's pointer at 02:A38E names the row at 09:8000
        let mut rom = vec![0_u8; ssd_mod::ROM_LEN];
        let table = 0x02 * 0x8000 + (0xA38E - 0x8000);
        rom[table..table + 2].copy_from_slice(&0x8000_u16.to_le_bytes());
        let row = 0x09 * 0x8000;
        for (at, word) in [0, 0x7FFF, 0x001F, 0x03E0, 0x7C00, 0x4210].iter().enumerate() {
            rom[row + 2 * at..row + 2 * at + 2].copy_from_slice(&u16::to_le_bytes(*word));
        }
        // An HD picture of 128 x 128 in two colours, as a PNG
        let rgba: Vec<u8> = (0..128 * 128)
            .flat_map(|at| {
                let (x, y) = (at % 128, at / 128);
                if (x / 16 + y / 16) % 2 == 0 { [255, 0, 0, 255] } else { [0, 0, 255, 255] }
            })
            .collect();
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, 128, 128);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&rgba).unwrap();
        let info = |id: &str| ssd_mod::PackInfo {
            id: id.to_owned(),
            name: id.to_owned(),
            version: "1.0.0".to_owned(),
            game: format!("{:08X}", ssd_rom::crc32(&rom)),
            author: String::new(),
            description: String::new(),
        };
        let manifest = ssd_mod::Manifest {
            pack: info("face.pack"),
            depends: Vec::new(),
            data: Vec::new(),
            scene: Vec::new(),
            messages: Vec::new(),
            map: Vec::new(),
            triggers: Vec::new(),
            map_art: Vec::new(),
            face: vec![ssd_mod::FaceEntry { id: 0, hd: "art/hd/faces/a.png".to_owned() }],
            tables: Vec::new(),
            chapters: Vec::new(),
            feature: Vec::new(),
            pose_variant: Vec::new(),
        };
        let pack = ssd_mod::Pack {
            manifest,
            files: std::collections::BTreeMap::from([("art/hd/faces/a.png".to_owned(), png)]),
        };
        let image = ssd_mod::build(&rom, std::slice::from_ref(&pack)).unwrap();
        let overlay = Overlay::of_pack(&image, "face.pack", Path::new("packs/face"));
        let pack = installed_pack().with_overlay(overlay);
        // The record's 32 tiles in VRAM, one per character slot, as the game uploads them
        let mut vram = vec![0_u16; 32768];
        for (slot, tile) in image.faces[0].tiles.iter().enumerate() {
            vram[slot * 16..slot * 16 + 16].copy_from_slice(&tile_words(tile));
        }
        let mut binder = Binder::new(&pack.index);
        binder.update(&vram);
        let asset = pack.index.assets().iter().position(|a| a.id == "face/face.pack/0").unwrap();
        // Each tile of the first picture binds to the face's asset at its first place in the record (a tile that the record
        // holds twice binds to its first place, as the binder does for any repeated tile), and that place is a cell
        // of the picture
        let tiles = &image.faces[0].tiles;
        let picture = &pack.art.pictures()[0];
        assert_eq!(picture.asset, "face/face.pack/0");
        for &slot in &ssd_mod::derive::FIRST_PICTURE {
            let first = (0..tiles.len()).find(|&s| tiles[s] == tiles[slot]).unwrap();
            let found = binder.tile(4, u16::try_from(slot * 16).unwrap()).expect("the tile binds");
            assert_eq!(found.asset as usize, asset);
            assert_eq!(
                found.tile as usize, first,
                "the slot {slot} binds to its tile's first place"
            );
            let cell = picture.tiles.iter().position(|&t| u32::from(t) == found.tile).unwrap();
            assert_eq!(ssd_mod::derive::FIRST_PICTURE[cell], first);
        }
        assert_eq!(picture.name, "a");
    }

    #[test]
    fn a_pack_lists_its_own_pose_variants_and_one_that_is_refused_is_left_out() {
        let cache = tempfile::tempdir().unwrap();
        let dir = cache.path().join("pack");
        write_pack(&dir, 1);
        assert!(Pack::open(&dir).unwrap().pose_art.is_empty(), "an old pack lists none");
        std::fs::create_dir_all(dir.join("art/variants")).unwrap();
        let entry = |art: &str, when: &str, margin: u32| PoseArtEntry {
            art: art.to_owned(),
            when: when.to_owned(),
            margin,
            file: format!("art/variants/{art}.webp"),
        };
        let listed = PoseArt {
            variants: vec![
                entry("battle-goku-ssj-000", "always", 76),
                entry("battle-goku-ssj-001", "bloom", 76),
                entry("battle-goku-ssj-002", "always", 77),
                entry("battle-goku-ssj-003", "kaioken", 96),
            ],
        };
        std::fs::write(dir.join(POSE_ART), toml::to_string(&listed).unwrap()).unwrap();
        let pack = Pack::open(&dir).unwrap();
        let drawn: Vec<(&str, u32)> =
            pack.pose_art.iter().map(|variant| (variant.art.as_str(), variant.margin)).collect();
        assert_eq!(
            drawn,
            [("battle-goku-ssj-000", 19), ("battle-goku-ssj-003", 24)],
            "the margin in game pixels; an unknown condition and half a game pixel are left out"
        );
        assert_eq!(pack.pose_art[0].file, "art/variants/battle-goku-ssj-000.webp");
        assert!(pack.variants.is_empty(), "the data packs' variants are apart");
        std::fs::write(dir.join(POSE_ART), "variant = 3").unwrap();
        assert!(Pack::open(&dir).is_err(), "a list that is not a list is an error");
    }
}
