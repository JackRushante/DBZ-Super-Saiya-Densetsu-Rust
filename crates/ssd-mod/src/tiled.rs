//! Tiled import: a pack map written in Tiled, as JSON (`.tmj`, no XML). Its conventions are in `docs/mods/maps.md`; in short:
//!
//! - the map is orthogonal, 256 x 256 pixel tiles (one tile is one block), `width` and `height` in blocks;
//! - its map properties `music_like` and `palette_like` (int, original map ids `0` to `47`);
//! - its tilesets are embedded (not `source`). An original block is a tile of a tileset with no tile carrying a
//!   `metatiles` property: tile `t` is block `t` (the blocks `$00` to `$77`; `mod-tileset` writes one for painting);
//! - a new block is a tile of the one tileset whose tiles carry a `metatiles` property (a string of 256 metatile
//!   numbers, row by row): tile `t` is block `$78 + t`, so the tiles are numbered `0, 1, 2...`;
//! - the layer `blocks` (tile layer, the map's size): each cell is a tile's global id (`firstgid` + the tile);
//! - the layer `objects` (object layer): a point per NPC, with the int properties `object` (the original's record,
//!   `0` to `71`) and `message` (the original's message id);
//! - the layer `zones` (object layer): a rectangle per zone in pixels, with the int properties `level` and `group`;
//! - the layer `start` (object layer): exactly one point, the start in pixels.

use crate::map::{BlockDef, MapError, MapObject, NEW_BLOCK_BASE, PackMap, Zone};
use serde::Deserialize;

/// The size of a block in pixels: a Tiled tile.
const BLOCK_PX: u32 = 256;
/// The original blocks (`$00` to `$77`).
const ORIGINAL_BLOCKS: u32 = NEW_BLOCK_BASE as u32;

#[derive(Debug, Deserialize)]
struct Map {
    width: u32,
    height: u32,
    #[serde(default)]
    tilewidth: u32,
    #[serde(default)]
    tileheight: u32,
    #[serde(default)]
    orientation: String,
    #[serde(default)]
    layers: Vec<Layer>,
    #[serde(default)]
    tilesets: Vec<Tileset>,
    #[serde(default)]
    properties: Vec<Property>,
}

#[derive(Debug, Deserialize)]
struct Layer {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    data: Option<serde_json::Value>,
    #[serde(default)]
    objects: Vec<Object>,
}

#[derive(Debug, Deserialize)]
struct Object {
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    width: f64,
    #[serde(default)]
    height: f64,
    #[serde(default)]
    properties: Vec<Property>,
}

#[derive(Debug, Deserialize)]
struct Tileset {
    firstgid: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    tilecount: Option<u32>,
    #[serde(default)]
    tiles: Vec<Tile>,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Tile {
    id: u32,
    #[serde(default)]
    properties: Vec<Property>,
}

#[derive(Debug, Deserialize)]
struct Property {
    name: String,
    value: serde_json::Value,
}

/// The pack map `id` of the Tiled map in `text`.
///
/// # Errors
///
/// [`MapError::Tiled`] for a file that is not Tiled JSON or does not follow the conventions above (the message names
/// the layer, the tileset, the object or the cell), and whatever [`PackMap::check`] refuses.
pub fn parse(text: &str, id: u8) -> Result<PackMap, MapError> {
    let map: Map = serde_json::from_str(text)
        .map_err(|e| tiled(format!("the file is not Tiled JSON: {e}")))?;
    if !map.orientation.is_empty() && map.orientation != "orthogonal" {
        return Err(tiled(format!(
            "the map is `{}`: only orthogonal maps are read",
            map.orientation
        )));
    }
    if (map.tilewidth, map.tileheight) != (BLOCK_PX, BLOCK_PX) {
        return Err(tiled(format!(
            "the tiles are {}x{} pixels: a block is {BLOCK_PX} x {BLOCK_PX}",
            map.tilewidth, map.tileheight
        )));
    }
    let width = u8::try_from(map.width)
        .map_err(|_| MapError::Size { width: map.width, height: map.height })?;
    let height = u8::try_from(map.height)
        .map_err(|_| MapError::Size { width: map.width, height: map.height })?;
    let music_like = int_property(&map.properties, "music_like", "the map")?;
    let palette_like = int_property(&map.properties, "palette_like", "the map")?;

    let (originals, news) = tilesets(&map.tilesets)?;
    let grid = blocks_layer(&map, &originals, news.as_ref())?;
    let objects = objects_layer(&map)?;
    let zones = zones_layer(&map)?;
    let start = start_layer(&map)?;
    let blocks = news.map(|news| news.blocks).unwrap_or_default();
    let pack = PackMap {
        id,
        width,
        height,
        grid,
        blocks,
        music_like: music_like as u8,
        palette_like: palette_like as u8,
        start,
        objects,
        zones,
    };
    pack.check()?;
    Ok(pack)
}

/// A tile whose global id is `firstgid` plus its number, in an original tileset (its blocks are the numbers) or in
/// the block tileset (its blocks are `$78` plus the number).
struct Original {
    firstgid: u32,
    count: u32,
}

/// The one tileset of new block definitions: its first global id, and the definitions by tile number.
struct News {
    firstgid: u32,
    count: u32,
    blocks: Vec<BlockDef>,
}

/// Reads the tilesets: the original ones (as ranges of global ids) and the one that holds the new blocks.
fn tilesets(tilesets: &[Tileset]) -> Result<(Vec<Original>, Option<News>), MapError> {
    let mut originals = Vec::new();
    let mut news: Option<News> = None;
    for tileset in tilesets {
        if let Some(source) = &tileset.source {
            return Err(tiled(format!(
                "the tileset `{}` is the file `{source}`: embed the tileset in the map (Tiled: Embed Tilesets)",
                tileset.name
            )));
        }
        let has_metatiles =
            tileset.tiles.iter().any(|tile| tile.properties.iter().any(|p| p.name == "metatiles"));
        if !has_metatiles {
            let count = tileset.tilecount.unwrap_or(tileset.tiles.len() as u32);
            if count > ORIGINAL_BLOCKS {
                return Err(tiled(format!(
                    "the tileset `{}` has {count} tiles: an original tileset holds the blocks $00 to $77",
                    tileset.name
                )));
            }
            originals.push(Original { firstgid: tileset.firstgid, count });
            continue;
        }
        if news.is_some() {
            return Err(tiled(format!(
                "the tileset `{}` is a second tileset of new blocks: a map has one",
                tileset.name
            )));
        }
        let mut numbered: Vec<(u32, BlockDef)> = Vec::new();
        for tile in &tileset.tiles {
            let Some(property) = tile.properties.iter().find(|p| p.name == "metatiles") else {
                return Err(tiled(format!(
                    "tile {} of `{}` has no `metatiles` property: every tile of a block tileset is a block",
                    tile.id, tileset.name
                )));
            };
            let text = property.value.as_str().ok_or_else(|| {
                tiled(format!(
                    "tile {} of `{}`: `metatiles` is not a string",
                    tile.id, tileset.name
                ))
            })?;
            let numbers = text
                .split(',')
                .map(|number| number.trim().parse::<u16>())
                .collect::<Result<Vec<u16>, _>>()
                .map_err(|_| {
                    tiled(format!(
                        "tile {} of `{}`: `metatiles` is not a list of numbers",
                        tile.id, tileset.name
                    ))
                })?;
            numbered.push((tile.id, BlockDef::from_metatiles(&numbers)?));
        }
        numbered.sort_by_key(|(id, _)| *id);
        if numbered.iter().enumerate().any(|(k, (id, _))| *id != k as u32) {
            return Err(tiled(format!(
                "the tiles of `{}` are numbered 0, 1, 2... in order: block $78 + the tile number",
                tileset.name
            )));
        }
        let count = numbered.len() as u32;
        news = Some(News {
            firstgid: tileset.firstgid,
            count,
            blocks: numbered.into_iter().map(|(_, block)| block).collect(),
        });
    }
    Ok((originals, news))
}

/// The layer `blocks`, cut into the grid of block ids (row by row).
fn blocks_layer(
    map: &Map,
    originals: &[Original],
    news: Option<&News>,
) -> Result<Vec<u8>, MapError> {
    let layer = find_layer(map, "blocks", "tilelayer")?;
    let data = layer.data.as_ref().and_then(|data| data.as_array()).ok_or_else(|| {
        tiled("the layer `blocks` has no data array (save the layer as an array, not base64)")
    })?;
    if data.len() != (map.width as usize) * (map.height as usize) {
        return Err(tiled(format!(
            "the layer `blocks` has {} cells: the map is {} x {}",
            data.len(),
            map.width,
            map.height
        )));
    }
    let mut grid = Vec::with_capacity(data.len());
    for (index, value) in data.iter().enumerate() {
        let (x, y) = (index % map.width as usize, index / map.width as usize);
        let gid = value
            .as_u64()
            .ok_or_else(|| tiled(format!("cell ({x}, {y}) of `blocks` is not a number")))?;
        if gid >> 29 != 0 {
            return Err(tiled(format!(
                "cell ({x}, {y}) of `blocks` is flipped: flipped blocks are not in this version"
            )));
        }
        let gid = gid as u32;
        if gid == 0 {
            return Err(tiled(format!(
                "cell ({x}, {y}) of `blocks` is empty: every cell names a block"
            )));
        }
        let block = if let Some(news) = news.filter(|n| in_range(gid, n.firstgid, n.count)) {
            u32::from(NEW_BLOCK_BASE) + (gid - news.firstgid)
        } else if let Some(original) = originals.iter().find(|o| in_range(gid, o.firstgid, o.count))
        {
            gid - original.firstgid
        } else {
            return Err(tiled(format!(
                "cell ({x}, {y}) of `blocks` is tile {gid}, which no tileset holds"
            )));
        };
        grid.push(block as u8);
    }
    Ok(grid)
}

/// Whether global id `gid` is one of the `count` tiles from `firstgid`.
fn in_range(gid: u32, firstgid: u32, count: u32) -> bool {
    gid >= firstgid && gid - firstgid < count
}

/// The layer `objects`: the NPCs.
fn objects_layer(map: &Map) -> Result<Vec<MapObject>, MapError> {
    let layer = find_layer(map, "objects", "objectgroup")?;
    let mut objects = Vec::new();
    for (index, object) in layer.objects.iter().enumerate() {
        let at = format!("object {index} of `objects`");
        let object_id = int_property(&object.properties, "object", &at)?;
        let message = int_property(&object.properties, "message", &at)?;
        objects.push(MapObject {
            object: u8::try_from(object_id)
                .map_err(|_| tiled(format!("{at}: `object` is {object_id}")))?,
            x: pixel(object.x, &at)?,
            y: pixel(object.y, &at)?,
            message: u16::try_from(message)
                .map_err(|_| tiled(format!("{at}: `message` is {message}")))?,
        });
    }
    Ok(objects)
}

/// The layer `zones`: the rectangles, converted to the blocks they cover (inclusive).
fn zones_layer(map: &Map) -> Result<Vec<Zone>, MapError> {
    let layer = find_layer(map, "zones", "objectgroup")?;
    let mut zones = Vec::new();
    for (index, object) in layer.objects.iter().enumerate() {
        let at = format!("zone {index} of `zones`");
        let (x, y) = (pixel(object.x, &at)?, pixel(object.y, &at)?);
        let (w, h) = (pixel(object.width, &at)?, pixel(object.height, &at)?);
        if w == 0 || h == 0 {
            return Err(tiled(format!(
                "{at} is empty: a zone is a rectangle of at least one pixel"
            )));
        }
        let level = int_property(&object.properties, "level", &at)?;
        let group = int_property(&object.properties, "group", &at)?;
        let block = |px: u16| u32::from(px) / BLOCK_PX;
        let last = |px: u16, len: u16| (u32::from(px) + u32::from(len) - 1) / BLOCK_PX;
        zones.push(Zone {
            x0: block(x) as u8,
            x1: last(x, w) as u8,
            y0: block(y) as u8,
            y1: last(y, h) as u8,
            level: u8::try_from(level).map_err(|_| tiled(format!("{at}: `level` is {level}")))?,
            group: u8::try_from(group).map_err(|_| tiled(format!("{at}: `group` is {group}")))?,
        });
    }
    Ok(zones)
}

/// The layer `start`: exactly one point.
fn start_layer(map: &Map) -> Result<(u16, u16), MapError> {
    let layer = find_layer(map, "start", "objectgroup")?;
    match layer.objects.as_slice() {
        [start] => Ok((pixel(start.x, "the start")?, pixel(start.y, "the start")?)),
        other => {
            Err(tiled(format!("the layer `start` has {} objects: it has exactly one", other.len())))
        }
    }
}

/// A coordinate in pixels: a whole number from 0 to 65 535.
fn pixel(value: f64, at: &str) -> Result<u16, MapError> {
    if value.fract() != 0.0 || !(0.0..=65_535.0).contains(&value) {
        return Err(tiled(format!(
            "{at}: the coordinate {value} is not a whole pixel from 0 to 65535"
        )));
    }
    Ok(value as u16)
}

/// The int property `name`, required.
fn int_property(properties: &[Property], name: &str, at: &str) -> Result<i64, MapError> {
    let property = properties
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| tiled(format!("{at} has no `{name}` property")))?;
    property.value.as_i64().ok_or_else(|| tiled(format!("{at}: `{name}` is not a whole number")))
}

/// The layer named `name`, of the type `kind`.
fn find_layer<'a>(map: &'a Map, name: &str, kind: &str) -> Result<&'a Layer, MapError> {
    let layer = map
        .layers
        .iter()
        .find(|layer| layer.name == name)
        .ok_or_else(|| tiled(format!("the map has no layer `{name}`")))?;
    if layer.kind != kind {
        return Err(tiled(format!("the layer `{name}` is a {}, it must be a {kind}", layer.kind)));
    }
    Ok(layer)
}

fn tiled(message: impl Into<String>) -> MapError {
    MapError::Tiled(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 x 2 Tiled map: an original tileset of 120 blocks (firstgid 1), a block tileset of two new blocks (firstgid
    /// 121; metatile 17 everywhere, and metatile 0 everywhere), the blocks, one NPC, one zone, the start. The cells
    /// are original blocks 0, 2, the new block $78, and original block 5.
    fn map_text() -> String {
        let numbers = |n: u16| vec![n.to_string(); 256].join(", ");
        r#"{
  "type": "map", "orientation": "orthogonal", "width": 2, "height": 2,
  "tilewidth": 256, "tileheight": 256,
  "properties": [ { "name": "music_like", "type": "int", "value": 34 },
                  { "name": "palette_like", "type": "int", "value": 34 } ],
  "tilesets": [
    { "firstgid": 1, "name": "blocks", "tilecount": 120 },
    { "firstgid": 121, "name": "new", "tilecount": 2, "tiles": [
        { "id": 0, "properties": [ { "name": "metatiles", "type": "string", "value": "METATILES17" } ] },
        { "id": 1, "properties": [ { "name": "metatiles", "type": "string", "value": "METATILES0" } ] } ] }
  ],
  "layers": [
    { "type": "tilelayer", "name": "blocks", "width": 2, "height": 2, "data": [1, 3, 121, 6] },
    { "type": "objectgroup", "name": "objects", "objects": [
        { "name": "guard", "x": 300, "y": 200, "properties": [
          { "name": "object", "type": "int", "value": 5 }, { "name": "message", "type": "int", "value": 290 } ] } ] },
    { "type": "objectgroup", "name": "zones", "objects": [
        { "name": "z", "x": 256, "y": 0, "width": 256, "height": 300, "properties": [
          { "name": "level", "type": "int", "value": 7 }, { "name": "group", "type": "int", "value": 3 } ] } ] },
    { "type": "objectgroup", "name": "start", "objects": [ { "name": "start", "x": 256, "y": 128 } ] }
  ]
}"#
        .replace("METATILES17", &numbers(17))
        .replace("METATILES0", &numbers(0))
    }

    #[test]
    fn a_tiny_tiled_map_compiles_to_the_expected_pack_map() {
        let pack = parse(&map_text(), 64).unwrap();
        assert_eq!((pack.id, pack.width, pack.height), (64, 2, 2));
        assert_eq!(
            pack.grid,
            [0, 2, NEW_BLOCK_BASE, 5],
            "the cell value minus the firstgid of its tileset"
        );
        assert_eq!(pack.blocks.len(), 2);
        assert_eq!(pack.blocks[0].cells[0], 0x42, "metatile 17 is row 1, column 1: word $42");
        assert_eq!(pack.blocks[1].cells[0], 0);
        assert_eq!((pack.music_like, pack.palette_like), (34, 34));
        assert_eq!(pack.start, (256, 128));
        assert_eq!(pack.objects, [MapObject { object: 5, x: 300, y: 200, message: 290 }]);
        assert_eq!(pack.zones, [Zone { x0: 1, x1: 1, y0: 0, y1: 1, level: 7, group: 3 }]);
    }

    #[test]
    fn a_refusal_names_its_layer_cell_or_property() {
        let no_start = map_text().replace("\"name\": \"start\"", "\"name\": \"other\"");
        assert!(
            matches!(parse(&no_start, 64), Err(MapError::Tiled(text)) if text.contains("`start`"))
        );
        let empty = map_text().replace("[1, 3, 121, 6]", "[1, 0, 121, 6]");
        assert!(
            matches!(parse(&empty, 64), Err(MapError::Tiled(text)) if text.contains("cell (1, 0)"))
        );
        let flipped = map_text().replace("[1, 3, 121, 6]", "[1, 3, 2147483769, 6]");
        assert!(
            matches!(parse(&flipped, 64), Err(MapError::Tiled(text)) if text.contains("flipped"))
        );
        let no_music = map_text().replace("\"music_like\"", "\"music\"");
        assert!(
            matches!(parse(&no_music, 64), Err(MapError::Tiled(text)) if text.contains("music_like"))
        );
        let external = map_text().replace(
            "\"name\": \"blocks\", \"tilecount\": 120",
            "\"name\": \"blocks\", \"source\": \"blocks.tsj\"",
        );
        assert!(
            matches!(parse(&external, 64), Err(MapError::Tiled(text)) if text.contains("embed"))
        );
        assert!(
            matches!(parse("{ not json", 64), Err(MapError::Tiled(text)) if text.contains("Tiled JSON"))
        );
    }
}
