//! `hd-import <dir>`: puts the approved HD art into the pack, so the renderer draws it.
//!
//! `<dir>` is the art directory (never tracked; `%LOCALAPPDATA%\dbz-ssd-recompiled\hd`):
//!
//! * `class1\index.json` and the PNGs beside it: the portraits and faces, 32 x 32 pictures redrawn
//!   at 4x. Each entry names the stream of the ROM and the half of it (`rom 16:C60A half 0`); its
//!   art covers the 16 characters of that half, which the pack's tile binding finds on screen.
//!   The palette the art was made for is read back from the entry's source picture
//!   (`class1\src\<id>.png`), because the same characters are drawn in several palettes.
//! * `class2\index.json` and the PNGs beside it: battle poses and effects at 4x. Each entry has
//!   the key of the pose's pixels (`source_key`, see `ssd_hd::poses`), so the renderer finds the
//!   art of an assembled sprite group by hashing it; `luma` marks a greyscale shape that takes
//!   its colours from the original pixels.
//! * `class3\index.json` and the PNGs beside it (optional): the field's sprites (the party, NPCs, ships and
//!   objects drawn as sprites), in the same format as class 2: each entry has the key of its sprite group.
//! * `cards\index.json` and the PNGs beside it: the battle cards, one 64 x 64 picture (8x) per tile of
//!   the game's UI set that a card is made of, in up to two layers (`base`, `roles`; see
//!   `ssd_hd::cards`). Each entry of `tiles` has the tile's `code` in the set and its files; `tips`
//!   are the 112 x 112 pictures of the tooltip (a kanji, an emblem), by the code of the card's
//!   kanji tile.
//!
//! * `maps\index.json` and the PNGs beside it (optional): the field maps' art, the layer of each map redrawn at 4x in
//!   tiles of 256 x 256 game pixels, with the layer at 1x as its source picture (`ssd_hd::mapart`); they are linked
//!   into `art/maps` of the pack and listed in `art/maps/maps.toml`.
//! * `mode7\index.json` and the PNGs beside it (optional): the Mode 7 plane's art (the world map a flight flies over),
//!   one plane redrawn at 4x in tiles of 256 x 256 game pixels, with the key of the plane's VRAM and the palette it was
//!   made in (`ssd_hd::mode7art`); linked into `art/mode7` of the pack and listed in `art/mode7/mode7.toml`.
//!
//! * `variants\index.json` and the PNGs beside it (optional): the poses' bigger art (the pose variants of the pack, see
//!   `ssd_hd::pack::PoseArt`): each entry of `variants` names the pose's `art` (its `id` in class 2), `when` it is drawn
//!   (`always`, or `kaioken`), the `margin` (art pixels on each side, a multiple of 4 up to 96) and the `file`: the pose's
//!   picture grown by the margin. Copied to `art/variants` of the pack and listed in `art/variants/variants.toml`; a data
//!   pack's variant of the same art wins over them.
//!
//! The art files are copied into the pack (`art/picture`, `art/object`, `art/cards`) and listed in
//! `art.bin` (the cards in `art/cards/cards.toml`). The pack must exist (`hd-convert`); running
//! `hd-convert` again removes the art, so the import is run after it.

use crate::convert;
use crate::tiles;
use anyhow::{Context, bail};
use ssd_hd::art_index::{ArtIndex, Object, Palette, Picture};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// Cell `n` of a 32 x 32 picture: the character of the stream's half that it shows. Half 0 is
/// the first eight characters of each of the stream's two rows of 16, half 1 the last eight (the
/// game uploads a portrait as two runs of eight).
const PICTURE_CELLS: [[u16; 16]; 2] = [
    [0, 1, 2, 3, 16, 17, 18, 19, 4, 5, 6, 7, 20, 21, 22, 23],
    [8, 9, 10, 11, 24, 25, 26, 27, 12, 13, 14, 15, 28, 29, 30, 31],
];

/// What an import did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub pictures: usize,
    pub picture_files: usize,
    pub objects: usize,
    /// Tiles of the cards' art.
    pub cards: usize,
    /// Layers of the maps' art, and the tiles of them.
    pub map_layers: usize,
    pub map_tiles: usize,
    /// Background layers with art.
    pub bg: usize,
    /// Mode 7 planes with art.
    pub mode7: usize,
    /// Pose variants (bigger poses' art).
    pub variants: usize,
    pub luma: usize,
    pub keys: usize,
    /// Objects with no silhouette (they are only found by their key).
    pub no_mask: usize,
    pub skipped: Vec<String>,
    /// Pictures whose palette is unknown (they match every palette, last).
    pub no_palette: Vec<String>,
}

fn json_str<'a>(value: &'a serde_json::Value, key: &str) -> anyhow::Result<&'a str> {
    value.get(key).and_then(serde_json::Value::as_str).with_context(|| format!("a `{key}` text"))
}

/// `rom 16:C60A half 1 (32x32, 4bpp)` -> the stream address `0x16C60A` and the half.
fn parse_source(text: &str) -> anyhow::Result<(u32, usize)> {
    let mut words = text.split_whitespace();
    anyhow::ensure!(words.next() == Some("rom"), "a source `rom BB:AAAA half N`: {text}");
    let address = words.next().context("an address")?;
    let (bank, offset) = address.split_once(':').context("BB:AAAA")?;
    anyhow::ensure!(words.next() == Some("half"), "a half: {text}");
    let half: usize = words.next().context("a half number")?.parse().context("a half number")?;
    anyhow::ensure!(half < 2, "half 0 or 1");
    Ok((u32::from_str_radix(bank, 16)? << 16 | u32::from_str_radix(offset, 16)?, half))
}

/// `#RRGGBB` -> its channels.
fn parse_colour(text: &str) -> anyhow::Result<[u8; 3]> {
    let hex = text.strip_prefix('#').context("a #RRGGBB colour")?;
    anyhow::ensure!(hex.len() == 6, "a #RRGGBB colour: {text}");
    Ok([
        u8::from_str_radix(&hex[0..2], 16)?,
        u8::from_str_radix(&hex[2..4], 16)?,
        u8::from_str_radix(&hex[4..6], 16)?,
    ])
}

fn read_png(path: &Path) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    ssd_hd::png::read_rgba(&bytes).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

/// The palette a picture's source was drawn with, from the palette the entry names in the ROM
/// (`palette 09:D4A0`: sixteen colours, which the picture's indexes pick from), when it does.
fn rom_palette(rom: &[u8], text: &str, indexes: &[u8; 1024]) -> anyhow::Result<Option<Palette>> {
    let Some(at) = text.find("palette 09:") else { return Ok(None) };
    let offset = u32::from_str_radix(text[at + 11..].get(..4).context("four digits")?, 16)?;
    let view = ssd_catalogue::RomView::new(rom)?;
    let start = view.offset(0x09_0000 | offset).context("a palette inside the ROM")?;
    let mut palette = Palette::default();
    for index in 0..16 {
        let word = u16::from_le_bytes([rom[start + 2 * index], rom[start + 2 * index + 1]]);
        let channel = |shift: u32| (((u32::from(word) >> shift) & 31) * 255 + 15) / 31;
        palette.colors[index] = [channel(0) as u8, channel(5) as u8, channel(10) as u8];
    }
    for &index in indexes {
        palette.mask |= 1 << index;
    }
    Ok(Some(palette))
}

/// The palettes the layers' art was made in, by layer id: `ref-palettes.json` of the art folder
/// (`{"bg": {id: text}, "maps": {id: text}}`, each text 256 colours of four hexadecimal digits,
/// see `ssd_hd::bgart::parse_palette`). The recolouring of a layer shown in another palette needs
/// them when the pack holds no source picture.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct RefPalettes {
    #[serde(default)]
    pub bg: BTreeMap<String, String>,
    #[serde(default)]
    pub maps: BTreeMap<String, String>,
}

fn ref_palettes(art_dir: &Path) -> anyhow::Result<RefPalettes> {
    let Ok(text) = std::fs::read_to_string(art_dir.join(crate::public::REF_PALETTES)) else {
        return Ok(RefPalettes::default());
    };
    let palettes: RefPalettes = serde_json::from_str(&text).context("ref-palettes.json")?;
    for (kind, list) in [("bg", &palettes.bg), ("maps", &palettes.maps)] {
        for (id, text) in list {
            anyhow::ensure!(
                ssd_hd::bgart::parse_palette(text).is_some(),
                "ref-palettes.json: {kind} {id} is not 256 colours of four hexadecimal digits"
            );
        }
    }
    Ok(palettes)
}

/// The palette a picture's entry carries inline (`palette_colours`: palette index to `#RRGGBB`),
/// when it has one.
fn inline_palette(entry: &serde_json::Value) -> anyhow::Result<Option<Palette>> {
    let Some(colours) = entry.get("palette_colours").and_then(serde_json::Value::as_object) else {
        return Ok(None);
    };
    let mut palette = Palette::default();
    for (index, colour) in colours {
        let index: usize = index.parse()?;
        anyhow::ensure!(index < 16, "a palette index");
        palette.colors[index] = parse_colour(colour.as_str().context("a colour")?)?;
        palette.mask |= 1 << index;
    }
    Ok(Some(palette))
}

/// The palette a picture's source was drawn with: for each palette index the picture's pixels
/// use, the colour of the source image at those pixels (the most common, if it varies).
fn picture_palette(indexes: &[u8; 1024], source: &(u32, u32, Vec<u8>)) -> anyhow::Result<Palette> {
    anyhow::ensure!((source.0, source.1) == (32, 32), "a 32 x 32 source picture");
    let mut counts: Vec<BTreeMap<[u8; 3], u32>> = vec![BTreeMap::new(); 16];
    for (at, &index) in indexes.iter().enumerate() {
        let pixel = &source.2[at * 4..at * 4 + 3];
        *counts[usize::from(index)].entry([pixel[0], pixel[1], pixel[2]]).or_default() += 1;
    }
    let mut palette = Palette::default();
    for (index, colours) in counts.iter().enumerate() {
        if let Some((&colour, _)) = colours.iter().max_by_key(|&(_, &count)| count) {
            palette.colors[index] = colour;
            palette.mask |= 1 << index;
        }
    }
    Ok(palette)
}

/// The palette-index picture of half `half` of the 4-bit stream `tiles`: 32 x 32 indexes.
fn picture_indexes(tiles: &[&[u8]], half: usize) -> anyhow::Result<[u8; 1024]> {
    let mut picture = [0_u8; 1024];
    for (cell, &number) in PICTURE_CELLS[half].iter().enumerate() {
        let tile = tiles.get(usize::from(number)).context("a stream of 32 characters")?;
        let pixels = tiles::decode_tile(tile, 4);
        for row in 0..8 {
            for col in 0..8 {
                picture[((cell / 4) * 8 + row) * 32 + (cell % 4) * 8 + col] = pixels[row * 8 + col];
            }
        }
    }
    Ok(picture)
}

/// The silhouette of RGBA pixels as a bitset: a bit for each pixel that is lit (its red channel,
/// the white of a mask, over 127) and opaque.
fn silhouette(rgba: &[u8]) -> Vec<u8> {
    let mut mask = vec![0_u8; (rgba.len() / 4).div_ceil(8)];
    for (at, pixel) in rgba.as_chunks::<4>().0.iter().enumerate() {
        if pixel[0] > 127 && pixel[3] > 127 {
            mask[at / 8] |= 1 << (at % 8);
        }
    }
    mask
}

/// Puts the art file in the pack under `relative` after checking its size: a hard link when the
/// pack is on the volume of the art store, a copy otherwise.
fn copy_art(from: &Path, pack: &Path, relative: &str, expected: (u32, u32)) -> anyhow::Result<()> {
    let (width, height) = image_size(from)?;
    anyhow::ensure!(
        (width, height) == expected,
        "{}: {width}x{height}, expected {}x{}",
        from.display(),
        expected.0,
        expected.1
    );
    link_art(from, &pack.join(relative))
}

/// The art file's name with the extension of the file it is made from: the art is a PNG or a
/// WebP (`ssd_hd::png::read_rgba` takes both), and the pack keeps what the store holds.
fn with_extension_of(relative: &str, from: &Path) -> String {
    let ext = from.extension().and_then(|ext| ext.to_str()).unwrap_or("png");
    match relative.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.{ext}"),
        None => relative.to_owned(),
    }
}

/// Copies the card tiles of `<art>/cards/index.json` into `<pack>/art/cards` and writes the cards'
/// index there; returns the number of tiles (0 when there is no such folder).
fn import_cards(cards: &Path, pack_dir: &Path, report: &mut Report) -> anyhow::Result<usize> {
    let index_path = cards.join("index.json");
    if !index_path.is_file() {
        return Ok(0);
    }
    let text = std::fs::read_to_string(&index_path)?;
    let index: serde_json::Value = serde_json::from_str(&text).context("cards/index.json")?;
    let tile = ssd_hd::cards::TILE as u32;
    let mut entries = Vec::new();
    for entry in index["tiles"].as_array().context("cards: a `tiles` list")? {
        let code = entry["code"].as_u64().context("a tile code")?;
        let set = entry.get("set").and_then(serde_json::Value::as_str);
        anyhow::ensure!(
            matches!(set, None | Some("title")),
            "tile {code}: the set is `title` or none"
        );
        let (set_name, tiles_in_set) = if set.is_some() { ("title", 256) } else { ("UI", 384) };
        anyhow::ensure!(
            code < tiles_in_set,
            "tile {code}: the {set_name} set has {tiles_in_set} tiles"
        );
        let carve = entry.get("carve").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let mut layer = |key: &str| -> anyhow::Result<Option<String>> {
            let Some(name) = entry.get(key).and_then(serde_json::Value::as_str) else {
                return Ok(None);
            };
            let from = cards.join(name);
            if !from.is_file() {
                report.skipped.push(format!("card tile {code}: no {key} file {name}"));
                return Ok(None);
            }
            copy_art(&from, pack_dir, &format!("art/cards/{name}"), (tile, tile))?;
            Ok(Some(name.to_owned()))
        };
        let (base, roles) = (layer("base")?, layer("roles")?);
        if base.is_some() || roles.is_some() {
            entries.push(ssd_hd::cards::Entry {
                code: code as u16,
                set: set.map(str::to_owned),
                carve,
                base,
                roles,
            });
        }
    }
    let mut tip = Vec::new();
    for entry in index.get("tips").and_then(serde_json::Value::as_array).into_iter().flatten() {
        let code = entry["code"].as_u64().context("a tooltip code")?;
        let name = json_str(entry, "image")?;
        let side = ssd_hd::cards::TIP_PX as u32;
        copy_art(&cards.join(name), pack_dir, &format!("art/cards/{name}"), (side, side))?;
        tip.push(ssd_hd::cards::TipEntry { code: code as u16, image: name.to_owned() });
    }
    let toml = toml::to_string(&ssd_hd::cards::Index { tile: entries, tip })?;
    let path = pack_dir.join(ssd_hd::cards::INDEX);
    let count = toml.matches("[[tile]]").count();
    std::fs::write(path, toml)?;
    Ok(count)
}

/// Puts the art file in the pack: a hard link when the pack is on the volume of the art store (the
/// maps' art is a few gigabytes), a copy otherwise.
fn link_art(from: &Path, to: &Path) -> anyhow::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if to.exists() {
        std::fs::remove_file(to)?;
    }
    if std::fs::hard_link(from, to).is_err() {
        std::fs::copy(from, to).with_context(|| format!("copying {}", from.display()))?;
    }
    Ok(())
}

/// The size of an art file, a PNG or a WebP, from its header.
fn image_size(path: &Path) -> anyhow::Result<(u32, u32)> {
    ssd_hd::png::file_dimensions(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

/// Puts the field maps' art of `<art>/maps/index.json` into `<pack>/art/maps` and writes the maps'
/// index there (`maps.toml`): for each layer its source picture and the tiles of its art, checked
/// for their sizes. The index lists the layers by the size and the data pointers of the map they
/// are the picture of (`ssd_hd::mapart`).
fn import_maps(
    maps: &Path,
    pack_dir: &Path,
    palettes: &BTreeMap<String, String>,
    report: &mut Report,
) -> anyhow::Result<()> {
    use ssd_hd::mapart::{self, Entry};
    let index_path = maps.join("index.json");
    if !index_path.is_file() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&index_path)?;
    let index: serde_json::Value = serde_json::from_str(&text).context("maps/index.json")?;
    anyhow::ensure!(index["version"] == 1, "maps: version 1 expected");
    anyhow::ensure!(
        index["scale"] == mapart::SCALE && index["tile"] == mapart::TILE,
        "maps: art at {}x in tiles of {} game pixels expected",
        mapart::SCALE,
        mapart::TILE
    );
    let root = pack_dir.join("art").join("maps");
    if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }
    let mut layers: Vec<Entry> = Vec::new();
    for layer in index["layers"].as_array().context("maps: a `layers` list")? {
        let id = json_str(layer, "id")?;
        let blocks: Vec<u8> = layer["blocks"]
            .as_array()
            .context("a `blocks` pair")?
            .iter()
            .map(|n| n.as_u64().and_then(|n| u8::try_from(n).ok()))
            .collect::<Option<_>>()
            .context("`blocks`: two small numbers")?;
        anyhow::ensure!(
            blocks.len() == 2 && blocks.iter().all(|&n| (1..=12).contains(&n)),
            "{id}: `blocks` is two numbers from 1 to 12"
        );
        let entry = Entry {
            id: id.to_owned(),
            maps: layer["maps"]
                .as_array()
                .context("a `maps` list")?
                .iter()
                .filter_map(|n| n.as_u64().and_then(|n| u8::try_from(n).ok()))
                .collect(),
            blocks: [blocks[0], blocks[1]],
            grid: json_str(layer, "grid")?.to_owned(),
            metatiles: json_str(layer, "metatiles")?.to_owned(),
            // (a pack made for sharing has no picture of the game's layer)
            source: layer
                .get("source")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned(),
            palette: palettes.get(id).cloned().unwrap_or_default(),
            palette_variant: layer
                .get("palette_variant")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            tiles: json_str(layer, "tiles")?.to_owned(),
        };
        anyhow::ensure!(
            mapart::parse_pointer(&entry.grid).is_some()
                && mapart::parse_pointer(&entry.metatiles).is_some(),
            "{id}: the pointers are `BB:AAAA`"
        );
        anyhow::ensure!(
            entry.tiles.contains("{x}") && entry.tiles.contains("{y}"),
            "{id}: `tiles` names the column `{{x}}` and the row `{{y}}`"
        );
        let (width, height) = entry.size();
        if !entry.source.is_empty() {
            let source = maps.join(&entry.source);
            anyhow::ensure!(
                image_size(&source)? == (width as u32, height as u32),
                "{}: the source is not {width}x{height}",
                source.display()
            );
            link_art(&source, &root.join(&entry.source))?;
        }
        let (columns, rows) = entry.tile_grid();
        let side = (mapart::TILE * mapart::SCALE) as u32;
        for y in 0..rows {
            for x in 0..columns {
                let file = entry.tile_file(x, y);
                let from = maps.join(&file);
                anyhow::ensure!(
                    image_size(&from)? == (side, side),
                    "{}: a tile is {side}x{side}",
                    from.display()
                );
                link_art(&from, &root.join(&file))?;
                report.map_tiles += 1;
            }
        }
        // (two layers have the same layout only when one is the art of other maps' palette)
        anyhow::ensure!(
            entry.palette_variant
                || !layers.iter().any(|other| !other.palette_variant
                    && other.blocks == entry.blocks
                    && other.grid == entry.grid
                    && other.metatiles == entry.metatiles),
            "{id}: another layer has the same size and data pointers"
        );
        layers.push(entry);
    }
    report.map_layers = layers.len();
    std::fs::write(
        pack_dir.join(mapart::INDEX),
        toml::to_string(&mapart::Index { layer: layers })?,
    )?;
    Ok(())
}

/// Copies the background layers of `<art>/bg/index.json` into `<pack>/art/bg` and writes their
/// index there (`ssd_hd::bgart`); returns the number of layers (0 when there is no such folder).
/// Each entry of `layers` names the registers that show the layer (`bg`, `mode`, `map_base`,
/// `char_base`, `size`), the tile `rows` its art covers, its content `key`, whether the art `fill`s
/// its pixels, its `file` (4x, RGBA) and its `source` (1x, the colours the art was made from; a
/// pack made for sharing has none and the art is then drawn as it is in any palette).
fn import_bg(
    bg: &Path,
    pack_dir: &Path,
    palettes: &BTreeMap<String, String>,
    report: &mut Report,
) -> anyhow::Result<usize> {
    let index_path = bg.join("index.json");
    if !index_path.is_file() {
        return Ok(0);
    }
    let text = std::fs::read_to_string(&index_path)?;
    let index: serde_json::Value = serde_json::from_str(&text).context("bg/index.json")?;
    let number = |entry: &serde_json::Value, key: &str| -> anyhow::Result<u64> {
        entry
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .with_context(|| format!("a `{key}` number"))
    };
    let mut layers = Vec::new();
    for entry in index["layers"].as_array().context("bg: a `layers` list")? {
        let id = json_str(entry, "id")?;
        let key = json_str(entry, "key")?;
        u64::from_str_radix(key, 16).with_context(|| format!("{id}: a hexadecimal key"))?;
        let rows = entry["rows"].as_array().context("rows")?;
        let rows = [
            rows.first().and_then(serde_json::Value::as_u64).context("a first row")? as u16,
            rows.get(1).and_then(serde_json::Value::as_u64).context("a last row")? as u16,
        ];
        anyhow::ensure!(rows[0] < rows[1] && rows[1] <= 64, "{id}: rows {rows:?}");
        let mut layer = ssd_hd::bgart::Entry {
            id: id.to_owned(),
            key: key.to_owned(),
            bg: number(entry, "bg")? as u8,
            mode: number(entry, "mode")? as u8,
            map_base: number(entry, "map_base")? as u16,
            char_base: number(entry, "char_base")? as u16,
            size: number(entry, "size")? as u8,
            rows,
            fill: entry.get("fill").and_then(serde_json::Value::as_bool).unwrap_or(false),
            art: format!("art/bg/{id}.png"),
            source: format!("art/bg/{id}-src.png"),
            palette_key: entry
                .get("palette_key")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned(),
            // (a layer that is a palette variant carries its own palette in the index)
            palette: entry
                .get("palette")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .or_else(|| palettes.get(id).cloned())
                .unwrap_or_default(),
        };
        let (width, height) = layer.art_size(false);
        let art = bg.join(json_str(entry, "file")?);
        let source = entry.get("source").and_then(serde_json::Value::as_str).map(|n| bg.join(n));
        if !art.is_file() || source.as_ref().is_some_and(|source| !source.is_file()) {
            report.skipped.push(format!("{id}: no art or source file"));
            continue;
        }
        let scale = ssd_hd::bgart::SCALE as u32;
        let (w, h) = (width as u32, height as u32);
        layer.art = with_extension_of(&layer.art, &art);
        copy_art(&art, pack_dir, &layer.art, (w * scale, h * scale))?;
        match &source {
            Some(source) => {
                layer.source = with_extension_of(&layer.source, source);
                copy_art(source, pack_dir, &layer.source, (w, h))?;
            }
            None => layer.source = String::new(),
        }
        layers.push(layer);
    }
    let count = layers.len();
    let toml = toml::to_string(&ssd_hd::bgart::Index { layer: layers })?;
    let path = pack_dir.join(ssd_hd::bgart::INDEX);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, toml)?;
    Ok(count)
}

/// Puts the Mode 7 planes' art of `<art>/mode7/index.json` into `<pack>/art/mode7` and writes their
/// index there (`mode7.toml`, `ssd_hd::mode7art`); returns the number of planes (0 when there is no
/// such folder). Each entry of `planes` has an `id`, the content `key` of the plane's VRAM
/// (`hd-art m7dump` prints it), the `palette` the art was made in (256 colours, `RRGGBB`) and the
/// `tiles` of its art, 4x, in tiles of 256 x 256 game pixels named with `{x}` and `{y}`; every tile
/// is checked for its size.
fn import_mode7(mode7: &Path, pack_dir: &Path) -> anyhow::Result<usize> {
    use ssd_hd::mode7art::{Entry, Index, SCALE, TILE};
    let index_path = mode7.join("index.json");
    if !index_path.is_file() {
        return Ok(0);
    }
    let text = std::fs::read_to_string(&index_path)?;
    let index: serde_json::Value = serde_json::from_str(&text).context("mode7/index.json")?;
    anyhow::ensure!(index["version"] == 1, "mode7: version 1 expected");
    anyhow::ensure!(
        index["scale"] == SCALE && index["tile"] == TILE,
        "mode7: art at {SCALE}x in tiles of {TILE} game pixels expected"
    );
    let root = pack_dir.join("art").join("mode7");
    if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }
    let mut planes: Vec<Entry> = Vec::new();
    for plane in index["planes"].as_array().context("mode7: a `planes` list")? {
        let id = json_str(plane, "id")?;
        let key = json_str(plane, "key")?;
        u64::from_str_radix(key, 16).with_context(|| format!("{id}: a hexadecimal key"))?;
        let palette: Vec<String> = plane["palette"]
            .as_array()
            .with_context(|| format!("{id}: a `palette` list"))?
            .iter()
            .map(|c| c.as_str().map(str::to_owned))
            .collect::<Option<_>>()
            .with_context(|| format!("{id}: the palette holds strings"))?;
        let entry = Entry {
            id: id.to_owned(),
            key: key.to_owned(),
            tiles: json_str(plane, "tiles")?.to_owned(),
            palette,
        };
        anyhow::ensure!(entry.colours().is_some(), "{id}: the palette is 256 colours `RRGGBB`");
        anyhow::ensure!(
            entry.tiles.contains("{x}") && entry.tiles.contains("{y}"),
            "{id}: `tiles` names the column `{{x}}` and the row `{{y}}`"
        );
        anyhow::ensure!(
            planes.iter().all(|other| other.key_value() != entry.key_value()),
            "{id}: another plane has the same key"
        );
        let side = (TILE * SCALE) as u32;
        for y in 0..Entry::tiles_across() {
            for x in 0..Entry::tiles_across() {
                let file = entry.tile_file(x, y);
                let from = mode7.join(&file);
                anyhow::ensure!(
                    image_size(&from)? == (side, side),
                    "{}: a tile is {side}x{side}",
                    from.display()
                );
                link_art(&from, &root.join(&file))?;
            }
        }
        planes.push(entry);
    }
    let count = planes.len();
    std::fs::write(
        pack_dir.join(ssd_hd::mode7art::INDEX),
        toml::to_string(&Index { plane: planes })?,
    )?;
    Ok(count)
}

/// The name of a pose variant's picture, and its `id` in the art store's index: the art's name for `always`, else the art's name
/// and the condition.
#[must_use]
pub fn variant_id(art: &str, when: &str) -> String {
    if when == "always" { art.to_owned() } else { format!("{art}-{when}") }
}

/// Puts the pose variants of `<art>/variants/index.json` into `<pack>/art/variants` and lists them in
/// `art/variants/variants.toml`; returns their number (0 when there is no such folder). An entry names the `art` of an object
/// of `objects`, the condition it is drawn `when`, its `margin` and its `file`, which must be the object's picture grown by the
/// margin: a size that is not that, a margin or a condition the game does not have, or two variants of one art for one
/// condition is an error; an entry with no file, or of art the pack has no pose of, is skipped.
fn import_variants(
    variants: &Path,
    pack_dir: &Path,
    objects: &[Object],
    report: &mut Report,
) -> anyhow::Result<usize> {
    let index_path = variants.join("index.json");
    if !index_path.is_file() {
        return Ok(0);
    }
    let text = std::fs::read_to_string(&index_path)?;
    let index: serde_json::Value = serde_json::from_str(&text).context("variants/index.json")?;
    let mut listed = Vec::new();
    for entry in index["variants"].as_array().context("variants: a `variants` list")? {
        let art = json_str(entry, "art")?;
        let when = json_str(entry, "when")?;
        let margin = u32::try_from(entry["margin"].as_u64().context("a margin")?)?;
        let name = json_str(entry, "file")?;
        let listed_entry = ssd_hd::pack::PoseArtEntry {
            art: art.to_owned(),
            when: when.to_owned(),
            margin,
            file: name.to_owned(),
        };
        listed_entry
            .variant()
            .map_err(|reason| anyhow::anyhow!("variant {art} ({when}): {reason}"))?;
        anyhow::ensure!(
            !listed
                .iter()
                .any(|done: &ssd_hd::pack::PoseArtEntry| done.art == art && done.when == when),
            "variant {art} ({when}) twice"
        );
        let Some(object) = objects.iter().find(|object| object.id == art) else {
            report.skipped.push(format!("variant {art} ({when}): no pose of that art"));
            continue;
        };
        let from = variants.join(name);
        if !from.is_file() {
            report.skipped.push(format!("variant {art} ({when}): no art file"));
            continue;
        }
        let relative =
            with_extension_of(&format!("art/variants/{}.png", variant_id(art, when)), &from);
        copy_art(
            &from,
            pack_dir,
            &relative,
            (u32::from(object.width) * 4 + 2 * margin, u32::from(object.height) * 4 + 2 * margin),
        )
        .with_context(|| format!("variant {art} ({when})"))?;
        listed.push(ssd_hd::pack::PoseArtEntry { file: relative, ..listed_entry });
    }
    if listed.is_empty() {
        return Ok(0);
    }
    let count = listed.len();
    std::fs::write(
        pack_dir.join(ssd_hd::pack::POSE_ART),
        toml::to_string(&ssd_hd::pack::PoseArt { variants: listed })?,
    )?;
    Ok(count)
}

/// The folders of the art store that hold the sprites' art (`class2`, `class3`, any other `classN`
/// with an `index.json`), sorted by name. Every folder with an `index.json` is read: one that no
/// reader knows is an error, so a branch that predates a folder never drops it from the pack
/// silently.
fn object_folders(art_dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(art_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.path().join("index.json").is_file() || KNOWN_OTHER.contains(&name.as_str()) {
            continue;
        }
        anyhow::ensure!(
            name.starts_with("class"),
            "{}: an art folder `{name}` with an index.json that this hd-import does not know: merge origin/main",
            art_dir.display()
        );
        if name != "class1" {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Folders of the art store with their own readers.
const KNOWN_OTHER: [&str; 5] = ["cards", "maps", "bg", "mode7", "variants"];

/// Imports the art under `art_dir` into the pack in `pack_dir`, for the ROM `rom`. `sources` is
/// the folder of the portraits' source pictures (`<id>.png`, 32 x 32, in the palette the art was
/// made for), which says the palette of a face the entry does not name. `masks` is the folder of
/// the poses' silhouettes (`<source key>.png`, white where the pose is opaque), which let a
/// partial pose find the pose it is part of; the silhouettes of a shared pack are one file of the
/// art folder, `masks.bin` ([`crate::public::Masks`]), which is read when there is no folder. A
/// picture's entry may carry its palette as `palette_colours` (index to `#RRGGBB`), which is what
/// a shared pack does instead of the source pictures.
///
/// # Errors
///
/// The pack, the indexes or an art file cannot be read, an art file has the wrong size, or a
/// picture names a stream the pack does not have.
pub fn run(
    rom: &[u8],
    art_dir: &Path,
    sources: Option<&Path>,
    masks: Option<&Path>,
    pack_dir: &Path,
    out: &mut dyn Write,
) -> anyhow::Result<Report> {
    anyhow::ensure!(
        pack_dir.join("hdpack.toml").is_file(),
        "no pack at {}: run hd-convert first",
        pack_dir.display()
    );
    let found = convert::discover(rom)?;
    let streams: BTreeMap<u32, &convert::Found> =
        found.iter().filter(|item| item.bpp == 4).map(|item| (item.source, item)).collect();
    // The art files are copied beside the pack's art folder and put in its place at the end, so a
    // renderer reading the pack while it is imported never finds the folder half made
    let staging = pack_dir.join(".import");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(staging.join("art"))?;
    let mut report = Report::default();
    let mut pictures = Vec::new();
    let class1 = art_dir.join("class1");
    if class1.join("index.json").is_file() {
        let text = std::fs::read_to_string(class1.join("index.json"))?;
        let index: serde_json::Value = serde_json::from_str(&text).context("class1/index.json")?;
        for entry in index["assets"].as_array().context("class1: an `assets` list")? {
            let id = json_str(entry, "id")?;
            let (source, half) = parse_source(json_str(entry, "source")?)?;
            let Some(stream) = streams.get(&source) else {
                report.skipped.push(format!("{id}: no 4-bit stream at {source:06X}"));
                continue;
            };
            let from =
                class1.join(entry.get("file").and_then(serde_json::Value::as_str).unwrap_or(""));
            let from = if from.is_file() { from } else { class1.join(format!("{id}.png")) };
            if !from.is_file() {
                report.skipped.push(format!("{id}: no art file"));
                continue;
            }
            let indexes = picture_indexes(&stream.tiles(), half)?;
            let named = rom_palette(
                rom,
                entry.get("palette").and_then(serde_json::Value::as_str).unwrap_or(""),
                &indexes,
            )?;
            let source = sources.map(|dir| dir.join(format!("{id}.png"))).filter(|p| p.is_file());
            let palette = match (inline_palette(entry)?, source, named) {
                (Some(palette), ..) | (None, None, Some(palette)) => palette,
                (None, Some(path), _) => picture_palette(&indexes, &read_png(&path)?)?,
                (None, None, None) => {
                    report.no_palette.push(id.to_owned());
                    Palette::default()
                }
            };
            let relative = with_extension_of(&format!("art/picture/{id}.png"), &from);
            copy_art(&from, &staging, &relative, (128, 128))?;
            pictures.push(Picture {
                name: id.to_owned(),
                asset: stream.id(),
                tiles: PICTURE_CELLS[half],
                palette,
                art: relative,
            });
            report.picture_files += 1;
        }
        report.pictures = pictures.len();
    }

    let packed_masks = match std::fs::read(art_dir.join(crate::public::MASKS)) {
        Ok(bytes) => Some(crate::public::Masks::decode(&bytes)?),
        Err(_) => None,
    };
    let mut objects = Vec::new();
    for class_name in object_folders(art_dir)? {
        let class2 = art_dir.join(&class_name);
        if class2.join("index.json").is_file() {
            let text = std::fs::read_to_string(class2.join("index.json"))?;
            let index: serde_json::Value =
                serde_json::from_str(&text).with_context(|| format!("{class_name}/index.json"))?;
            for entry in index["assets_list"]
                .as_array()
                .with_context(|| format!("{class_name}: an `assets_list`"))?
            {
                let id = json_str(entry, "id")?;
                let key = u64::from_str_radix(json_str(entry, "source_key")?, 16)?;
                let size = entry["source_size"].as_array().context("a source_size")?;
                let (width, height) = (
                    size[0].as_u64().context("a width")? as u16,
                    size[1].as_u64().context("a height")? as u16,
                );
                let luma = entry.get("luma").and_then(serde_json::Value::as_bool).unwrap_or(false);
                let mut palette = Palette::default();
                if let Some(used) =
                    entry.get("palette_indexes_used").and_then(serde_json::Value::as_object)
                {
                    for (index, colour) in used {
                        let index: usize = index.parse()?;
                        anyhow::ensure!(index < 16, "a palette index");
                        palette.colors[index] = parse_colour(colour.as_str().context("a colour")?)?;
                        palette.mask |= 1 << index;
                    }
                }
                let from = class2.join(json_str(entry, "file")?);
                if !from.is_file() {
                    report.skipped.push(format!("{id}: no art file"));
                    continue;
                }
                let relative = with_extension_of(&format!("art/object/{id}.png"), &from);
                copy_art(
                    &from,
                    &staging,
                    &relative,
                    (u32::from(width) * 4, u32::from(height) * 4),
                )?;
                let mask = match masks.map(|dir| dir.join(format!("{key:016x}.png"))) {
                    Some(path) if path.is_file() => {
                        let (w, h, pixels) = read_png(&path)?;
                        anyhow::ensure!(
                            (w, h) == (u32::from(width), u32::from(height)),
                            "{}: {w}x{h}, expected {width}x{height}",
                            path.display()
                        );
                        silhouette(&pixels)
                    }
                    _ => match packed_masks.as_ref().and_then(|packed| packed.get(key)) {
                        Some((w, h, bits)) if (w, h) == (width, height) => bits.to_vec(),
                        _ => {
                            report.no_mask += 1;
                            Vec::new()
                        }
                    },
                };
                objects.push(Object {
                    key,
                    id: id.to_owned(),
                    luma,
                    palette,
                    mask,
                    width,
                    height,
                    art: relative,
                });
                report.luma += usize::from(luma);
            }
        }
    }
    report.objects = objects.len();
    report.keys =
        objects.iter().map(|object| object.key).collect::<std::collections::BTreeSet<_>>().len();

    let art_root = pack_dir.join("art");
    if art_root.exists() {
        std::fs::remove_dir_all(&art_root)?;
    }
    std::fs::rename(staging.join("art"), &art_root)?;
    std::fs::remove_dir_all(&staging)?;
    report.cards = import_cards(&art_dir.join("cards"), pack_dir, &mut report)?;
    let palettes = ref_palettes(art_dir)?;
    import_maps(&art_dir.join("maps"), pack_dir, &palettes.maps, &mut report)?;
    report.bg = import_bg(&art_dir.join("bg"), pack_dir, &palettes.bg, &mut report)?;
    report.mode7 = import_mode7(&art_dir.join("mode7"), pack_dir)?;
    report.variants = import_variants(&art_dir.join("variants"), pack_dir, &objects, &mut report)?;

    let index = ArtIndex::new(pictures, objects);
    std::fs::write(pack_dir.join(ssd_hd::art_index::FILE), index.encode())?;
    writeln!(
        out,
        "hd-import: {} pictures (portraits and faces), {} objects ({} luma, {} distinct keys, {} without a silhouette) into {}",
        report.pictures,
        report.objects,
        report.luma,
        report.keys,
        report.no_mask,
        pack_dir.display()
    )?;
    if !report.no_palette.is_empty() {
        writeln!(
            out,
            "hd-import: no palette for {}: {}",
            report.no_palette.len(),
            report.no_palette.join(" ")
        )?;
    }
    for reason in &report.skipped {
        writeln!(out, "hd-import: skipped {reason}")?;
    }
    if report.cards > 0 {
        writeln!(out, "hd-import: {} card tiles", report.cards)?;
    }
    if report.map_layers > 0 {
        writeln!(
            out,
            "hd-import: {} map layers ({} art tiles)",
            report.map_layers, report.map_tiles
        )?;
    }
    if report.bg > 0 {
        writeln!(out, "hd-import: {} background layers", report.bg)?;
    }
    if report.mode7 > 0 {
        writeln!(out, "hd-import: {} Mode 7 planes", report.mode7)?;
    }
    if report.variants > 0 {
        writeln!(out, "hd-import: {} pose variants (bigger poses' art)", report.variants)?;
    }
    if report.pictures
        + report.objects
        + report.cards
        + report.map_layers
        + report.bg
        + report.mode7
        == 0
    {
        bail!(
            "{} holds no art (class1, class2, class3, cards, maps, bg or mode7 with an index.json)",
            art_dir.display()
        );
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_class_folder_is_read_and_an_unknown_folder_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["class1", "class2", "class3", "class7", "cards", "bg", "mode7", "empty"] {
            std::fs::create_dir_all(dir.path().join(name)).unwrap();
            if name != "empty" {
                std::fs::write(dir.path().join(name).join("index.json"), "{}").unwrap();
            }
        }
        assert_eq!(object_folders(dir.path()).unwrap(), ["class2", "class3", "class7"]);
        std::fs::create_dir_all(dir.path().join("newer")).unwrap();
        std::fs::write(dir.path().join("newer/index.json"), "{}").unwrap();
        assert!(object_folders(dir.path()).is_err());
    }

    #[test]
    fn the_cards_art_is_copied_and_indexed_by_tile() {
        let dir = tempfile::tempdir().unwrap();
        let (art, pack) = (dir.path().join("hd/cards"), dir.path().join("pack"));
        std::fs::create_dir_all(&art).unwrap();
        let png = |name: &str, side: u32| {
            let rgba = vec![9_u8; (side * side * 4) as usize];
            ssd_hd::png::write_rgba(&art.join(name), side, side, &rgba).unwrap();
        };
        png("a.png", 64);
        png("a-roles.png", 64);
        png("tip.png", 112);
        std::fs::write(
            art.join("index.json"),
            r#"{"tiles": [{"code": 146, "base": "a.png", "roles": "a-roles.png"},
                          {"code": 147, "roles": "missing.png"},
                          {"code": 203, "set": "title", "carve": true, "roles": "a-roles.png"}],
                "tips": [{"code": 248, "image": "tip.png"}]}"#,
        )
        .unwrap();
        let mut report = Report::default();
        assert_eq!(import_cards(&art, &pack, &mut report).unwrap(), 2);
        assert_eq!(report.skipped.len(), 1, "the tile with no art file is skipped: {report:?}");
        let loaded = ssd_hd::CardArt::load(&pack).unwrap().expect("the pack has card art");
        assert!(format!("{loaded:?}").contains("tiles: 2"));
        // The title set's tile keeps its set and its carving in the pack's index
        let listed = std::fs::read_to_string(pack.join(ssd_hd::cards::INDEX)).unwrap();
        assert!(listed.contains("set = \"title\"") && listed.contains("carve = true"), "{listed}");
        // A title tile past the set's 256, and a set that is neither, are refused
        std::fs::write(
            art.join("index.json"),
            r#"{"tiles": [{"code": 256, "set": "title", "roles": "a-roles.png"}]}"#,
        )
        .unwrap();
        assert!(import_cards(&art, &pack, &mut report).is_err());
        std::fs::write(
            art.join("index.json"),
            r#"{"tiles": [{"code": 1, "set": "other", "roles": "a-roles.png"}]}"#,
        )
        .unwrap();
        assert!(import_cards(&art, &pack, &mut report).is_err());
        std::fs::write(
            art.join("index.json"),
            r#"{"tiles": [{"code": 146, "base": "a.png", "roles": "a-roles.png"}],
                "tips": [{"code": 248, "image": "tip.png"}]}"#,
        )
        .unwrap();
        assert_eq!(import_cards(&art, &pack, &mut report).unwrap(), 1);
        assert!(pack.join("art/cards/tip.png").is_file());
        // No folder, no cards
        let none = import_cards(&dir.path().join("nothing"), &pack, &mut report).unwrap();
        assert_eq!(none, 0);
        // A picture of the wrong size is refused
        png("a.png", 32);
        assert!(import_cards(&art, &pack, &mut report).is_err());
    }

    #[test]
    fn the_maps_art_is_linked_into_the_pack_and_listed_by_layout() {
        let dir = tempfile::tempdir().unwrap();
        let (maps, pack) = (dir.path().join("hd/maps"), dir.path().join("pack"));
        std::fs::create_dir_all(maps.join("map-01")).unwrap();
        let png = |name: &str, side: u32| {
            let rgba = vec![9_u8; (side * side * 4) as usize];
            ssd_hd::png::write_rgba(&maps.join(name), side, side, &rgba).unwrap();
        };
        png("map-01/source.png", 256);
        png("map-01/t0_0.png", 1024);
        let index = r#"{"version": 1, "scale": 4, "tile": 256, "layers": [
            {"id": "map-01", "maps": [1, 6], "blocks": [1, 1], "grid": "06:F45E",
             "metatiles": "09:EFB0", "source": "map-01/source.png",
             "tiles": "map-01/t{x}_{y}.png"}]}"#;
        std::fs::write(maps.join("index.json"), index).unwrap();
        let mut report = Report::default();
        import_maps(&maps, &pack, &BTreeMap::new(), &mut report).unwrap();
        assert_eq!((report.map_layers, report.map_tiles), (1, 1));
        let loaded = ssd_hd::mapart::Index::open(&pack).unwrap();
        assert_eq!(loaded.find((1, 1), (0x06, 0xF45E), (0x09, 0xEFB0), 1), Some(0));
        assert!(pack.join("art/maps/map-01/t0_0.png").is_file());
        // The same layer twice, a tile of the wrong size and a missing tile are refused
        let twice = index.replace("]}", ",")
            + r#"{"id": "map-02", "maps": [2], "blocks": [1, 1], "grid": "06:F45E",
                  "metatiles": "09:EFB0", "source": "map-01/source.png",
                  "tiles": "map-01/t{x}_{y}.png"}]}"#;
        std::fs::write(maps.join("index.json"), &twice).unwrap();
        assert!(import_maps(&maps, &pack, &BTreeMap::new(), &mut Report::default()).is_err());
        // ... unless it is the art of a palette variant for other maps
        let variant = twice.replace("\"maps\": [2]", "\"maps\": [2], \"palette_variant\": true");
        std::fs::write(maps.join("index.json"), variant).unwrap();
        import_maps(&maps, &pack, &BTreeMap::new(), &mut Report::default()).unwrap();
        let loaded = ssd_hd::mapart::Index::open(&pack).unwrap();
        assert_eq!(loaded.find((1, 1), (0x06, 0xF45E), (0x09, 0xEFB0), 2), Some(1));
        assert_eq!(loaded.find((1, 1), (0x06, 0xF45E), (0x09, 0xEFB0), 1), Some(0));
        std::fs::write(maps.join("index.json"), index).unwrap();
        png("map-01/t0_0.png", 512);
        assert!(import_maps(&maps, &pack, &BTreeMap::new(), &mut Report::default()).is_err());
        std::fs::remove_file(maps.join("map-01/t0_0.png")).unwrap();
        assert!(import_maps(&maps, &pack, &BTreeMap::new(), &mut Report::default()).is_err());
        // No folder, no maps
        assert!(
            import_maps(
                &dir.path().join("nothing"),
                &pack,
                &BTreeMap::new(),
                &mut Report::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn the_mode_7_planes_art_is_linked_into_the_pack_and_listed_by_key() {
        let dir = tempfile::tempdir().unwrap();
        let (mode7, pack) = (dir.path().join("hd/mode7"), dir.path().join("pack"));
        std::fs::create_dir_all(mode7.join("plane")).unwrap();
        let png = |name: &str, side: u32| {
            let rgba = vec![9_u8; (side * side * 4) as usize];
            ssd_hd::png::write_rgba(&mode7.join(name), side, side, &rgba).unwrap();
        };
        for y in 0..4 {
            for x in 0..4 {
                png(&format!("plane/t{x}_{y}.png"), 1024);
            }
        }
        let palette = vec!["102030"; 256].join("\", \"");
        let index = format!(
            r#"{{"version": 1, "scale": 4, "tile": 256, "planes": [
            {{"id": "plane", "key": "00000000000000ab", "tiles": "plane/t{{x}}_{{y}}.png",
              "palette": ["{palette}"]}}]}}"#
        );
        std::fs::write(mode7.join("index.json"), &index).unwrap();
        assert_eq!(import_mode7(&mode7, &pack).unwrap(), 1);
        let loaded = ssd_hd::mode7art::Index::open(&pack).unwrap();
        assert_eq!(loaded.find(0xAB), Some(0));
        assert!(pack.join("art/mode7/plane/t3_3.png").is_file());
        // A palette that is not 256 colours, a tile of the wrong size and a missing tile are refused
        std::fs::write(mode7.join("index.json"), index.replace("102030", "1020")).unwrap();
        assert!(import_mode7(&mode7, &pack).is_err());
        std::fs::write(mode7.join("index.json"), &index).unwrap();
        png("plane/t0_0.png", 512);
        assert!(import_mode7(&mode7, &pack).is_err());
        std::fs::remove_file(mode7.join("plane/t0_0.png")).unwrap();
        assert!(import_mode7(&mode7, &pack).is_err());
        // No folder, no plane
        assert_eq!(import_mode7(&dir.path().join("nothing"), &pack).unwrap(), 0);
    }

    #[test]
    fn the_background_art_is_copied_checked_by_size_and_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let (art, pack) = (dir.path().join("hd/bg"), dir.path().join("pack"));
        std::fs::create_dir_all(&art).unwrap();
        let png = |name: &str, width: u32, height: u32| {
            let rgba = vec![7_u8; (width * height * 4) as usize];
            ssd_hd::png::write_rgba(&art.join(name), width, height, &rgba).unwrap();
        };
        // Rows 0 to 2 of a one-screen layer: 256 x 16 game pixels, 4x
        png("a.png", 1024, 64);
        png("a-src.png", 256, 16);
        let palette = "4321".repeat(256);
        let index = |file: &str| {
            format!(
                r#"{{"layers": [{{"id": "a", "key": "00000000000000ff", "bg": 3, "mode": 0,
                   "map_base": 28672, "char_base": 20480, "size": 0, "rows": [0, 2], "fill": true,
                   "file": "{file}", "source": "a-src.png",
                   "palette_key": "00000000000000aa", "palette": "{palette}"}},
                  {{"id": "b", "key": "1", "bg": 3, "mode": 0, "map_base": 1, "char_base": 1,
                   "size": 0, "rows": [0, 2], "file": "none.png", "source": "none.png"}}]}}"#
            )
        };
        std::fs::write(art.join("index.json"), index("a.png")).unwrap();
        let mut report = Report::default();
        assert_eq!(import_bg(&art, &pack, &BTreeMap::new(), &mut report).unwrap(), 1);
        assert_eq!(report.skipped.len(), 1, "the layer with no files is skipped: {report:?}");
        let loaded = ssd_hd::bgart::Index::open(&pack).unwrap();
        assert_eq!(loaded.layer.len(), 1);
        assert_eq!(loaded.layer[0].key_value(), 0xFF);
        assert!(loaded.layer[0].fill && loaded.layer[0].map_base == 0x7000);
        // A palette variant says which palette it was made for and carries that palette
        assert_eq!(loaded.layer[0].palette_value(), Some(0xAA));
        assert_eq!(loaded.layer[0].palette, palette);
        assert!(pack.join("art/bg/a.png").is_file() && pack.join("art/bg/a-src.png").is_file());
        // No folder, no layers; art of the wrong size is refused
        assert_eq!(
            import_bg(&dir.path().join("nothing"), &pack, &BTreeMap::new(), &mut report).unwrap(),
            0
        );
        png("a.png", 1024, 32);
        assert!(import_bg(&art, &pack, &BTreeMap::new(), &mut report).is_err());
    }

    #[test]
    fn the_pose_variants_are_checked_by_size_copied_and_listed() {
        let dir = tempfile::tempdir().unwrap();
        let (art, pack) = (dir.path().join("hd/variants"), dir.path().join("pack"));
        std::fs::create_dir_all(&art).unwrap();
        let png = |name: &str, width: u32, height: u32| {
            let rgba = vec![7_u8; (width * height * 4) as usize];
            ssd_hd::png::write_rgba(&art.join(name), width, height, &rgba).unwrap();
        };
        let object = |id: &str| Object {
            key: 1,
            id: id.to_owned(),
            luma: false,
            palette: Palette::default(),
            mask: Vec::new(),
            width: 4,
            height: 5,
            art: String::new(),
        };
        let objects = [object("battle-a-000"), object("battle-a-001")];
        // The pose is 4 x 5 game pixels (16 x 20 art pixels): with a margin of 8 art pixels each side, 32 x 36
        png("a.png", 32, 36);
        let index = |body: &str| format!(r#"{{"variants": [{body}]}}"#);
        let entry = |art: &str, when: &str, margin: u32, file: &str| {
            format!(r#"{{"art": "{art}", "when": "{when}", "margin": {margin}, "file": "{file}"}}"#)
        };
        let write = |body: String| std::fs::write(art.join("index.json"), index(&body)).unwrap();
        let mut report = Report::default();
        write(entry("battle-a-000", "always", 8, "a.png"));
        assert_eq!(import_variants(&art, &pack, &objects, &mut report).unwrap(), 1);
        assert!(pack.join("art/variants/battle-a-000.png").is_file());
        let text = std::fs::read_to_string(pack.join(ssd_hd::pack::POSE_ART)).unwrap();
        let listed = toml::from_str::<ssd_hd::pack::PoseArt>(&text).unwrap().variants;
        assert_eq!((listed[0].art.as_str(), listed[0].margin), ("battle-a-000", 8));
        assert_eq!(listed[0].file, "art/variants/battle-a-000.png");
        // A condition other than `always` names its file after it
        write(entry("battle-a-000", "kaioken", 8, "a.png"));
        assert_eq!(import_variants(&art, &pack, &objects, &mut report).unwrap(), 1);
        assert!(pack.join("art/variants/battle-a-000-kaioken.png").is_file());
        // No folder, no variants; an entry with no file, or of art the pack has no pose of, is skipped
        assert_eq!(
            import_variants(&dir.path().join("none"), &pack, &objects, &mut report).unwrap(),
            0
        );
        write(format!(
            "{}, {}",
            entry("battle-a-000", "always", 8, "none.png"),
            entry("battle-b-000", "always", 8, "a.png")
        ));
        assert_eq!(import_variants(&art, &pack, &objects, &mut report).unwrap(), 0);
        assert_eq!(report.skipped.len(), 2, "{report:?}");
        // A picture of another size, a margin that is not a whole game pixel, a condition the game does not have, and two
        // variants of one art for one condition are refused
        write(entry("battle-a-000", "always", 4, "a.png"));
        assert!(import_variants(&art, &pack, &objects, &mut report).is_err(), "the size");
        write(entry("battle-a-000", "always", 7, "a.png"));
        assert!(import_variants(&art, &pack, &objects, &mut report).is_err(), "the margin");
        write(entry("battle-a-000", "bloom", 8, "a.png"));
        assert!(import_variants(&art, &pack, &objects, &mut report).is_err(), "the condition");
        write(format!(
            "{}, {}",
            entry("battle-a-000", "always", 8, "a.png"),
            entry("battle-a-000", "always", 8, "a.png")
        ));
        assert!(import_variants(&art, &pack, &objects, &mut report).is_err(), "twice");
    }

    #[test]
    fn a_source_names_a_stream_and_a_half() {
        assert_eq!(parse_source("rom 16:C60A half 1 (32x32, 4bpp)").unwrap(), (0x16_C60A, 1));
        assert!(parse_source("rom 16:C60A half 2").is_err());
        assert!(parse_source("somewhere").is_err());
    }

    #[test]
    fn a_colour_is_six_hex_digits() {
        assert_eq!(parse_colour("#FFBD84").unwrap(), [0xFF, 0xBD, 0x84]);
        assert!(parse_colour("FFBD84").is_err());
        assert!(parse_colour("#FFBD8").is_err());
    }

    #[test]
    fn the_cells_of_the_two_halves_are_the_sixteen_characters_of_their_runs() {
        let mut all: Vec<u16> = PICTURE_CELLS.iter().flatten().copied().collect();
        all.sort_unstable();
        assert_eq!(all, (0..32).collect::<Vec<u16>>());
    }

    #[test]
    fn a_palette_is_the_commonest_source_colour_of_each_index() {
        let mut indexes = [0_u8; 1024];
        indexes[0] = 3;
        indexes[1] = 3;
        indexes[2] = 3;
        let mut rgba = vec![0_u8; 1024 * 4];
        for (at, colour) in [[1, 2, 3], [1, 2, 3], [9, 9, 9]].iter().enumerate() {
            rgba[at * 4..at * 4 + 3].copy_from_slice(colour);
        }
        let palette = picture_palette(&indexes, &(32, 32, rgba)).unwrap();
        assert_eq!(palette.colors[3], [1, 2, 3]);
        assert_eq!(palette.mask & 0b1000, 0b1000);
    }
}
