# Research item 6: title, new game, Continue, endings, world map, chapter set-up (read-only notes, scratch)

Scope: the plan's research item 6 (title menu code and entries, the Continue slot list, what the new-game path sets,
where the ending is detected and a safe place to flag "completed", how the flight map starts a landing and a go-to-map,
the chapter set-up outputs a synthesized save image needs). For Task 8 (chapters, title entry, unlock) and Task 6 (POIs).

Tags: **SEEN** = code read at the address, or a headless run (named); **INFERRED** = read but not exercised, or a design
conclusion. Original code is described, not quoted. Tools: `xtask disasm`, `shots --watch/--watch-from/--peek/--frames`,
`probe`, `save-edit`, `replay-from-save`, Python readers of the cached ROM (`.work/m10-research/r456/`, scratch).

## 0. Result in twelve lines

1. **Boot** (`00:8000`) clears work RAM `$00-$1FFF` and the lists `$7E:29A0-2B7E`, then calls the **title routine `00:9D97`** by `JSR` (from `00:809C`) and
   tests its return: **A = `$FF` -> Continue (`game_mode = $8E`), anything else -> new game (mode `$87`, story transition 0)** (SEEN: probe of `00:9D97` over
   `wander`, exit A low byte `$07`, and over `secret-raditz`, exit A `$00FF`; code `00:80A4..80AC`).
2. **The title menu has two entries, GAME START and CONTINUE, drawn from the title's graphics, and it exists only when a save slot with a valid mark exists.**
   With empty SRAM the menu is never shown: Start leaves the title straight into the new game (SEEN: `wander` frame 2440: logo only, new game at 2465;
   code `00:A9B1..A9E8`). **A title entry for a chapter must therefore also work without saves.**
3. The menu is the loop `00:AA92..AAD8`: `$82` is the cursor (0 = GAME START, `$FF` = CONTINUE), toggled by Right, Left and Select (`$48 & $23`), confirmed
   by Start (`$48 & $10`); the routine returns `$82` in A with `RTS`. Cursor tiles are written at VRAM `$6F43` / `$6F51`. The labels are tiles of the title picture
   (no strings).
4. **Continue** = mode `$0E` (slot list, init `00:9579` -> `03:E80C`, per frame `03:E8AA`), then mode `3` ("Load game?" with the party status), then mode 0 (the
   ordinary map load `00:8B7A`). The data comes from **`03:E5FC`** (RTL): slot `$0D8E` -> SRAM base `03:E5EE[slot]` = `$0100/$0600/$0B00`; it copies `$3F0` bytes to
   `$7E:2800` and `$100` bytes to `$0100`, rebuilds the party (`03:E4F4`) and applies fix-ups (`03:E66B`).
5. **Continue does not run the chapter set-up** (`03:EA1E`): `xtask probe --address 03EA1E` over `secret-raditz` (a replay that starts with Continue): 0 calls
   in 5304 frames; `shop` and `secret-nail-porunga` 1 call each (from the story transition, `00:939B`). Everything a chapter set-up leaves behind must therefore be in the
   saved image and page, or be recomputed by the map's trigger (section 6).
6. **New game** = title returns not-`$FF` -> mode `$87` -> story transition 0 (`04:9FDC`) -> ... -> mode 6 (`00:933A`, start position `06:FCDF[0]`) -> the chapter set-up of map 0
   -> map load. The party of the new game is Goku and Piccolo, level 1 each; items, zeni and flags are zero (SEEN peek, `wander` frame 3300).
7. **The ending** is game mode `$11` (or `$12`): set by the battle-win routine (`05:97F3..9861`) after the groups `$56`/`$6E` (`$11`) or `$6F` (`$12`); the mode inits are
   **`00:96A5`** (`$11`) and **`00:9730`** (`$12`), both RTL (called from the main loop through the mode table). A wrapper that notes "completed" at `00:96A5` (and `00:9730`)
   runs once per ending, at its start (SEEN: `secret-ending-normal` frames 4576, 4609, 4610: `game_mode` `$11`, `$91`, `$11`, then no change to frame 40 000).
8. The **world map** is map 0 (and the other field maps) with the flight state in `$01BC` (bit 0 = airborne). A landing is an **entity touch** (`03:967E`, only when
   not airborne): `landing_spot` (`$01BD`) = the entity id, `game_mode = 1`; mode 1 (`00:8E76`) sets `$01A1 = $80` and loads the town screen `$30 + type`
   (SEEN: `shop.ssdreplay` frames 3684-3719). `$01A1` bit 7 = "inside a town screen".
9. `03:F7A4` is **not** per frame: it is the map-entry hook, called once at the end of every map load (only caller `00:8CA8`).
10. `go to map` (script `2C`) places the follower/camera at its operand and the leader at +(`$80`, `$78`), sets `$0716 = $A1`, `field_state = 3`, `game_mode = 0`, and
    the fade ends in a map load; it does not run the chapter set-up (`r4-maps.md`, section 8).
11. **A synthesized slot is enough to start a chapter**: the game's own Continue path needs only valid SRAM slots (mark at image `$3E0`; the checksum is checked at boot only).
    The most robust way is to give the Chapter 2 session its own SRAM file with the synthesized slot written by `ssd_save::SaveFile::set_slot`, which needs no wrapper
    at all in the Continue path (INFERRED design; the pieces are SEEN).
12. What the saved memory does **not** carry (and a chapter must recreate): the name slots `$1640-$1645`, `$0D59`, `$1648` (scene running), `$164B`, the story-transition state.
    `$01E4` is saved (page) but only set by the set-up (section 6), so a synthesized page must set it.

## 1. Boot and the title (SEEN)

`00:8000` (reset, 1 routine): clears `$00-$1FFF` (the 16-bit loop with `STA $00,X`, direct page 0) and `$7E:3800`, `$7E:29A0`, `$7E:3A00`, `$7E:2B00` for `$1E0` bytes each; sets
`$01D0 = 2` (text speed), `$0D59 = 0`, `$121F = 0` (transition target), `$24 = $87`, `$01AA = $01AC = $70`, `message_lists = 07:8000`, number slots `$1610/$1620/$1630 = $80FF`,
`$1303 = $80`; calls `04:844A` (sound), **`03:E70E`** (SRAM validation: for every slot whose mark is present and checksum wrong, restore from the backup or wipe), then **`JSR 00:9D97`**.
After the return: `CMP #$FF; BNE` -> if equal `$24 = $8E`. The main loop (`main_loop_idle`, `00:80CE`) then runs `JSL 00:8B28` for a mode with bit 7 set: it clears the bit and
jumps through the mode table `00:8B54` (19 words: 0 `8B7A`, 1 `8E76`, 2 `8E96`, 3 `9102`, 4 `91B6`, 5 `9251`, 6 `933A`, 7 `93AC`, `8` `943F`, 9 `9444`, `A` `9459`, `B` `952B`,
`C` `9530`, `D` `9535`, `E` `9579`, `F` `95A9`, `10` `967E`, `11` `96A5`, `12` `9730`).

`00:9D97` is the whole intro and title (one long blocking routine, 600 to 800 million master cycles in the two probed calls): logo, clouds, the title screen. Its helper
`00:A961` = "wait a frame and test Start" (called about 20 times); the test is `00:A9A5..A9E8`:

- `$0D91` bit 7 or Start (`$48` bit 4) -> continue the test, else return;
- `$7C == 0` (the attract loop) -> pop the helper's return and jump to the title screen setup `00:A153` (the first Start goes from the demo to the title);
- `$7C != 0` (title shown): scan the three slots for the 8-byte mark at `$70:04E0 + $500 * n` (pattern at `00:AAD9`, same constant as the save mark);
  **a match and `$3D == 8` -> build the two-entry menu (`00:A9F1`..`AA90`)**; no match -> the helper returns through an extra pop, i.e. **the title routine itself returns** (the new game)
  with a value other than `$FF` (SEEN: exit A low byte `$07`).

Menu loop `00:AA92`: waits a frame, draws the cursor at VRAM `$6F43` (GAME START) or `$6F51` (CONTINUE), reads `$48`: `& $23` toggles `$82` (`EOR #$FF`), `& $10` returns `$82`.
(`$48` bit 0 = right, bit 1 = left, bit 5 = select, bit 4 = start, by the code; not cross-checked against the joypad map.) Screenshots: `shots secret-raditz --frames 1750` shows
"GAME START   CONTINUE" with the cursor; frame 1900 "Which slot?" with three windows.

**Hook options for a third entry** (INFERRED; all addresses SEEN):

| Idea | Hook | Notes |
|---|---|---|
| Add an entry to the original menu | `00:AAAA..AAB4` (the toggle) and `00:AAD6` (the return) inside `00:9D97` | needs new tiles for the label (the labels are picture tiles of `00:EB97`-`EFE3`); the loop is not a routine entry: a label hook |
| Decide at the return | `00:80A4` (the compare in the boot code) | the host chooses mode `$8E`, `$87` or a chapter start by what the title returned plus a host flag |
| Draw the entry in the HD overlay and start the chapter from the host | no hook in the title | the host reads the input of the title, loads a Chapter 2 SRAM and resets the machine into its own title (flow decision for Task 8) |
| Skip the title for chapter sessions | none | the host starts the machine with the chapter's SRAM, so the original menu (GAME START / CONTINUE) is the chapter's own slot list |

## 2. New game (SEEN)

Title returns not-`$FF`: `game_mode` stays `$87`; the main loop clears bit 7 and runs the mode 7 init `00:93AC`, which copies a 5-byte `JSL`+`RTL` trampoline into `$1223` and patches its
24-bit target from the 26-entry table at `00:93EE` (3 bytes each, bank 4) by `transition_target` (`$121F`, 0 at boot): target 0 = `04:9FDC` (the first transition, the opening), 1 `A304`, 2 `A68A`,
3 `A81E`, 4 `A915`, 5 `A95E`, 6 `AA32`, 7 `B09E`, 8 `B29B`, 9 `B906`, `A` `BB71`, `B` `BE3F`, `C` `BFB7`, `D`/`E`/`13`/`16`-`18` `9235` (idle), `F` `BFC7`, `10` `C715`, `11` `CA53`,
`12` `CDC5`, `14` `D39B`, `15` `D546`, `19` `D709`. Each is run every frame in mode 7. The tables `04:9A0E`, `9A2A`, `9A46`, `9A62` (28 rows by `story_step`) give, for the step that
ends, the effect byte, the next `game_mode`, the next step and the next map. Step 0 gives mode 6 and map 0 (SEEN: bytes `83`/`00`, `06`, `FF`, `00`).
In `wander`: mode 7 init at frame 2465; mode 6 requested at 3086 (`game_mode = 6`, `$0716 = $82`), the fade ORs `$80` at 3134 and the init of mode 6 (`00:933A`) runs at 3135.

**Mode 6 init `00:933A`** (SEEN, `00:933A..93AC`): position from **`06:FCDF[map * 4]`** (x word, y word; map 0 = (2160, 2688)); stores them in `$01AA/$01AC`, `$01C0/$01C2/$01C4/$01C6` and the
camera copies at -(`$80`, `$78`), runs `00:8E26`, clamps the camera copies into the map, `00:92FB`, then **`03:EA1E` (the chapter set-up)** and `JMP 00:8B7A` (the map load).
Table `06:FCDF` is valid for ids 0 to 31 only.

State after the first map is up (SEEN, `shots wander --frames 3300 --peek ...`): party record 0 (`$0B00`): flags `$C0`, unit `00`, level 1, HP `$50`, KI `$0C`, BP `$01A0`; record 1 (`$0B20`):
flags `$C0`, unit `01`, level 1; `$01BB = 2`; `$01B2` (item count) 0, zeni 0; story flags `$0170-$017F` all 0; **`$0190` (party list) = `$FF`: the list is only written by a Save**
(`03:E556` calls `03:E4D1`) or by the set-up at the *start* of `03:EA1E`; map 0 at (2160, 2688) with leader (`$01A2/$01A4`) `$0870, $0A80`; `$01D2 = 0`, `$01D3 = 3`, `$01E4 = $0A`; `$1640..` 0;
`$7E:29A0` and `$7E:2AA0` 0 (cleared by the boot, so not `$FF`-terminated).

## 3. Continue (SEEN: trace of `secret-raditz`, `--watch-from 1700`)

| Frame | Event | Code |
|---|---|---|
| 1701 | menu built, `$82 = 0` | `00:AA3B` |
| 1765 | Right: `$82 = $FF` | `00:AAB6` |
| 1789 | Start: the title returns, `$24 = $8E` | `00:80AC` |
| 1789 | main loop: bit 7 cleared, mode `$0E` init | `00:8B4F`, `00:9579` -> `03:E80C` |
| 1812-1813 | the three slots are loaded into WRAM one after the other to draw their party windows (`$0D8E` = 0, 1, 2; empty slots give `map_id` `$FF`) | `03:E82B`.., `03:E854` x3 |
| 1846 | fade done, `field_state = $0E` | `00:8441` |
| 1893 | A on the first slot: `$0D8E = 0`, `03:E5FC`, mark test `03:E655`, `$0D8D = 1`, `$0716 = $81`, `game_mode = $0E` again | `03:E8C5..E8F5` |
| 1952 | "Load game?" with the party: `game_mode = 3` | `01:92A6..92AF` |
| 2049 | countdown state done (`$0D8D = 2`) | `03:E901` |
| 2097 | Yes: `03:E5FC` once more, `$0716 = $81`, `field_state = 3`, `game_mode = 0` | `03:E933..E969` |
| 2130 | fade done: `game_mode = $80` | `00:844E` |
| 2131-2132 | main loop clears bit 7: mode 0 init `00:8B7A`; `$01A1 = 0` at `00:8C25` | |

`03:E5FC` (RTL, entry widths assumed M8 X16): `$0D8E` (0 to 2) -> `03:E5EE[slot]`; copies the follower position `$01A6/$01A8` to the camera `$33/$35` and the saved leader
`$01AA/$01AC` to `$01A2/$01A4`; copies `$3F0` bytes from `$70:base` to `$7E:2800` and `$100` bytes from `$70:base + $3F0` to `$0100` (so the last two bytes of the page, the
checksum, land in `$01FE/$01FF`); `JSL 03:E4F4` (party records from the list `$0190`: for each id until `$FF`, `03:E461` copies the image record `$7E:2800 + 32 * id` into the next party slot
at `$0B00`) and `JSL 03:E66B` (clears `$0D87`, `$1661`, `$1665`; if `$01D7` bit 7 is set and the scene is `$11` it re-arms the Saiyan chapter scene; for map `$18` jumps to `03:E6CF`,
for `$37` and `$3A` sets `$0D87 = $0C` / 4). After the Yes, `03:E937..E960`: if `$01BC` bit 0 (airborne) it ORs `$20` and sets the travel mode `$0D55/$0D57` to 4 (bit 6) or 2.

The save side (SEEN, `03:E556`): `03:E4D1` (list from the party records), `03:E419` (every party record with flags bit 7 or 6 into `$7E:2800 + 32 * char id`), the three junk bytes
`$4C00-$4C02 -> $2BD8-$2BDA`, the 8-byte mark from `03:E5F4` into `$7E:2BE0`, `$01A6..$01AC` = camera and leader, then `$3F0` + `$100` bytes to SRAM `03:E5EE[$0D62]`, the checksum
(`03:E6E1`) and the backup copy at `+$1000`. `ssd_save::SaveFile::set_slot` does the same without the game.

**Layout the loader consumes** (also `save-format.md`): image `$000-$17F` twelve records by character id; `$1A0` the `$29A0` list (note: **the whole image is copied by the save, so the
lists at `$7E:29A0` (image `$1A0`) and `$7E:2AA0` (image `$2A0`) and the last-fight words `$2B03`, `$2B0A` (`$303`, `$30A`) are saved verbatim**; `save-format.md` says "not saved" meaning no code writes
them for the player, they are still in the copy); page `$00-$2F` items, `$40-$5E` cards, `$70-$7F` story flags, `$80-$83` events, `$90-$9F` party list, `$A0` map, `$A1` town bit, `$A2-$AD` positions,
`$B2` item count, `$B4` zeni, `$BB` party size, `$BC` flight bits, `$D0-$E7` scene and effect state.

Character id -> unit id of a record (`+01`), SEEN, table `00:8AE5`: `0->00, 1->09, 2->01, 3->0A, 4->0B, 5->0C, 6->0D, 7->50, 8->22, 9->23, 10->24, 11->61`.

## 4. Endings (SEEN)

- Detection: **`05:97F3`** (the battle-win routine, M8 X16): by `encounter_group_index` (`$0EA3`): `$51`, `$57`, `$54` special cases; group `$6E` or `$56` -> `$11`; `$6F` -> `$12`; the mode is stored
  by the tail at `05:985F` (`AND #$7F; STA $24`). A won battle that ends a scripted loss (`$57`, the Nail fight) goes through another branch.
- Mode `$11` init **`00:96A5`**: `03:F08D` (clears the follower words `+12/+18` of the party records); if Goku's record (`03:F021` finds a member by character id) is unit `$3B` (Super Saiyan)
  nothing is dropped, else Vegeta (char 7) is demoted (`+00 = $80`); builds `$82` = a bitmask of the living members (table `00:9728`); **`JSL 00:B013`** = the story captions over the flight and the
  staff roll (about 11 000 frames, blocking; the five texts at `00:BD87..C052`); then if bit 7 of the mask (Vegeta alive) -> `$164C = 7`, `03:CF8C`, enemy unit `$4B`, group `$6F`, mode 2 init
  (`JMP 00:8E96`: the Super Vegeta fight). Otherwise `RTL` and the game stays in mode `$11` forever (the credits are the last thing; SEEN: no further `game_mode` change in
  40 000 frames of `secret-ending-normal`).
- Mode `$12` init **`00:9730`**: `JSL 00:B340` (the true ending's screens, `$81 = 1`), `RTL`.
- Timeline SEEN (`secret-ending-normal`): battle modes 2/4/`$0C` at frames 2352-3706, `game_mode = $11` at 4576 (set by `05:985F`), `$91` at 4609 (fade end), `$11` at 4610.

**Where to flag "completed"** (INFERRED, all addresses SEEN): a wrapper on **`00:96A5`** and **`00:9730`** (RTL, entered from the mode table; resume with `Ctx::resume_at` of its own entry like
`MONEY_ENTRY`) that sets a flag byte in the mod state block and lets the original run. It is reached exactly once per ending, only in the story's final fights, only in Enhanced when the wrapper
is installed. It flags at the start of the credits, not at their end; flagging at the end needs a hook inside `00:B013` or a counter of frames in mode `$11`: not needed for "finished_main".

## 5. The flight map, landings, `go to map` (SEEN unless marked)

State bytes: `$01A0` map id; `$01A1` bit 7 = inside a town screen (SEEN: set at frame 3718, cleared at 8950 of `shop`); `$01BC` flight bits (SEEN in `landing`: `$82` take-off at 3762, `$A1`
flying at 3819, `$82` landing at 4010, `$80` landed at 4076; bit 0 = airborne is the only bit whose use I read in code: `03:967E` returns "no hit" while it
is set, `03:F163` returns early while it is set except on map `$10`, the walk probe `01:8485` passes attribute 1 while `$01BC & $0F`); `$01BD` = `landing_spot`.

**Landing (town entry)**, shop replay, frames 3684 to 3770:

1. A map entity (`$1000 + 6n`, built by `03:92CF` from `02:E4C8[map]`; its position equals the landing record's, INFERRED copy) is touched: `03:967E` (RTL) tests the leader box against the entity box
   (`lookup_byte_pair` sizes), requires `$01BC` bit 0 clear, then `03:970A..9728`: music `$84`, `$01BD = entity id`, `$24 = 1`, `$25 = 3`, `$0716 = $A1`, `$0D87 = type & $3F`.
2. Fade out; `00:844E` ORs `$80` into `game_mode` (frame 3717: `$81`); mode 1 init `00:8E76`: leader position -> `$01DE/$01E0`, **`$01A1 = $80`** (frame 3718), `00:8D2B` computes
   `$01A0 = $30 + 02:E3FB[02:E629[spot * 5] & $1F]` (frame 3719: `$3A`), `02:D812`, `JMP 00:8B7A`.
3. The map load takes the town branch (`r4-maps.md`, section 4): `02:D97D` places the party at x `$80`, y `$A8` (spots `$9E`, `$9F`, `$32` have their own), objects from `02:ED17[spot]`, tile set `08:BF20`,
   metatiles `08:FA00`, **`03:F7A4`** at the end: for `landing_spot` 4 it starts scene 0 if story flag 3 is clear, for 9 scene 4 if flag 0 is clear, for `$32` nothing (via `03:F7DE`).
4. Exit: attribute `10` in the town table (or a door scene) -> `$24 = 5`, `$0716 = $A1` (frame 8916, `01:84EB`), mode 5 init `00:9251` (frame 8950): restores the leader position, sets
   `$01A1 = 0` (`00:92AC`), `$01A0 = 06:F98D[spot]` (the parent map), positions from `02:E629[spot * 5 + 1..4]` (y + `$10`), `00:8E26` to clamp the follower, then the mode-0 style load.

So a **landing spot is a row in these tables**: `02:E629` (type, x, y), `06:F98D` (parent map), `02:84E4` (palette), `02:ED17` (town objects), the entity list `02:E4C8[parent]`, `02:E3FB` (type to town map),
plus the shop tables (`shop_places`, 30 places). All have 160 entries or fewer; the original's spots end at 159 (`$9E`, `$9F` special). **A new spot needs wrappers on `03:92CF` (entity list), `00:8D2B`
(town map from type), `00:9251` (parent map and exit position), `00:877E` (palette) and `03:9336` (objects)**; the 16 town screens (`06:F2DE` ids 48-63) cannot grow.

**Go to map by a pack (the POI on the world map)**: the cleanest is not a spot but a trigger: a wrapper on **`03:F163`** (RTL, M8 X16 by trace) that, before resuming the original, tests the leader position
`$01A2/$01A4` against the pack's rectangles and the flight bit, then performs the same stores as script `2C` (`03:B4D9..B520`): `$01A0`, `$01A6/$01A8/$01AA/$01AC`, `$0716 = $A1`, `$25 = 3`, `$24 = 0`.
A prompt can be a normal message (`00:89AC`) before it. The original's gate (`$01BC` bit 0) is inside `03:F163`, so a wrapper placed first can act in flight as well.

## 6. The chapter set-up and what a synthesized image must contain (SEEN where marked)

`03:EA1E` (RTL, M8 X16 by trace, only caller `00:939B` = the end of mode 6; the call list: `00:933A`, mode 6, `04:xxxx` transitions that end in mode 6):

1. clears the scene state: `$01D9`, `$1648`, `$165F`, `$01D4`, `$0D59`;
2. `03:E419`: every party record with flags bit 7 or 6 is copied to the image slot `$7E:2800 + 32 * char id` (so characters leaving the party are kept);
3. unless the map is `$16`: `03:E4D1` rebuilds the list `$0190` (character ids of the party records in slot order, `$FF` end);
4. `03:E4BF`: all nine party records' flag bytes cleared; `03:EF1A`: the 16 flag bytes `$0170-$017F` cleared; `$01BB = 0`; `03:F040`: `$01E4 = 03:F04F[map]` (a per-map byte, `$0A` for maps 0, 5, 11, `09` for 9, `0C` for 16 and 22, `0F` for 24, else 0: decremented after won battles by `05:932F`, displayed by `01:B4EE`; INFERRED role: a battle countdown); `03:F08D`: the words at `+12` and `+18` of every record cleared;
5. `JMP` through the table `03:EA55` (**32 words, maps 0 to 31**) to the routine of the map. Distinct routines (entry: maps): `EA95`: 0, 2; `EAC4`: 1; `EB23`: 3; `EB53` (RTL): 4; `EB54`: 5; `EC24` (RTL)/`EC25`: 6, 7;
   `EC63`: 8, 9; `ECEE` (RTL): 10; `ECEF`: 11; `ED94`: 12-15; `ED9C`: 16; `EE27`: 17; `EE2A`: 18-20; `EE73` (RTL): 21; `EE74`: 22; `EEE1` (RTL): 23; `EEE2`: 24; `EF19` (RTL): 25-31.

What the routines produce (read, not all run): the party for the chapter: new records through **`03:B203`** (`$10` = character id, `$11` = level, X = record offset; it zeroes `+07..09`, `+0C`, `+0D`, sets
`+1C`, `+02`, flags `$C0`, calls `00:89B8` (INFERRED: the level-row builder), `00:8AD6` (unit id from char id, table `00:8AE5`), `01:C0FD`, copies max HP/KI into HP/KI, `+17` from the unit stats) or by copying image records
(`03:E461`: A = char id, Y = party slot offset), then `$01BB` (party size) or `INC`s of it; level bumps through **`03:C64A`** (`$164C` = character, `$164D` = levels to add); story flags
(bit setting in `$0170+` through `03:9B7F`: the flag number of a character is its id for ids 0 to 11, plus chapter flags such as `$0173 = $1E`, `$0171 = 6`); the palette indexes `$01D2`, `$01E2`, `$01D3`
(overwritten from the map table at the next `00:877E`); `$0D59` (1 for the Namek and Earth-chase maps); `$01BC = $80` (map 8/9 set-up); **the first scene** (`$01D8` and `$1648 = $80`:
map 5 scene `$4E`, map 9 scene `$12`, map 21 scene `$50`, 22 scene `$35`, 24 scene `$3B`, 16 `$1C`); **name slots** `$1640/$1642` (index `$52`/`$23`/`$24`, type `$80` = unit) set by map 11's routine;
for map 22 the list **`$7E:29A0`** (character ids of image records whose flags are below `$C0`: the people not in the party) and `$01B0 = 0`; for map 16 effect values.

Outputs that survive a Save and a Continue (they live in the image or the page): the party records (image), the party list `$0190`, `$01BB`, the story flags, `$01D2/$01D3/$01E2`, `$01D8`, `$01E4`,
`$01BC`, the lists `$7E:29A0/$2AA0`. Outputs that do not: `$1648` (a scene in progress), `$1640-$1645` (name slots), `$0D59`, `$164B`, `$164C/$164D`, `$0D87`, the follower trail (`+12/+18` words, rebuilt),
the entity tables. After Continue the map's own trigger (`03:F163`) restarts a scene from the flags and the position (`scenes.md`); a chapter that must open with a scene needs the start through
`03:F7DE` (A = scene id: `$01D8`, `$1648 = $80`) at the map-entry hook `03:F7A4`, and the name slots written by the host.

**Checklist for a complete synthesized slot** (INFERRED from the above; compare `ssd-save` `Slot` helpers and `save-edit`):

1. image: for each party member the 32-byte record at `$000 + 32 * char id`: `+00 = $C0`, `+01` unit id (table above), `+02` level, `+03` HP, `+05` KI, `+07..09` BP, `+0A` max HP, `+0C` max KI,
   `+0E`, `+0F..11` EXP, `+17`, `+1B` join order, `+1C` char id (from `characters.md` and `save-format.md`; built by the level rows as `save-edit goku.level=` does); other characters present with flags `$00`/`$40`
   (not in party) so the story's "recruited" logic sees them as the original's set-up leaves them (map 1's set-up, `03:EB03`, tests bit 6 of the image records; the meaning of that bit is not read);
2. image `$1A0` (`$7E:29A0` list) and `$2A0` (`$7E:2AA0` list) only for the scenes that read them (map 22 effect `$35`; the last chapter's party swap); `$303/$30A` for the final fight; mark at `$3E0`;
3. page: list `$0190` ending in `$FF` and `$01BB` equal to its length (the game drops the extra members a few frames after load, SEEN in `save-format.md`); flags `$0170-$017F` (flag n = char id for
   members, plus the chapter's flags); `$0180-$0183`; item list and `$01B2` count (the attribute bytes are rewritten by `01:C535`); zeni `$01B4`; map `$01A0` and `$01A1 = 0`;
   `$01A2/$01A4` (leader), `$01A6/$01A8` (camera: leader minus (`$80`, `$78`), clamped to 0), `$01AA/$01AC` (leader again); `$01BC` (flight bits, `$80` for a walking party on the Namek maps);
   `$01D2/$01D3/$01E2` (from `02:8484`), `$01D8` (scene id), `$01E4` (from `03:F04F[map]`), `$01B0/$01B1` battle counters (0);
4. outside the save: set `$1640..` when the first message names someone; `$0D59`; start the first scene through the `03:F7A4` hook.

## 7. Hook points (pattern of `crates/ssd-game/src/cheats.rs`: `hook_entries`, `Ctx::resume_at`, `Ctx::call_original`)

| Entry | Routine | Ret | Widths | Use |
|---|---|---|---|---|
| `00:9D97` | the title (whole intro) | RTS (JSR from `00:809C`) | M8 X16 (trace) | decide new game / Continue / chapter by the returned A; do not `call_original` it (hundreds of millions of cycles; the wrapper cannot wrap a blocking routine) |
| `00:A9B1` / `00:AAAA` / `00:AAD6` | Start test, toggle, return of the menu | label hooks | M8 X16 (read) | a third entry |
| `00:80A4` | compare of the title's result | label | M8 X16 | host decision |
| `03:E5FC` | slot load (RTL) | RTL | assumed M8 X16 | inject a synthesized image instead of reading SRAM (`$0D8E` = slot) |
| `03:E655` | mark test (RTL, carry = valid) | RTL | M8 X16 (trace) | mark of a virtual slot |
| `03:E70E` | boot validation | RTL | M8 X16 (read) | keep a synthesized SRAM from being wiped (it needs a good checksum) |
| `00:9579` / `03:E80C` | Continue (mode `$0E`) init | RTL | assumed | list a chapter slot |
| `00:93AC` | mode 7 init (the opening) | RTL | M8 X16 (trace) | replace the opening by a chapter start |
| `00:933A` | mode 6 init (JMP target) | no return | M8 X16 (trace) | start position and the chapter set-up |
| `03:EA1E` | chapter set-up | RTL | M8 X16 (trace) | skip or replace for a pack chapter |
| `03:A6CB` | map-load prelude: `$01D6 == $60` rebuilds the party from the list (`03:E419` + `03:E4F4`) | RTL | M8 X16 (read) | a way to ask for a party rebuild on the next map load |
| `03:F7A4` | map-entry hook | RTL | M8 X16 (trace) | first scene of a chapter, pack entry scenes |
| `03:F163` | per-frame trigger dispatch | RTL | M8 X16 (trace) | POI triggers, `go to map` while flying |
| `03:F7DE` | start a scene (A = id) | RTL | assumed | call from Rust-side glue |
| `00:96A5`, `00:9730` | ending modes `$11`, `$12` | RTL (jump target) | assumed M8 X16 | flag "completed" |
| `05:97F3` | battle-win routine | RTL? (read as a routine) | M8 X16 (flags) | alternative: the group tests, not needed |
| `03:967E` | entity touch | RTL | assumed | custom entity types |
| `03:92CF` | entrance entity builder | RTL | M8 X16 (trace) | entities of pack maps/spots |

## 8. Changes to the plan, open points

1. The plan's "title menu's entries": there are two, shown only with a saved slot; a chapter entry needs the no-save case (section 1) and a new label graphic.
2. "The ending (mode `$24 = $11`) reached in an Enhanced run writes `chapter2.unlocked`": use `00:96A5`/`00:9730` (the group `$6F` fight is `$12`, after `$11`'s Super Vegeta branch).
3. "The chapter set-up outputs": the set-up routines are map-specific code (14 routines, 32 maps), not data; a chapter is a synthesized image plus the first scene through `03:F7A4`; it never needs `03:EA1E`.
4. The doc `scenes.md` says Continue runs the set-up: it does not (section 0, item 5); and `03:F7A4` is a map-entry hook, not a per-frame check. Both should be corrected when the notes are merged.
5. Not verified: the exact joypad bits of `$48` in the title; the tiles of the title labels; the model of the follower trail; `01:C0FD`/`00:89B8` (the stat builder) beyond their use; whether an empty-slot Continue
   (SRAM wiped, mark absent) is reachable; the effect of a synthesized slot's scene state `$01D7` bit 7.

## 9. Runs

`xtask probe --address 009D97 --returns rts --dir rp2` (title: two calls, exit A `$00FF` for Continue and `$0F07` for a new game; entry widths M8 X16); `xtask probe --address 03EA1E --returns rtl --dir rp`;
`xtask shots secret-raditz --frames 1750,1812,1900` (title and slot list screenshots); `--upto 2300 --watch game_mode,7E0D8D,7E0D8E,map_id,7E0716,7E0025,7E0082,7E01A1 --watch-from 1700`;
`xtask shots wander --frames 3300 --peek ...` (new game state), `--frames 2300,2420,2440,2460 --peek 7E0082,7E007C,7E003D,game_mode`;
`xtask shots .work/replays/shop.ssdreplay ... --watch ...` (town entry and exit); `xtask shots .work/replays/secret-ending-normal.ssdreplay --upto 40000 --watch game_mode,...`;
`xtask save-edit` / `replay-from-save` for map 48/64 (hang, see `r4-maps.md`).
