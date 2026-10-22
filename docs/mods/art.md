# HD art in data packs

A data pack can give its own pictures in three places: the HD art of a pack map (what the Enhanced picture draws over
the map), the picture of a face (the portrait the game shows for a face index, which the game draws in its own
small size), and the picture of a battle pose while a condition holds (a pose variant). All are optional. Without them
a pack draws the game's own art.

Packs are data: nothing here is code. The Enhanced picture reads the HD files at 4x the game's size. The game itself
reads only the SNES art, which is derived from the HD picture of a face when the pack is built.

## Pack map art: `[[map_art]]`

```toml
[[map_art]]
map = 64
tiles = "art/hd/maps/64/t{x}_{y}.png"
```

The map is the pack map `map` (its `[[map]]` entry). The HD picture of the map is cut into tiles, one tile per block
of 256 x 256 game pixels: `{x}` is the tile's column, `{y}` its row. A tile is a PNG of 1024 x 1024 pixels (4x). A map
of 8 x 6 blocks needs 48 files, `t0_0.png` to `t7_5.png`. Each file must exist and be that size; a map art entry
whose map is not in the pack is refused.

The art is found by the map's layout (its size, its grid and the tile set), so a tile is drawn where the game draws
the map. The Enhanced picture reads the original blocks only: while the camera shows a pack block definition (a block
id from `$78`), that frame is drawn without the map art. Keep such blocks outside the camera's view until the picture
reads them.

## Faces: `[[face]]`

```toml
[[face]]
id = 0
hd = "art/hd/faces/goku.png"
```

The face `id` (0 to 255) is the index the party record and the corner portraits use, and the one a message's
`{portrait N}` names. The HD picture is a PNG of 128 x 128 pixels (a face is 32 x 32 game pixels at 4x).

The game's portrait is 16 tiles of 4 bits, in a 4 x 4 grid, shown in a 32 x 32 box. The image derives those 16 tiles
from the HD picture, in the palette row the game draws the face in (the colour row 6 of the field menu, which is the
palette of object 28). The derived tiles are written into the face's record in both of the record's two pictures, so
the picture the game shows is the face. The record is an LZ stream of 32 tiles, the same stream format as the game's
own portraits (`docs/re/data/menus.md`, "Corner faces").

Two cells of the picture that derive the same 8 x 8 tile show the HD art of the first of them: the art is found by the
tile, not by the cell. Paint such cells alike.

## Pose variants: `[[pose_variant]]`

```toml
[[pose_variant]]
art = "battle-goku-ssj-000"
when = "kaioken"
file = "art/hd/variants/battle-goku-ssj-000.png"
margin = 24
```

A variant replaces the picture of one battle pose while a condition holds. `art` is the name of the object's picture in the
HD art (its file's stem). `when` is a condition the game has, by name: `kaioken` is the Kaioken of Super Saiyan Goku, on when
`$0C36` bit 7 is set and a party record has unit `$3B` (`docs/re/data/kaioken-bp.md`, section 4). `file` is a PNG: the
object's own picture, grown by `margin` pixels on each side. The pixels are four to a game pixel, so `margin` is a multiple of
4 (a whole game pixel), up to 96.

- While the condition holds, the variant is drawn over the box of the object's picture grown by the margin: the margin shows
  past the sprites it grows from. Its colours are its own (the palette of the pose does not recolour it); the screen's fade
  still dims it. A pixel that another object shows is not drawn over.
- The variant is drawn only where the renderer draws the object it names. When another unit has the same pixels (for Goku,
  `battle-goku-000` for `battle-goku-ssj-000`), the renderer draws the unit with the most poses in the index, recoloured to
  the pose's palette, and the variant of the other unit does not apply. While Super Saiyan Goku is in the party (a party
  record has unit `$3B`), the renderer draws the Super Saiyan art for those pixels, where the index has it.
- A file that is missing, or whose size is not the object's picture plus the margin, is not drawn (the log says why): the object
  is drawn as it is. The pack loads without its pictures.
- A pack is refused at load when a variant names no condition the game has, a margin that is not a whole game pixel or is
  over 96, or a second variant of one object for one condition.
- Only the Enhanced picture draws variants; the Classic game is not changed. A `.ssdpack` zip's variants are not drawn (as the
  zip's HD art is not), so unzip the pack into a folder.

The example pack `examples/ssj-kaioken` gives the 27 poses of the Super Saiyan Goku, `battle-goku-ssj-000` to `026`. Their
pictures are the poses redrawn with the Kaioken's aura, made locally from the installed HD art: `xtask kaioken-art
examples/ssj-kaioken` copies them from `.work/m10/kk-art/variants/`. They are derived from the game, so the pack's `art/`
folder is not tracked. The variants of ids 000 to 007 are drawn, while their condition holds, for the poses that have the same
pixels as `battle-goku-000` to `007`: the Super Saiyan art draws those poses while Super Saiyan Goku is in the party. The
variants of the other ids apply where the battle draws them under their own names: `battle-goku-ssj-020` is drawn so in the
`secret-ssj-frieza` replay (frame 8375, with the Kaioken on).

## Limits

- The HD art of a pack given as a `.ssdpack` zip is not drawn: unzip the pack into a folder to draw it. The SNES art
  (faces and everything else) works from the zip.
- A face is derived when the image is built. A picture that is not a PNG, or not 128 x 128 pixels, is refused with the
  pack and file named.
- Two packs cannot give the same face index (the same rule as every other slot).

## Making the tiles: `xtask art-derive`

```
cargo run --release -p xtask -- art-derive <hd.png> <face|tiles> <out dir> [--palette <file>]
```

The command derives the SNES art of one HD picture and writes four files into `<out dir>`:

| File | Contents |
|---|---|
| `palette.toml` | the 16 SNES colours (`words`, 15-bit `0BBBBBGGGGGRRRRR`); word 0 is transparent |
| `tiles.bin` | the 4-bit tiles, 32 bytes each, in the layout the game holds in video memory |
| `tiles.lz` | the stream the game decodes: the tiles of a sheet, or a face's record (32 tiles) |
| `map.toml` | the cells row by row as `[tile, h flip, v flip]`, with `columns` and `rows` |

- `face` takes a 128 x 128 picture and derives its 16 tiles. Without `--palette` the palette is chosen from the
  picture; a pack uses the game's row (see above), so `--palette` takes a palette file of the same form as
  `palette.toml`.
- `tiles` takes a map sheet: a multiple of 32 pixels each way (8 game pixels at 4x). Each distinct tile is kept once,
  and a cell that is another tile flipped shows it with the flip.

The derivation: a game pixel is the alpha-weighted average of its 4 x 4 block; a block whose alpha averages under 50 %
is transparent. The colours are reduced to the 15-bit SNES colours, and each pixel takes the nearest colour of the
palette. A palette without `--palette` is chosen by median cut on the picture's colours (at most 15, deterministic).

Budgets: a face is exactly 16 tiles. A sheet may hold at most 432 distinct tiles, the field's tile set; a picture that
needs more is refused with the number to merge.

## Where the files go

The folder `<out dir>` must not be inside the repository unless it is under `.work/`: the command refuses it.
