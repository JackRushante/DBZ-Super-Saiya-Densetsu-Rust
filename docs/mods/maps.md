# Maps in data packs

A data pack adds a field map: a room, a town of its own, a cave, with its own size, its own block grid, its own
objects, encounter zones, music and palette. A pack map takes an id from 64 to 127. Its art is the game's own field
art: the tile set, the metatile table and the walk attributes of the field stay as they are, so a pack map is built
from the blocks the game already has, plus new block definitions made of the game's existing metatiles.

## The manifest

```toml
[[map]]
id = 64
file = "maps/room.tmj"
```

The file is a Tiled map in JSON (`.tmj`). Tiled's XML format is not read. A pack may name several maps, one id
each; two packs that name the same id are refused.

## The Tiled map

The map is orthogonal, with 256 x 256 pixel tiles: one tile is one block. `width` and `height` are the size of the
map in blocks. The tilesets are embedded in the map file (Tiled: *Embed Tilesets*); a tileset named by `source` is
refused.

Map properties (integers, the original maps `0` to `47`):

| Property | Meaning |
|---|---|
| `music_like` | the original map whose music the pack map plays |
| `palette_like` | the original map whose palette indexes and background palette the pack map uses |

Layers, by name:

| Layer | Type | Content |
|---|---|---|
| `blocks` | tile layer, the size of the map | the block of each cell |
| `objects` | object layer | one point per object (an NPC) |
| `zones` | object layer | one rectangle per encounter zone |
| `start` | object layer | exactly one point: the start, in pixels |

Positions of objects and of the start are whole numbers of pixels, inside the map. The map is at most `128 x 128`
blocks.

### Blocks

The blocks of the original are a tileset of 120 tiles (`$00` to `$77`): tile `t` is block `t`. The tool
`cargo run --release -p xtask -- mod-tileset <folder>` writes one, `blocks.tsj` with `blocks.png`, rendered from the
game's own art, so that an author can paint with the blocks. The folder must be under the repository's `.work/`
directory or outside the repository; the tool refuses any other place. The picture is only an aid to painting:
its colours follow the background palette of map 0, and the palettes the tiles name other than 2 to 5 are an
approximation of the game's.

A new block is a tile of a second tileset, one that holds the new blocks only, whose tiles each carry a property
`metatiles`: a string of 256 numbers, the metatiles of the block row by row (16 numbers a row). A metatile number
`n` is the metatile in row `n / 16` and column `n % 16` of the game's metatile table, so `0` to `4095` are
addressable and the game's own metatiles are the ones that exist. The tiles of that tileset are numbered `0, 1, 2`
in order; tile `t` is block `$78 + t`. A map has at most one such tileset and at most 136 new blocks (`$78` to
`$FF`).

A block is not a picture: the walls and the floor of a pack map come from the metatiles the block uses, as they
do in the game's own blocks. A metatile that is a wall in the game is a wall in a pack map.

### Objects

Each point of `objects` has two integer properties:

| Property | Meaning |
|---|---|
| `object` | the original's object record, `0` to `71`; the record gives the object's sprite and its kind |
| `message` | the message id of the original (`0` to `65535`), recorded in the pack (see below) |

A map has at most 16 objects. An object is always shown: the hide flags of the original are not read in this
version.

### Zones

Each rectangle of `zones` gives the blocks it covers, from its pixels: the zone is the blocks whose pixels the
rectangle touches. It has two integer properties:

| Property | Meaning |
|---|---|
| `level` | the encounter level, `0` to `255`; `0` gives no battles in the zone |
| `group` | the original's encounter group, `0` to `111`: the battle is that group |

A map has at most 255 zones. The zones are tested in the order the layer gives them, and the first rectangle that
holds the camera's block decides. A map with no zone has no random battles. A zone whose level is `0` gives no
battles. While the party flies the level is one more, and one more again when the game's bit 6 of the same flags is
set, as in the original.

## Limits

| What | Limit |
|---|---|
| id | `64` to `127` |
| size | `1` to `128` blocks a side; the grid holds width times height block ids |
| original blocks | `$00` to `$77` |
| new blocks | `$78` to `$FF` (at most 136), each 512 bytes; the grid and the new blocks of a map come to at most 32 768 bytes |
| metatile numbers | `0` to `4095` |
| objects | at most 16 (the object loader has 16 slots) |
| zones | at most 255 |
| music_like, palette_like | `0` to `47` |

A refused map names its file, the layer, the cell, the object or the value, and the pack is not loaded.

## What a pack map changes and what it does not

A pack map may change:

- its size, its block grid and its new block definitions;
- its music and its palette: they are those of the original maps it names;
- its start (the position the game's mode 6 init gives it: the position after a story transition; a Continue or a
  `go to map` keeps the position it has);
- its objects (where they stand, which original object they are, and the message they speak: see `triggers.md`);
- the scenes a place starts when the party reaches it (see `triggers.md`);
- its encounter zones and their levels and groups.

A pack map does not change, in this version:

- the tile set and the metatile table of the field: there is no new tile art and no new metatile;
- the walk attributes: a wall or a floor is a metatile of the game's;
- the entrances and the landing spots: a pack map has none, so no place leads to it or from it;
- the hide flags of the objects: they are not read in this version;
- the battle groups of the original: a zone names one of them, it does not add one.

## Checks

`cargo run --release -p xtask -- mod-check <pack>` checks a pack without a game: the map's refusals are reported at
once. The game runs the same checks at load and refuses the pack as a whole.
