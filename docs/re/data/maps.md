# Research item 4: maps (read-only notes, scratch)

Scope: the plan `2026-10-16-m10-mods.md`, research item 4 (map ids, header table, object lists, encounter
zones, music and palette, tile sets, walk rules, set-up and trigger dispatch, exits and entrances, size limits,
block definitions, compression). Written for the people who build Tasks 2, 5 and 6.

Tags: **SEEN** = code read at the given address, or a headless run (the command is named); **INFERRED** = read
from code or data but not exercised, or a conclusion drawn from several SEEN facts. Quotes of the original code
are avoided (at most a few words). Everything here is derived from the ROM: it stays in `.work/`.

Tools used: `target/release/xtask.exe disasm`, `shots --peek / --watch / --watch-from`, `probe`, `save-edit`,
`replay-from-save`, plus small Python readers of the cached ROM (`.work/m10-research/r456/*.py`, scratch).
Address notation `bank:address` is physical LoROM (`06:F2DE` = bank 6, offset `$F2DE`).

## 0. Result in ten lines

1. **Maps 48 to 63 exist as headers, but they are not field maps**: they are the 16 one-screen "town" maps (SEEN),
   entered through `landing_spot` with `$01A1` bit 7 set; their id is computed as `$30 + type` by `00:8D2B`. Using
   an id of 48 or more through the ordinary path makes the map load hang (SEEN for 64; 48 not completing in 100 s).
2. The header table `06:F2DE` has exactly **64 words** (ids 0 to 63); there is no room to grow it in place, and the
   header record's bank is forced to 6 by `00:8E26`. A pack map id must therefore be **64 or more, with `00:8E26`
   wrapped** (the 24-bit grid pointer is then free to point into the extended image). Ids up to 127 fit the
   original's 8-bit arithmetic; 128 and up would wrap in `ASL A` (SEEN).
3. **There is one tile set and one metatile table for all field maps** (`08:8D4C` LZ, 432 tiles, and `09:EFB0`),
   and one pair for the town screens (`08:BF20` and `08:FA00`). Both pairs are loaded by constants in code
   (`00:8C25`..`8C2D` and `00:8C5C`..`8C61`). A map differs from another by its block grid, its block definitions,
   its palette indexes and its music byte only. **A pack map with its own art needs a wrapper on the tile-set load
   and on `$0707-$0709`.**
4. **Collision is not in the map data.** It is a byte table indexed by the 8x8 *tile quarter of the metatile*
   (`06:E035`, `$540` bytes for the field; `06:E575` for towns). A Tiled `collision` layer can only be compiled
   into these bytes through the metatiles it uses: a new metatile needs new attribute bytes, i.e. a wrapper on
   the probe `01:8485` for pack maps.
5. **No map data is compressed**: header records, block grids, block definitions, metatile tables, attribute tables,
   object lists, palettes are all raw ROM bytes read through 24-bit pointers. Only the tile sets are LZ (`00:C559`).
   The encoder needed is the LZ one of `graphics.md` (decoder `ssd_catalogue::graphics::lz_decode`).
6. **Per-map tables stop at id 47** (objects, zones, levels, groups, palette index, music byte) or earlier
   (set-up: 32 maps; walkers: 32 maps), the dispatch of triggers at 50 words. Every one of them needs a redirector
   for a pack id (table in section 2).
7. The object (NPC) loader keeps **16 objects per map** (the work area is `$1180`, 16 x 7 bytes, no bounds check) and the
   hide flag of an object can name flags 0 to 31 only (SEEN: the low 5 bits).
8. `go to map` (script command `2C`) is only used twice in the whole ROM (scene `$34`: map `$10` at (1792, 768),
   and map `$0B` at (768, 424); SEEN with `tools/re/scenes.py`). It sets the map id and four position words, then
   requests the standard map reload; **it does not run the per-map set-up** (`03:EA1E`).
9. The per-frame trigger dispatch `03:F163` returns at once while the party **flies** (`$01BC` bit 0), except on map
   `$10`: a point of interest on the world map (map 0) cannot be a per-map trigger in flight; use the entity
   (landing) mechanism, or a wrapper that runs before the original's gate.
10. Hook points are in section 11 (address, widths, return kind). All the field-load routines below are `RTL`,
    entered in M8 X16.

## 1. Map ids and the header table

SEEN (`00:8E26`, one routine, 43 lines): the map id is `$01A0`. For ids below `$30` it also copies the byte
`02:E41B[id]` to `$01E7` (the "copy select" byte: music and place flags, section 7). It then doubles the id
(8-bit `ASL`, so ids of 128 and more would wrap), reads the word at `06:F2DE + 2 * id` (always bank 6), and
stores: `$0704-$0705` = the word + 2 (the pointer of the block grid), `$0706` = 6 (its bank), `$0703` = byte 0
of the record (**width in blocks**), `$0702` = byte 1 (**height in blocks**).
This routine is the only writer of `$0702-$0706` (SEEN: a scan of the ROM for stores to those addresses finds only
`00:8E5A..` here; two stores in bank `$1B` are another variable).

Run, `xtask shots wander --frames 3300 --peek map_id,7E0702,...`: map 0 gives `$0702 = $0B`, `$0703 = 9`,
`$0704-$0706 = 06:F360` (record `F35E` + 2), `$0707-$0709 = 09:EFB0` (SEEN).

Record format (SEEN, `06:F2DE..F876`): width (1 byte), height (1 byte), then `width * height` block ids, one byte
each, row by row (`grid[y * width + x]`, from the multiply at `06:F096..F0A8`). Table words 0 to 63; records
follow at `06:F35E` and **identical maps share one record** (read with `.work/m10-research/r456/t3.py`):

| ids | size (w x h blocks) | notes |
|---|---|---|
| 0 | 9 x 11 | first map |
| 1, 6, 7 | 1 x 1 | one record (`F45C`, block `$4F`) |
| 2 | 5 x 5 | |
| 3 | 1 x 2 | |
| 4, 5 | 12 x 10 | one record |
| 8 | 6 x 7 | |
| 9, 11, 16, 20-25 | 12 x 12 | 9 and 11 own records; 16, 24, 25 share one; 20-23 share one |
| 10 | 7 x 8 | |
| 12, 26 | 8 x 6 | (two records) |
| 13, 27 | 7 x 6 | (two records) |
| 14, 15, 28-31 | 6 x 7 | 14/15 share, 28-31 share |
| 17 | 4 x 7 | |
| 18, 19 | 1 x 2 | share |
| 32 to 47 | 1 x 1 | **one record**, block `$4E`: sixteen identical empty caves |
| 48 to 63 | 1 x 1 | sixteen distinct records, blocks `$68` to `$77`: the town screens |

**Ids 48 to 63** (SEEN, `shop.ssdreplay`, `xtask shots .work/replays/shop.ssdreplay --upto 20000 --watch ...`):
at frame 3684 the party touches an entrance entity (`game_mode` 1, `$0716 = $A1`), at 3718 `$01A1` becomes `$80`,
at 3719 `map_id` becomes `$3A` (town type 10 + `$30`). `00:8D2B` computes it as `02:E3FB[ 02:E629[spot * 5] & $1F ] + $30`
(SEEN). While `$01A1` bit 7 is set the loader uses the town tile set `08:BF20`, the town metatile table `08:FA00`
(peek: `$0707-$0709 = 08:FA00` at frame 4000, header 1 x 1), the town object tables and the town attribute table.
**Above 63** there is no header: the table is followed by the first record (`06:F35E`).

Evidence that out-of-table ids hang: `save-edit ... --set page:A0=40` (map 64) then `replay-from-save status.txt` and
`xtask shots m64.ssdreplay --frames 2330`: frame 2100 prints (`map_id=40`), frame 2330 (after the load) does not
finish within 60 s (a normal run of the same length takes 5 s). SEEN. For id 64 the header word read is `$0B09`
(the first bytes of the first record), a bank-6 offset below `$8000`, which reads work RAM: INFERRED cause. Id 48
(a valid header) also did not finish within 100 s: the reason is another table (INFERRED: the town screens need
the landing state).

## 2. Per-map tables and where each one stops (SEEN: addresses read from the code; sizes from the data)

| What | Address | Index | Entries | Used by |
|---|---|---|---|---|
| header words | `06:F2DE` | id | 64 | `00:8E26` |
| block definition table | `06:F1EE` | block id | 120 (`$00-$77`, ends at `F2DE`) | `06:F089` |
| palette set, background rows 2 to 5 (`$80` bytes each) | `06:F876` | id | 64 (+5 odd words) | `00:88AE` (palette pointer `3` bytes: 24-bit) |
| sprite palette sets (`$100`/`$80` bytes) | `06:F945` | `$01D2`, `$01D3` | 24 | `00:8921`, via `00:88D8` |
| palette indexes per map | `02:8484` | id * 2 (bytes: `$01D2`, `$01D3`) | 48 (then it runs into the landing table) | `00:877E` |
| palette index per landing spot | `02:84E4` | `landing_spot` | 160 | `00:877E` (town) |
| portrait/face palette nibble | `02:8584` | id >> 1, nibble by parity | 64 | `00:86C8` into `$0D85` |
| music/place byte `$01E7` | `02:E41B` | id | 48 (code guards `CMP #$30`) | `00:8E26` |
| object lists (NPCs) | `02:EAAC` | id | 48 words | `03:93FA` |
| object records (5 bytes) | `02:EB67` | object id | 72 (`$00-$47`) | `03:93FA` |
| object hide flag | `02:ECCF` | object id | 72 bytes, `$FF` = none | `03:93D0` |
| town object lists | `02:ED17` | `landing_spot` | 160 words | `03:9336` |
| town object records (3 bytes) | `02:EEA2` | town object id | 43 | `03:9336` |
| town object hide flag | `02:EF23` | town object id | 43 | `03:93D0` |
| walkers (ambient sprites) lists | `02:EF8A` | id | 32 words (code guards `CMP #$20`) | `03:99B3` |
| walker records (4 bytes) | `02:EF4E` | walker id | 15 | `03:99B3` |
| encounter zone rectangles | `02:D4A4` | id | 48 words | `02:D38B` |
| encounter levels | `02:D62A` | id * 4 + zone | 192 bytes | `02:D38B` |
| encounter group lists | `02:D6EA` | id | 48 words (maps 28 and up point at code bytes, never read: level 0) | `02:D34A` |
| set-up routines | `03:EA55` | id | **32** words (inline table after a `JSR 03:8000`) | `03:EA1E` |
| trigger routines | `03:F1AE` | id | 50 words (ids 0-47 and the two town slots) | `03:F163` |
| entrance entity lists | `02:E4C8` | id | 51 words | `03:92CF` |
| default start position (mode 6) | `06:FCDF` | id * 4 (x word, y word) | ids 0-31 valid, 32 and up `$FFFF` | `00:933A` |
| landing record (5 bytes) | `02:E629` | `landing_spot` | 160 | `00:8D2B`, `00:9251` |
| parent map of a spot | `06:F98D` | `landing_spot` | 160 | mode 5 exit, `00:9251` |
| town type to map id | `02:E3FB` | type | 30 | `00:8D2B` |
| walk attribute, field | `06:E035` | see section 5 | `$540` bytes | `01:8485` |
| walk attribute, town | `06:E575` | see section 5 | ~`$2B0` bytes | `01:8485` |
| field metatile table | `09:EFB0` (constant in code) | | 303 words used, max index `$508` | `06:F089` |
| town metatile table | `08:FA00` (constant) | | 162 used, max `$288` | `06:F089` |

All the "words" tables hold 16-bit addresses in **bank 2 (or bank 3 for set-up and trigger code)**: a redirector for a
pack id must supply the list itself (the original cannot address another bank through them, because the bank is set
by code from a constant: `$02` is stored as the bank of the list pointer in `02:D38B`, `03:93FA`, `03:92CF`...).
The records themselves (grid, block definitions, metatile table, palettes) are read through 24-bit pointers and may lie
in the extended image without any code change (SEEN: `LDA [$04]`, `LDA [$07],Y`, `00:88EA` copies a 3-byte pointer).

## 3. Block definitions, metatiles, cells (SEEN, `06:F089`, 83 instructions, called by `01:8442`, `01:8485`,
`06:EF70`, `06:F046`)

- Input: world pixel x in `$00-$01`, y in `$02-$03`. A pixel is outside the map (result 0) when the high byte of x or y
  is negative (bit 7) or equals the width or height. **So the original cannot address more than 128 blocks a side;
  the grid index `y * width + x` is a 16-bit product, so `width * height` stays under 65 536.**
- Block id = `grid[...]`; the definition address is `(bank << 16) | (mid << 8)` from the word at `06:F1EE + 2 * id`
  (high byte = bank, low byte = middle byte; the low byte of the address is 0): **definitions are `$200`-byte aligned
  to 256 and may be in any bank, in particular in the extended image.**
  The word table gives 120 ids (`$00-$77`); the definitions live in banks `$0B` and `$0C` (`0B:8000` to `0C:CA00`).
  Nineteen ids point at the same definition as block 0 (`0B:8000`, which holds a few repeated ground metatiles; ids
  `0, 3, 6, 8, 10, 20, 26, 45, 50, 53, 56, 61, 68, 72, 73, 82, 84, 98, 101`, SEEN with `t14.py`); of the 120 ids
  **23 are referenced by no map** (`3, 6, 8, 10, 19, 20, 26, 45, 50, 53, 56, 58, 61, 63, 68, 72, 73, 82, 84, 94, 98,
  101, 103`, SEEN), 18 of them aliases of block 0. 97 ids are in use. Because a table entry is a full bank and
  middle byte, an unused id can be repointed at a definition in the extended image by patching `06:F1EE` in the
  in-memory image (data, no code), which gives a pack up to 23 new block ids without wrapping `06:F089`.
- Cell: `cell = (y & $F0) | ((x & $FF) >> 4)` (a 16 x 16 grid of 16-pixel cells), word at `definition + 2 * cell`. The word is
  masked with `$3FDE` (clears bits 0, 5, 14, 15) and saved at `$0736`. Bits 0, 5, 14, 15 are **zero in every cell of the
  120 definitions** (SEEN, 30 720 cells), so they are free in the data but the code throws them away: no flag
  lives in a cell word.
- Metatile address = `$0707-$0709` + `(masked word) * 2`; layout: 16 metatiles per row, each 2 words wide, 2 rows
  (row stride `$40` bytes), so a metatile row is `$80` bytes; the quarter is picked by bit 3 of x (word) and bit 3 of y
  (row). The result word is `$04/$05`, high byte first (the tile map word: tile, palette, flip, priority).
  Up to `$3FDE >> 1` = 4095 metatiles are addressable; the VRAM tile budget (432) is the real limit.
- `ssd_hd::world::FieldMap::tile_word` ports exactly this (SEEN: `crates/ssd-hd/src/world/map.rs`), reading through a
  `Rom(&[u8])` of any length, so a 2 MiB image works unchanged on the HD side.

## 4. Tile sets and metatile tables (SEEN, `00:8B7A` and `00:8C0D..8C80`)

`00:8B7A` is the init of game mode 0 (the map load; mode table `00:8B54`, entry 0). After a common prelude
(`03:A6CB`, `00:8887`, `06:E837`, `03:9255`...) it branches on `$01A1`:

- bit 7 clear (fields, caves): `02:D37D`, `03:99B3` (walkers), `03:93FA` (objects), `06:EC12`; sets `$0707-$0709` to
  **`09:EFB0`**; tile set pointer **`08:8D4C`** (LZ header size `$3600` = 432 tiles of 4 bits).
- bit 7 set (town screens): `02:D97D` (default party spot), `03:9336` (town objects), `$01A0 = $30 + type`
  (`00:8D2B`); `$0707-$0709 = 08:FA00`; tile set **`08:BF20`** (`$1B20` = 216 tiles).
- then both go on at `00:8C6D`: `00:C559` (LZ decode) + `00:C68C` (bitplane regroup) to VRAM `$3000` (direct DMA in forced
  blank), faces (`00:8DAC`), palette indexes (`00:877E`), `03:91FC`, **`03:F7A4`** (the map-entry story hook), header
  re-read and palettes (`00:8DFA`: `00:8E26`, `00:88AE`, `00:8921`, copy of the bank-9 block, `00:90C1` = UI tile set,
  `00:86C8`, `06:F14D` = the first 35 columns), item attributes (`01:C535`), music (`00:8D4E`), `03:D2F2`, `02:D844`
  (saves the leader and camera positions in `$01C0..$01CE`), `copy_source_block` (`06:EBC1`).
- The same two constant pairs are repeated in the bank-4 story transitions (`04:9BEE..9C00`), which also call
  `00:8DFA`.
- There is **no per-map tile-set table**. (`00:8844` named in the plan is the generic scene tile loader that indexes a
  table at `02:8470` for ids `$26` and up; it is not the field's.)

The tile set and metatile table being global, a map with new art needs: its own LZ tile stream (an encoder: section 10),
a metatile table, an attribute table (section 5), and wrappers: (a) on the load of the tile set (replace the pointer
in `$83-$85` at `00:8C6D`, or wrap `00:C559` for that call site), (b) on `$0707-$0709` (they are set before
`00:8E26` runs the last time through `00:8DFA`, so a wrapper on `00:8E26` can write them), (c) on `01:8485`.

## 5. How the walk code decides what is solid (SEEN code, partial runs)

`01:8485` (RTL, M8 X16, 6 callers inside the direction probes at `01:8627..86D3`) is the tile probe. It calls `06:F089`
for a probe point, stores the block id in `$0D51`, then reads **one byte of an attribute table** and answers with the
carry flag (carry set = blocked):

```
index = $0736 (the masked cell word) + (x bit 3: +1) + (y bit 3: +$20)     [x, y = the probe point]
attr  = 06:E035[index]   when $01A1 bit 7 is clear (field)
        06:E575[index]   when it is set (town)
```

The table is therefore a 32-column array of the metatile *quarters* (16 metatiles x 2), 2 quarter-rows per metatile row.
The field table has `$540` bytes (21 metatile rows); the field's blocks reach index `$508 + $21`. Value histogram of the
field table (SEEN): `00` x433, `01` x493, `02` x374, `10` x12, `40` x16, `80`..`85` x16.

Meaning (code of `01:84BE..8558`, SEEN; the names of the states are mine):

| value | result |
|---|---|
| `00` | free |
| `01` | wall; passable when `$01BC & $0F` is nonzero (the party flies) |
| `02` | wall; also when flying (and marks `$01BC |= $20` when flying) |
| `10` | exit tile: when `$01BC` bit 0 is clear it plays music `$84`, sets `game_mode = 5`, `$0716 = $A1`, `field_state = 3` (leave the place); blocked when flying |
| `40`..`7F` (bit 6) | blocked on foot (field); in a town the low nibble selects an event: 0 scene `$1B`, 1 nothing (blocked), 2 scene `$2B`, 3 scene `$45`, 4 blocked, 5 scene `$4D` (the door of a building) |
| `80`..`8F` (bit 7) | a partial block: the low nibble (0 to 5) selects one of six shape routines (jump table `01:857B`) that look at the probe's position inside the 8x8 tile |

`$0D88` keeps the attribute for other code. A second routine, `01:842E`, reads the table at `01:86D8` (a byte per
quarter: `01` = shallow water or a footstep effect, sets `$01E7` bit 2) — INFERRED meaning, only the table is SEEN.

**Consequence for Tiled** (Task 5): a `collision` layer cannot be compiled per cell. It is a property of the metatile quarters
used; compile it by giving each metatile of the pack's metatile table its four attribute bytes (a pack
`attributes` array per metatile), and supply them through a wrapper on `01:8485` (attribute table of the pack map).
The `$FFx` shape values (80 to 85) can be reused as they are.

## 6. Objects, entrances, walkers, triggers on a map

**NPC objects** (SEEN, `03:93FA`, called from the map load and from `03:CE56`, `03:D116`, `03:F36C`):
clears `$1180` (`$70` bytes), reads the list pointer from `02:EAAC + 2 * id`, then for each id until `$FF`: calls `03:93D0`
(hide test: the flag number is `02:ECCF[id]`, `$FF` none; the flag helper `03:9B7F` keeps only the **low 5 bits**, so flags
0 to 31 only; carry set = the object is hidden), else writes a 7-byte slot at `$1180 + 7n`: `+0` = `$80 | type`
(type from the record), `+1` = `id + $30`, `+2` x word, `+4` y word, `+6` = 2 when the record's bit 7 is set. The record
(5 bytes at `02:EB67 + 5 * id`): byte 0 = `bit 7 | type (7 bits)`, x word, y word. The loop **has no bound**: 16 slots
(`$70 / 7`) is the limit; a 17th would overwrite `$11F0..`. The most any original map lists is 14 (map 9).
**Town objects** use `02:ED17[spot]` (list), `02:EEA2 + 3 * id` (x byte, y byte, flags) and `02:EF23` (hide flag).
The message an object speaks comes from its talk program (`03:9DF1`, one word per object id), see `scenes.md`; I did not re-read
those programs (not verified here).

**Entrances (the landing spots)** are entities in the `$1000` table (6 bytes each, up to 32): `03:92CF` fills `+0 = $80`,
`+1 = id` from `02:E4C8[map]` (ids up to `$E3`); the position `+2/+4` and the type in the low bits of `+0` equal the landing record
`02:E629[spot]` (SEEN: entity 0 on map 0 at frame 3300 is `84 00 8C 08 74 0A`, the record of spot 0 is type 4,
x `$088C`, y `$0A74`; INFERRED: the entity is built from the record, I did not read the writer). The touch test is `03:967E` (RTL; returns at once when `$01BC` bit 0 is set, i.e. while flying):
a hit sets `landing_spot` (`$01BD`) = the entity id, `game_mode = 1`, `field_state = 3`, `$0716 = $A1` (`03:970A..9728`;
special cases: map `$0B` with type 5 starts scene `$2C`; entity types `$19`/`$1A` (under a condition on the walking
direction `$0D59`) set `$01A0` to `$15`/`$14` directly through `03:973D`/`9741`, which also set `field_state 3`,
`game_mode 0`, `$0716 = $A1`: a second "go to map" by entity; INFERRED reading, not run).

**Walkers** (the ambient sprites of maps 0 to 31): `03:99B3`, table `02:EF8A`, 4-byte records `02:EF4E`.

**Triggers (hand-written per map)**: `03:F163` (RTL, called every frame from `00:97A4`): every 64 frames it also runs
`02:D37D`; returns when `$0716 != 0`, when `game_mode` bit 7 is set, or in flight (`$01BC` bit 0) except for map `$10`; in a
town (`$01A1` bit 7) only maps `$33` and `$34` dispatch (to entries `$30`/`$31`); otherwise `JMP (03:F1AE + 2 * id)` with no bound.
Maps 25 to 33 and 35 to 47 point at a bare `RTL`. A trigger starts a scene with `03:F7DE` (A = scene id: stores `$01D8`
and `$1648 = $80`; 18 callers). The doc `scenes.md` calls `03:F7A4` "the per-frame check": it is **not** per frame; its only caller
is `00:8CA8`, in the map load (SEEN: `t5.py`). It dispatches a few map-entry events: town spots 4, 9, `$32`; otherwise
`03:FB07` for maps `$0B`, `$10`, 5 (`03:FA95`).
`03:EA1E` (the chapter set-up) is called **only** from `00:939B`, the end of game mode 6 (SEEN: `t5.py`; and
`xtask probe --address 03EA1E` over `secret-raditz` (a Continue from a crafted save), `secret-nail-porunga` and `shop`:
0, 1 and 1 calls). **Continue never runs it** (see `r6-title-chapters.md`); `go to map` does not run it either.
The set-up table `03:EA55` has 32 entries only (maps 0 to 31); the code after it is a routine, so an id of 32 or more would
jump to garbage (not exercised: no path calls it with such an id).

## 7. Encounter zones, music, palette (SEEN code; data dumped with `t10.py`, `t9.py`)

**Zones** (`02:D38B`, RTL, called from `00:8C12` and the map's random-battle roll). Only for ids below `$30`.
List `02:D4A4[id]`: records of 5 bytes `x0, x1, y0, y1, zone` ended by `$FF`. The test compares the **camera's block
coordinates** (`$34` = camera x high byte, `$36` = camera y high byte, so the camera, not the leader): `x0 <= cx <= x1` and
`y0 <= cy <= y1`; the first match stores `zone` in `$0D8B` and then `encounter_level = 02:D62A[id * 4 + zone]` (+1 when
`$01BC` bit 0, +1 more when bit 6). Zone 0 or level 0 = no battles. The group is chosen by `02:D34A` (RTL):
`02:D6EA[id]` points at a list of 8-id rows; `group = row[(zone - 1) * 8 + (rng & 7)]`. Maps 25 to 47 have the single rectangle
(0, 2, 0, 2, zone 0). `02:D428` (run after the load) additionally zeroes the zone and level for ids `$20` and up and
for the special maps 0, 2, 5, `$18` (hand-coded): **a pack map with id 64 or more gets no encounters unless
`02:D38B`, `02:D34A` and `02:D428` are wrapped** (SEEN: the constants are in code).

**Music** (SEEN, `00:8D4E`, `04:85A1`): the song byte is derived from `$01E7`: bit 7 -> `$07`; else bit 4 -> `$05`; else bit 3 ->
`$06`; else bit 1 -> `$09`; else bit 0 -> `$0C`; else `$03` (priority in that order; code read; the values for map 0 and
map `$10` were confirmed: song `03` written at frame 3197 of `wander`, `0C` at 2210 of `secret-nail-porunga` when `$01E7 = 1`).
In a town the song is `$04` (`$01E7` bit 0 clear) or `$0A`. The byte of the map is `02:E41B[id]` (ids 0 to 47). Bit 3 of
`$01E7` also selects the sound bank `$1303` (0 or 2, `04:844A`).

**Palettes** (SEEN, `00:877E`, `00:88AE`, `00:8921`): `$01D2`/`$01D3` = `02:8484[2 * id]`/`[2 * id + 1]` (for a town `$01D3 =
02:84E4[spot]`); `06:F876[id]` is a 3-byte pointer to 128 bytes copied to the colour shadow `$0240..$02BF` (background palettes
2 to 5); `06:F945[$01D2]` (256 bytes) and `06:F945[$01D3]` (128 bytes over the first half) go to `$0300..` (sprite palettes
8 to 15). The tables hold raw SNES colours (BGR15). `00:86C8` fills `$0D85` (the corner-face palette choice) from `02:8584`.

## 8. How the party is placed (SEEN)

- **`go to map` (script `2C`, handler `03:B4D9`)**: operands byte (`$164B`), byte (map id into `$01A0`), word x, word y. It
  stores `$01A6 = x`, `$01AA = x + $80`, `$01A8 = y`, `$01AC = y + $78`: **the operand is the follower/camera spot; the
  leader is 128 right and 120 down of it** (run: `secret-nail-porunga`, frame 2323, `--watch-from 2318`: stores at
  `03:B4EC..B50E`). It then sets `$0716 = $A1`, `field_state = 3`, `game_mode = 0`. The fade routine `00:8433` (state `$0716` bit 7)
  ORs `$80` into `game_mode` at the end of the fade (`00:844E`, frame 2356), the main loop clears it and runs the mode-0 init
  `00:8B7A` (frame 2357), which reads `$01A6..$01AC` (`00:8BA3..8BC4`, masked to 8 pixels) and reloads everything
  (`$01A1 = 0` at `00:8C25`, frame 2358).
- **Town entry (mode 1, `00:8E76`)**: stores the leader position in `$01DE/$01E0`, sets `$01A1 = $80`, `00:8D2B` (map id),
  `02:D812`, then the map load; the party appears at the town's default spot (`02:D97D`: x `$80`, y `$A8`, or the
  special values for spots `$9E`, `$9F`, `$32`).
- **Town exit (mode 5, init `00:9251`, run in `shop.ssdreplay` frames 8916-8951)**: attribute `10` -> `game_mode = 5`; the init
  restores `$01A0 = 06:F98D[spot]`, `$01AA/$01AC = 02:E629[spot * 5 + 1..4]` (y + `$10`), `$01A6/$01A8` = leader - (`$80`, `$78`)
  clamped at 0, and re-runs `00:8E26` to clamp the follower inside the new map's size.
- **Start position for mode 6** (`00:933A`): `06:FCDF[id * 4]`, used after story transitions.
- **Camera** (SEEN, `06:F14D` initial, `06:EDB3`/`06:EE42` per frame): the camera follows the leader by at most 4 pixels per
  frame; at the start it is leader minus `$88` in x and minus `$80` in y. Per frame it is clamped to `0 .. (size - 1) * 256`
  per axis; the initial value is clamped to `(size - 2) * 256 + $F8` (a size of 1 block gives the word `$FFF8`, i.e. -8 pixels:
  the code path for a 1-block axis, read in `06:F14D`, not run).
- **Size limits**: width and height are bytes (`$0703`/`$0702`), the world coordinates are 16-bit with bit 15 meaning
  "outside" in `06:F089`, so at most **128 x 128 blocks**; the VRAM ring is 64 columns of 8 pixels and only the 35 columns
  nearest the camera are ever built (`graphics.md`), which is independent of the map size. The 16-bit grid index limits
  `width * height` to 65 535. The smallest original map is 1 x 1, the largest 12 x 12; nothing larger has been run.

## 9. Compression (the question "find the decompressor")

SEEN: none of the map data is compressed. The reads are direct: `06:F089` indexes the block grid, the block table, the
definition and the metatile table through long-indirect loads; the object, zone and palette tables are read the same way.
The **tile sets** are the only compressed part: the LZ stream of `graphics.md` (16-bit size, flag-byte groups, 12-bit distance
and nibble length + 3), decoded by `00:C559` (source `$83/$85`, output `$7E:9000`, size in `$89`), then regrouped by `00:C68C`.
The tile set `08:8D4C` is one stream of `$3600` bytes. An encoder must produce a stream whose decoded size is a multiple of
`$20` and which the decoder stops reading when the output is full; the existing decoders (`ssd_catalogue::graphics::lz_decode`,
`ssd_catalogue::art::lz_decode`) are the reference for a round trip test.

## 10. Hook points for Rust wrappers (pattern of `crates/ssd-game/src/cheats.rs`: `hook_entries`, `Ctx::resume_at(entry)`
to decline, `Ctx::call_original(entry, ReturnKind::Rtl)` to act after)

Widths: "trace" = observed by the traces of `.work/re`; "assumed" = no trace covered the entry (everything runs M8 X16 at
those points as far as the callers show).

| Entry | Routine | Ret | Widths | Wrapper for | Notes |
|---|---|---|---|---|---|
| `00:8E26` | map header lookup | RTL | M8 X16 (trace) | id >= 64: write `$0702/$0703` (size), `$0704-$0706` (24-bit grid), `$0707-$0709` (metatile table), `$01E7` (music/place byte), then return without calling the original | called by `00:8DFA`, `00:931D`, `00:9377`; the `$0707-$0709` constants are written earlier at `00:8C28`/`8C5C`, the last call (via `00:8DFA`) is after them |
| `06:F089` | pixel to tile word | RTL | M8 X16 (trace) | block ids above `$77`, definitions outside the table | hot (32 calls per column); decline at the first compare for original maps; preserves X, Y (it pushes them) |
| `01:8485` | tile probe | RTL | M8 X16 (trace) | attribute table of a pack map | returns carry; writes `$0D88`, `$0D51` |
| `00:8C6D` | tile-set load (label) | (resume) | M8 X16 | replace `$83-$85` and the later `$0707` | reached by `JMP` from `00:8C39` and by fall-through; a label hook, like `03:F320` |
| `03:93FA` | NPC object loader | RTL | M8 X16 (trace) | the pack's object list | clears `$1180..` itself; honour 16 slots |
| `03:93D0` | hide test | RTL | M8 X16 (trace) | hide flags above 31 (extension flags) | A = object id, returns carry |
| `02:D38B` / `02:D34A` / `02:D428` | zone, group, post-load reset | RTL | M8 X16 (trace) | zones of a pack map | `02:D38B` returns early for ids >= `$30` |
| `00:877E`, `00:88AE`, `00:8921`, `00:86C8` | palette indexes and copies | RTL | M8 X16 (trace) | palette sets of a pack map | `00:88AE` copies `$80` bytes to `$0240`; `00:8921` `$100` + `$80` bytes to `$0300` |
| `00:8D4E` | music and sound-bank selection | RTL | M8 X16 (trace) | the song of a pack map | derived from `$01E7`; `04:85A1` queues the song |
| `03:F163` | per-frame trigger dispatch | RTL | M8 X16 (trace) | pack triggers, including while flying | the original returns early in flight; run the pack triggers before `resume_at` |
| `03:F7A4` | map-entry hook | RTL | M8 X16 (trace) | an entry scene of a pack map | only caller `00:8CA8` |
| `03:EA1E` | chapter set-up | RTL | M8 X16 (trace) | not needed for pack maps (never runs for them) | only caller `00:939B` |
| `03:B4D9` | script `2C` | RTL | assumed | none: `go to map` to an id >= 64 works as long as the load wrappers exist | the operand maps to `$01A6..$01AC` |
| `00:933A` | mode 6 init (JMP target) | (no return) | M8 X16 (trace) | start positions of pack maps (`06:FCDF` stops at 31) | |
| `03:F7DE` | start a scene | RTL | assumed | none | A = scene id |

## 11. Changes to the plan, and what a map task must settle first

1. Replace "no map from 48" / "rooms use ids 48 to 63" by: ids 48-63 are the town screens; **pack ids start at 64** and need
   the wrappers of section 10 (header, objects, zones, palette, music, triggers, walk attributes, optional tile set).
2. Replace "the walk code decides by the metatile/tile word" by section 5: one attribute byte per metatile quarter, a new
   metatile needs four bytes in a pack table, not a bit in the cell.
3. The cheapest first milestone ("an original room as a pack plays identically"): ids 1, 6, 7 and 32-47 are 1 x 1 maps with
   no scripted trigger (the caves 32-47 share a record). A pack that replaces map 34 keeps the original tile set and
   metatile table; only the header, the block grid (and one definition) change. This avoids the art wrappers.
4. A new tiny room with new art needs wrappers (a), (b) and (c) of section 4.
5. The plan's "tile-set table" does not exist; "the compression used for map data" is none (section 9).
6. Open questions I could not close: the exact tiles of the 16 free block ids when edited in place (they are aliases of the
   blank block, so a pack can use them with a patched `06:F1EE` in the image, 23 ids at most), the talk programs of
   `03:9DF1`, the three-state `$01BC` bits other than bit 0, and what exactly hangs for id 48.

## 12. Runs (all headless)

- `xtask shots wander --frames 3300 --no-png --peek map_id,7E0702,...` (map 0 header registers).
- `xtask shots .work/replays/shop.ssdreplay --upto 20000 --every 20000 --no-png --watch map_id,7E01A1,7E01BD,...` and the
  `--upto 8960 --watch-from 8914` variant (town entry and exit with PCs).
- `xtask shots .work/replays/secret-nail-porunga.ssdreplay --upto 2360 --watch-from 2318 --watch map_id,7E164B,7E01A6,...`
  (script `2C` with PCs); `--watch 7E1306,map_id,7E01E7` (song bytes).
- `xtask probe --address 03EA1E --returns rtl --dir .work/m10-research/r456/rp` (set-up calls).
- `save-edit --set page:A0=40` then `replay-from-save` then `shots` (map 64: no frame after the load within 60 s).
- Python readers: `lib456.py` (cached ROM at `%LOCALAPPDATA%\dbz-ssd-recompiled\cache\rom\canonical.sfc`), `t3.py` (headers),
  `t7.py`, `t9.py`, `t10.py`, `t13.py`, `t14.py`, `t4.py`, `t12.py`.
