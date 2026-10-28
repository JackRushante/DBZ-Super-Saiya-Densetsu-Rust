# HD art in data packs

A data pack can give its own pictures in three places: the HD art of a pack map (what the Enhanced picture draws over
the map), the picture of a face (the portrait the game shows for a face index, which the game draws in its own
small size), and the picture of a battle pose, in every frame or while a condition holds (a pose variant). All are
optional. Without them a pack draws the game's own art.

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

A variant replaces the picture of one battle pose, in every frame or while a condition holds. `art` is the name of the object's
picture in the HD art (its file's stem). `when` is a condition, by name:

- `always`: every frame. The variant is drawn whenever the renderer draws the object, so a pack can give any battle pose bigger
  art (for Super Saiyan Goku, hair that stands up past the box of the original sprite).
- `kaioken`: the Kaioken of Super Saiyan Goku, on when `$0C36` bit 7 is set and a party record has unit `$3B`
  (`docs/re/data/kaioken-bp.md`, section 4).

`file` is a PNG: the object's own picture, grown by `margin` pixels on each side. The pixels are four to a game pixel, so
`margin` is a multiple of 4 (a whole game pixel), up to 96.

- While the condition holds, the variant is drawn over the box of the object's picture grown by the margin: the margin shows
  past the sprites it grows from. Its colours are its own (the palette of the pose does not recolour it); the screen's fade
  still dims it. A pixel that another object shows is not drawn over. These rules are the same for every condition.
- An object has at most one variant for each condition: one `always` and one `kaioken` variant of the same art are allowed.
  When both hold, the specific condition wins: the `kaioken` variant is drawn while the Kaioken is on, the `always` variant
  in every other frame. The order of the entries does not change this.
- The variant is drawn only where the renderer draws the object it names. When several units have the same pixels, the
  renderer draws the pose from one unit's art, and only that unit's variants apply: for Goku, the rule below; for the others
  (the three Saibamen), the unit with the most poses in the index, recoloured to the pose's palette.
- Goku: Super Saiyan Goku's poses `battle-goku-ssj-000` to `007` have the same pixels as the normal Goku's `battle-goku-000`
  to `007`. While a party record has unit `$3B` (Super Saiyan Goku), those pixels are drawn from `battle-goku-ssj-NNN`, so
  its variants (an `always` one included) are drawn, margin included. Without that unit in the party (the normal Goku,
  before the transformation), they are drawn from `battle-goku-NNN`, whatever the number of poses of each set, and no
  variant of `battle-goku-ssj-NNN` is drawn; a variant of `battle-goku-NNN` is drawn then, and not for Super Saiyan Goku.
- A file that is missing, or whose size is not the object's picture plus the margin, is not drawn (the log says why): the object
  is drawn as it is. The pack loads without its pictures.
- A pack is refused at load when a variant names no condition the game has, a margin that is not a whole game pixel or is
  over 96, or a second variant of one object for one condition.
- An object's variant for one condition is a slot, as a data entry is: two packs that give one object a variant for the same
  condition are refused together, and the set names both packs. A variant for `always` in one pack and one for `kaioken`
  of the same object in another are two slots: both are drawn, with the precedence above.
- The HD pack itself can carry variants: the shared `hd-pack.zip` has a `variants/` folder (`variants/index.json` and a picture for
  each), which the install puts in `art/variants` of the installed pack (`ssd_hd::pack::Pack::pose_art`). They are the same
  thing as a data pack's: the same margin, drawing and rules, the Goku rule included (the pack gives the 27 poses of
  Super Saiyan Goku, `battle-goku-ssj-000` to `026`, an `always` variant with the hair standing up, margin 76). The precedence
  is a data pack's before the HD pack's own: while a variant of a data pack holds for an object (`always` or `kaioken`), it is
  drawn and the HD pack's own is not; the HD pack's own is drawn in every other frame (so `ssj-kaioken` shows its Kaioken poses
  while the Kaioken is on, and the pack's hair otherwise), and when the data pack's picture cannot be drawn. An old
  `hd-pack.zip` has no variants: its poses are drawn in their box, as before. The `ssj-goku-hd` mod is redundant with the pack.
- Only the Enhanced picture draws variants; the Classic game is not changed. A `.ssdpack` zip's variants are drawn from its
  files unpacked into the cache (see "Limits").

The example pack `examples/ssj-kaioken` gives the 27 poses of the Super Saiyan Goku, `battle-goku-ssj-000` to `026`. Their
pictures are the poses redrawn with the Kaioken's aura (since 1.1.0 on the art of `examples/ssj-goku-hd`, with a
margin of 96 to hold the hair and the aura), made locally from the installed HD art: `xtask kaioken-art
examples/ssj-kaioken` copies them from `.work/m10/kk-art/variants/`. They are derived from the game, so the pack's `art/`
folder is not tracked. The variants of ids 000 to 007 are drawn, while their condition holds, for the poses that have the same
pixels as `battle-goku-000` to `007`: the Super Saiyan art draws those poses while Super Saiyan Goku is in the party. The
variants of the other ids apply where the battle draws them under their own names: `battle-goku-ssj-020` is drawn so in the
`secret-ssj-frieza` replay (frame 8375, the power-up). That replay never turns the Kaioken on: to see a Kaioken variant there,
set it with `--wram` (`$0C36` bit 7, and unit `$3B` in a party record where the frame has none).

The example pack `examples/ssj-goku-hd` gives the same 27 poses an `always` variant: the Super Saiyan Goku redrawn with golden hair
standing up in tall spikes, which rise past the box of the original sprite (`margin = 76`, the hair of the tallest pose rises 75
art pixels). Its pictures are made locally from the installed HD art, one picture per pose, each the pose's own picture grown by the
margin with the body where the installed art draws it, so the pose does not move; the pack's `art/` folder is not tracked.

## Limits

- The HD art of a pack given as a `.ssdpack` zip (map tiles, face pictures, pose variants) is unpacked once into
  `cache\mods\` of the data folder, one folder per pack id, version and zip size, and the Enhanced picture reads it from
  there. The SNES art (faces and everything else) is read from the zip.
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
