# Research item 5: text and font (read-only notes, scratch)

Scope: the plan's research item 5 (message table format, pages, portrait and handler words, name slots, line
length and window size, the printable set of the English font, long names, caption tables, whether a message
can address another bank, the tile-code table of `ssd-catalogue::text` as the compiler's source). For Task 4.

Tags: **SEEN** = code read at the address given, data dumped from the cached ROM, or a headless run (named);
**INFERRED** = read but not exercised, or a conclusion. Original code is described, not quoted.
`docs/re/data/text-engine.md` and `names.md` are the base; where they are wrong or silent this note says so.

Tools: `xtask disasm`, `xtask text-dump .work/m10-research/r456/strings.tsv` (the dump also wrote `messages.tsv` next to it),
`xtask shots ... --peek`, a ROM reader in Python (`.work/m10-research/r456/lib456.py`), and a render of the 384 font tiles
(`font_sheet.png`, `fz*.png`, scratch).

## 0. Result in twelve lines

1. **Correction to the plan and to the vocabulary: `D00n` selects a string table, it is not a portrait word.** The portrait is `F4 nn`
   inside the string (two operands in the battle table). `D002`, `D003`, `D004` also open the dialogue window (entries `$08`, `$09`,
   `$3C` of the window engine), `D001` and `D005`-`D007` open none (SEEN, `01:D3DD`, `01:D42E`).
2. A message is a word list in the table `message_lists` (24-bit pointer in `$0738-$073A`, normally `07:8000`, 368 entries); each word
   is `D0nn` (table), a string index (high nibble 0, so index `<= $0FFF`), `E0nn` (run handler `nn`, 52 of them) or `Fxxx` (end).
   **The list base is a 24-bit variable and the list words are read with the bank of that variable**: a wrapper on `01:D3C0` (RTL, entry
   M8 X16 by trace) can serve message ids of 368 and up from the extended image (SEEN).
3. **String tables are addressed by 24-bit pointers** (`07:8F16`, 8 entries of word + bank) and the string words of a table are read with the
   bank of the table's pointer: tables 0, 1, 3 to 7 are in bank 7 and **table 2 (dialogue) is in bank 6**. A string table in the extended
   image works for the original's engine if the pointer entry is changed; its strings must sit in the same bank as its word array (SEEN, `01:CF3A`).
4. `D0nn` with `nn >= 8` reads garbage (the pointer table has 8 entries and the window offsets table `01:CCB4` too): a **new table needs a
   wrapper on `01:CF3A` (per character, hot) or on the call that sets `message_text_base` (`01:CC62..CC7A`)**; reuse of tables 4 to 6
   (9, 2, 2 strings, never growing) is not a solution.
5. Name slots are the codes `FB`, `FA`, `F9` = the slot words at `$1640`, `$1642`, `$1644` (the dump's `{name:1}` to `{name:3}`); the slot byte 0 is an index (names
   at bank 2: `string_pointers`, `unit_name_pointers`, `attack_name_pointers`, `object_name_pointers`, each at most 256 entries,
   name = length byte + tiles), byte 1 tells the table by its high bits. The copy buffer is `$1600-$160F`: **a name longer than 15
   tiles overwrites the number slots**. Number slots `F8`/`F7`/`F6` read FF-terminated tiles at `$1610/$1620/$1630`.
6. The window budgets of the original (cells of 8 pixels): help 16, battle 17, **dialogue 20**, **narration 25**, system strings up to 26.
   The engine does not wrap and does not check: `FF` ends a line, `FE FF` a page, `FF FF` the string, `FD` a string that closes itself.
   Original pages hold up to 3, 4 (3 in practice), **5** and 4 lines in tables 0, 1, 2, 3.
7. **The printable set** (tiles proven by the font render and by the ROM's own use): space `01`; `a-z` and `A-Z` (two tile ranges, no
   alphabetical order); digits `80-89`; `! ? . , ' -` (`3F 3E 7C 7B 42 1C`), `…` (`40`), the two quote marks `7E` (opening)
   and `7F` (closing). No tile for `: ; ( ) + % & * = / # @` in the Latin range: `/` exists as `8F` (used by the menus, unused by the text).
8. The tile-code table `decode_tile` of `ssd-catalogue::text` is complete for this set and is one-way (decode); the compiler needs the
   inverse (a map char -> code, with `"` split into open/close) and two aliases that exist in the ROM: hyphen `43` (a copy of `1C`) and
   `7D` (two dots).
9. `FF` + leading space convention: lines after the first of a dialogue string start with a space tile `01` (the first line starts one
   cell to the left, so that the opening quote hangs out; the engine writes `message_indent = $8002`). Each page of a speaker repeats `F4 nn`.
10. Captions of the story transitions are ordinary messages of table 3 started by immediate ids in bank-4 routines (`LDY #id` then `JSL 00:89AC`);
    the ending's texts are **not** messages (own tile set, own terminators).
11. `00:89AC` (RTL, entry M8 X16) starts a message: `message_number = Y` (16-bit), state `$80`, word 0 (SEEN, 5 instructions).
12. The E0nn handlers (section 6) are shop, menu and event code, not text: a pack cannot add one without a wrapper on `01:D433`.

## 1. Where a message is read (SEEN)

| Step | Routine | What |
|---|---|---|
| start | `00:89AC` (99 call sites: bank 1 x15, bank 3 x47, bank 4 x35, bank 5 x1, bank `1A` x1; SEEN by a byte scan for `JSL`/`JML`) | `$072C = Y`, `$071B = $80`, `$072A = 0` |
| per frame | `01:CBE0` (RTL) | if `$071B` bit 7: handler `$072E` active -> `01:D433`, else the state in the low nibble (jump table `01:CBFC`) |
| next word | `01:D37A` (RTL), `01:D3DD` (first word, opens the window) | `01:D3C0` sets `$00-$02` = the list (word at `message_lists + 2 * message_number`, bank = `$073A`) and Y = `2 * message_word` |
| table select | `D0nn` | `$0733 = nn`; the next word, if its high nibble is 0, is the string index `$0723` |
| window | `01:D3DD`: `D002/D003/D004` -> entry `01:D42E[n]` = `08/09/3C` into `$0D66`, opened by `01:B72B`/`01:B67A` | `D001`, `D005`-`D007`: no window |
| text origin | `01:CC62`: `message_text_base` `$0721` = `01:CCB4[table]`, except table 7: `07:8F06[string]` | |
| string address | `01:CF3A` (every character): base = 3 bytes at `07:8F16 + 3 * table` (word, bank); pointer = word at `base + 2 * string`, same bank | |
| character | `01:CFB4` (called per character) | writes the tile word (code, `$24`) at `$7E:3000 + position` |

Message list words (SEEN: lists dumped for ids 0, 1, `$A7`, `$106`, `$77`, 367): `D0nn`, string index, `E0nn`, `FFFF`. A list may hold several
`D0nn` and several strings (id 367: table 1, six strings). The word array has 368 entries; the first list starts at `07:82E0`.
Examples in the structure, not the text: `D001 0058 E000 0059 E002 FFFF` (battle strings with handlers between them), `D002 000C FFFF` (one dialogue string).

`D0nn`: only `n = 2, 3, 4` open a window; table 1 (battle) is shown in the battle's own window by other code. The index word
`AND #$F000 == 0` test limits string indexes to `$0FFF`. Tables seen (indexes): 0: 42, 1: 338, 2: 366, 3: 167, 4: 9, 5: 2, 6: 2, 7: 8.

## 2. The string format (SEEN: codes of `01:CFB4`, raw strings dumped for str2:0, battle:9; `text-engine.md` has the full code table)

```
dialogue page    F4 nn  7E  <tiles...>  FF  01 <tiles...>  FF ...  7F  FE FF          next page: F4 nn 7E ...
last page        ... 7F  FF FF
narration        <tiles> FF <tiles> FF ... FF FF                                          (no portrait, no quote)
battle           [F4 nn mm] FB 01 <tiles> FF <tiles> FD FF FF                             FB = {name:1}; FD = close by itself
```

- `01` is a space: **the cell is skipped, nothing is written** (the tile under it stays what the window drew).
- `F4 nn` (battle: `F4 nn mm`): `$0D01 = nn` (the face), `message_indent = $8002`: the first line starts 1 cell left. The portrait
  ids met in the ROM's dialogue and narration: 44 distinct values, `00-0F`, `18, 1B, 1E, 22-25, 28, 2B, 2E, 31, 34-38, 54, 61, 6A-6F, 74, 76,
  7B, 80, 83, 91, 92, 96, EF` (SEEN, from the dump tokens). Which picture each is (face tables, `menus.md`) is not decoded here.
- Page control: `FE FF` ends a page (the engine waits for A, clears, starts the next page after the `FF`), `FF` ends a line, `FF FF` the string,
  `FD` the string with an automatic close (state 6 after a countdown). `E0`-`E7` as a tile code (inside text) = the inline phrase
  `inline_phrase_pointers[code & 7]` (`07:939E`).
- Codes `A0`-`DF` are translated (`01:BC88`) to tiles of the second set with attribute `$40`/`$41` (cards, kanji; not for text);
  `C8`-`CC` = fixed tiles `$10C`, `$10D`, `$10E`, `$150`, `$151`; `FC` = tile `$132`.
- The engine **does not wrap and does not check width or height**: each character goes to `message_text_base + 2 * column + $80 * line`
  (a line is two tile rows, `$80` bytes). A compiler must break lines itself and respect the budgets below.

## 3. Windows and budgets

Text origins (tile map word offsets from `01:CCB4`, `01:CCA2`; SEEN) and what the ROM's own strings use (`text-dump`, table `over` column and
a recount with `.work/m10-research/r456/lines.py`, tokens counted as zero cells):

| Table | Use | Origin (col, row) | Budget (cells) | Longest original line | Lines per page (original) |
|---|---|---|---|---|---|
| 0 | object help (Item screen) | (12, 23) | 16 | 16 | 1-3 |
| 1 | battle messages | (8, 24) | 17 | 22 (12 of 351 pages exceed 17; names add cells) | 1-4 (3: 45; 4: 3) |
| 2 | dialogue with portrait | (9, 20); first line (8, 20) | 20 | 26 (19 of 547 pages exceed 20; a leading space counts) | 1-5 (5: 13 pages) |
| 3 | narration, captions, townspeople | (4, 23) | 25 | 25 | 1-4 |
| 4 | dragon wishes | (17, 4) | not measured | 15 | up to 10 |
| 5, 6 | wish lines | (9, 20), (4, 24) | not measured | 17 | up to 11 |
| 7 | short system lines | per string (`07:8F06`) | 13 for string 6 | 26 | 1-3 |

The dialogue window is the box of columns 2 to 29; the 4-column portrait sits at its left; the text starts at column 9 (screenshots of
`story-1` frame 2616, narration, and `secret-raditz` frame 2317, dialogue: SEEN). The line pitch is 16 pixels. The box visible in the
screenshot of the dialogue fits about 3 to 4 lines; pages of 4 and 5 lines exist in the ROM (57 and 13 of 547) and I did not measure whether
they are clipped or the window is taller (not verified).

## 4. The English font and the printable set

The font is the UI tile set `08:8000` (LZ, Japanese) overwritten by the uncompressed English block `11:8036` (`$1800` bytes, 384 tiles of 2 bits)
after the decode (`graphics.md`). Rendered by `font.py` / `font2.py` to `font_sheet.png` (SEEN, viewed). The tiles used by the text:

| Group | Tiles |
|---|---|
| space | `01` (not drawn) |
| lower case | `a 05`, `b 07`, `c 09`, `d 0A`, `e 0C`, `f 0D`, `g 0E`, `h 11`, `i 15`, `j 16`, `k 17`, `l 18`, `m 19`, `n 1A`, `o 1B`, `p 1D`, `q 51`, `r 52`, `s 53`, `t 54`, `u 55`, `v 56`, `w 57`, `x 58`, `y 5B`, `z 5C` |
| capitals | `A E8`, `B 8C`, `C E9`, `D EA`, `E EB`, `F EC`, `G ED`, `H 8A`, `I 8E`, `J 04`, `K 8D`, `L EE`, `M 0F`, `N 50`, `O 5A`, `P 8B`, `Q 12`, `R 06`, `S 14`, `T 47`, `U 0B`, `V EF`, `W 10`, `X 08`, `Y 45`, `Z 13` (the `E8`-`EF` ones go through the "second set" translation of `01:BC88` but draw as plain capitals) |
| digits | `0`-`9` = `80`-`89` |
| punctuation | `! 3F`, `? 3E`, `. 7C`, `, 7B`, `' 42`, `- 1C` (also `43`: same glyph), `… 40` (a two-dot tile), `" 7E` open and `7F` close |
| extra glyphs that exist | `/ 8F` (slash), `7D` (two dots), `41` (a small mark) |

Usage counts in the ROM's own text (all eight tables, 29 783 tile bytes, `r456/t15.py`): `!` 629, `?` 123, `,` 188, `.` 426, `'` 316, `…` 273, quotes 791 + 781, `-` never (the one `-` of the
dump is in an attack name), `43` five times. **There is no glyph for `: ; ( ) + % & * = # @` and none for `/` in the text range**; a pack text that
needs them must be rewritten or drawn by the HD text pass (Enhanced only). Capitals J, Q, X, Z exist (`04 12 08 13`) and are rare in the ROM.
Tile `00` is a filled black cell and appears 37 times in the strings (a few lines where an `F4` was probably meant); never emit it.

`ssd_catalogue::text::decode_tile` (`crates/ssd-catalogue/src/text.rs`): SEEN, it holds exactly the table above except `43`, `7D`, `8F`, `41`;
its test `no_two_tiles_spell_the_same_letter` allows a duplicate only for space and `"`. For the compiler: (a) invert the table in
`ssd-mod::text` (do not edit the catalogue's one-way function), (b) pick `7E` for an opening quote and `7F` for a closing one, (c) reject
every character not in the set with the line and column.

## 5. Names, number slots, long names (SEEN)

`01:D0A1..D17D`: code `F6-F8` copies from `$1630/$1620/$1610` until a `FF`; `F9/FA/FB` = slots 3/2/1 = the words at `$1644/$1642/$1640` (the dump
calls them `{name:3}`, `{name:2}`, `{name:1}`; `FB` is the first; I first read the order backwards from a mis-decoded table and checked it against
the jump table words `D0DC D0CA D0B8 D10A D0FC D0EE` for `F6..FB`). A slot word is *index, type*: type bit 7 = unit (`02:ACE8`), bit 6 = object
(`02:B0D9`), bit 5 = attack (`02:AEA6`), bit 4 = `02:B204` (not decoded); the first match from the top wins. The index is doubled and used on the table
(so at most 256 names a table); the name is read from the word's address in bank 2: **new names for ids past the table's end need a wrapper on `01:D0EE`
region (`01:D115..D15B`) or on `01:D15B`** (the code writes `$1600 + n` without any bound).
After a name the state is 5 (`01:D17D`): the name is written by the character loop, then back to 4.

Name tables (SEEN, lengths read from the ROM): characters (`string_pointers`, first 12): 4 to 8 tiles; units: 3 to 8 tiles (105); attacks: 1 to 14 (the
two outliers 27 and 114 are the broken Japanese entries); objects: 4 to 7 (one of 12). The slot copy buffer ends at `$160F`, so **15 tiles is the
hard limit of a name**; the menus' own columns for a name (8 cells for units and characters) were not measured, so 8 is the safe limit for units.
The names' length byte is not padded (the dump's `Pa  ` padded names are the characters' two-tile-padded display in the menus).

## 6. E0nn handlers (SEEN: the table `01:D43B`, 52 words; classes read from the code of each, not decoded in full)

Dispatcher `01:D433`: `message_handler` (`$072E`) = `$80 + nn`; the handler runs every frame until it clears `$072E` (and ends with `JMP 01:D574`, `01:D56B`
or `01:CE37`). A wrapper on `01:D433` (JSL entry) can take `nn >= $34`.

| nn | Entry | What it does (first read) |
|---|---|---|
| 00, 01, 03, 05 | `D4A3`, `D4B3`, `D4FF`, `D55D` | close or restart the window; 05 marks a `$1000` entity |
| 02 | `D4D1` | picks message `$0B` or `$0C` by `$0EA2` bits (a branch of the battle result) |
| 04, 06 to 0C | `D513`..`D6D5` | the shop and yes/no dialogues: `$07AF` shop mode, zeni `$01B4`, item count `$01B2`, `$01D6`, a follow-up message through `00:89AC` |
| 0D | `D6F1` | sets `$0716` (fade request) |
| 0F | `D7B5` | sets `game_mode` |
| 10, 11, 13, 17, 18, 1C, 1E | `D7C2`, `D7CF`, `D7EC`, `D845`, `D84A`, `D8E8`, `D91F` | **start a scene** (`$01D8` and `$1648 = $80`): 10 the fixed scene 5, 11 the scene from the message number, 13 scene 4, 17/18 its neighbours, 1C map id - 2 (`01:D8FB`), 1E scene `$3C` |
| 14 | `D7F9` | adds a word (`$0D53`) to the zeni (the cheat wrapper `MONEY_ENTRY`) |
| 12, 15, 16, 1A, 1D, 22, 24, 30, 32, 33 | | flag and item bookkeeping that ends with `JMP 01:D56B` |
| 19, 1B, 1F, 27 | `D84F`, `D8BF`, `D92C`, `D9E9` | give or take items and flags (27 calls the item use routine `01:C639`) |
| 20, 21, 2A, 2B, 31 | `D94C`, `D969`, `DA32`, `DA6C`, `DB8A` | change `game_mode` and `$0716` (a mode switch from a message) |
| 26 | `D9E0` | `JSL 03:F7DE` (start a scene whose id is in A) |
| 2C | `DA7F` | start another message |
| 2D, 2E | `DA96`, `DB44` | write the number slots `$1610/$1612` (the numbers in later text) |
| 0E, 23, 25, 28, 29, 2F | | not classified |

The plan's `message.var` and other extensions therefore need either a new `E0nn` (wrapper `01:D433`) or a pack-side number-slot write. The number slots
are plain WRAM bytes: a wrapper on a scene command can fill `$1610..` before the message; no new handler is needed for `{number}`.

## 7. Can a message address another bank? (the question of the extended image)

| Piece | Reach | How |
|---|---|---|
| word array of message lists | **24 bits**, bank free | `$0738-$073A` are variables set by code (`00:8078` at boot, `05:97B2` after the battle, `05:9391` for the battle at `07:D79E`); `01:D3C0` reads them |
| a message list | same bank as the array (16-bit word from the array, bank of `$073A`) | so the list and its array travel together |
| string table pointer | **24 bits** (`07:8F16`, three bytes each), table 2 already in bank 6 | the table of 8 pointers is ROM data of the image: patch an entry for a replaced table (class A) |
| strings | same bank as their word array | a replaced table must carry its strings |
| name tables | **bank 2 fixed** (`$02` stored by code) | wrapper |
| window offsets | `01:CCB4`, 8 words | wrapper for new table ids |
| portrait ids | `F4 nn`, face table | not traced |

Practical routes for Task 4 (class B, wrappers): (1) message ids `>= 368`: wrapper on `01:D3C0` (all messages) or on `00:89AC` setting `$0738-$073A` for the
message's lifetime; (2) strings for ids past a table's end: either a new table id `>= 8` (wrapper on `01:CF3A` and the window origin) or a merged
copy of the table built at load from the player's ROM and the pack, placed in an extended bank, with the pointer at `07:8F16 + 3 * n` patched (the merged copy
contains ROM bytes, so it exists only in memory); (3) names: wrapper on the name copy (`01:D15B`).

## 8. Captions and the ending (SEEN)

- The captions of the story transitions (mode 7, bank 4) are messages: each routine calls `00:89AC` with an immediate id (examples `$2F`, `$47`, `$AF`
  at `04:A0FF`, `04:A34F`, `04:D0FC`), table 3 strings, shown over the black screen by the same engine. The tables `04:9A0E`, `9A2A`, `9A46`, `9A62`
  (28 rows each, indexed by `story_step`) hold the effect byte, next `game_mode`, next story step and next map id, **not text** (SEEN, `04:997C..99E6`).
  So a new caption is a new message plus the immediate in a bank-4 routine (class C): the pack can only reuse an existing transition routine with a changed message
  (wrapper on `00:89AC`).
- The ending (mode `$11`) uses `00:B013`, with five texts at `00:BD87`, `00:BEC7`, `00:BFB4`, `00:BFF0`, `00:C052`, written by `00:BA73` from the pointer in `$7B` with the
  **title tile set** (`00:DF39`: digits `80-89`, capitals `DA-FD`, comma `8C`, apostrophe `42`): other code table, not the message font. Out of scope for the compiler.

## 9. What the compiler (Task 4) should do, in one place

1. Source units: pages of lines, speaker (portrait id), table choice (`talk`, `narration`, `battle`, `system`), name slots (1 to 3) and number slots (1 to 3).
2. Encode with the inverse of the table of section 4; reject the glyphs without a tile with `line:column`.
3. Break lines at the budget of the table (16, 17, 20, 25; 20 includes the leading space of continuation lines and counts the quote of the first line);
   at most 3 lines (battle), 4 (dialogue) and 3 (narration) per page unless the window height is measured; end each page `FE FF`, the string `FF FF` (or `FD`).
4. Dialogue string: `F4 nn`, `7E` ... `7F`; continuation lines start with `01`; the quote pair is per speech, not per page.
5. Message list: `D00t`, string index words, `FFFF`; ids from 368 (wrapper `01:D3C0`).
6. Names: at most 15 tiles (8 for units); slot word = `index`, `type`.
7. Lint: width, lines, glyphs, name length, string index `<= $0FFF`, message count, and the original's own overruns (19 dialogue pages, 12 battle pages exceed their budget:
   a pack that replaces them can be stricter).

## 10. Not verified here

The portrait ids' pictures; whether pages of 4 and 5 lines are clipped by the window; the menus' columns for names; `E0nn` semantics beyond the classes; the `02:B204` name
table; the talk programs (`03:9DF1`, per object); the text-speed control (`$01D0`); whether HD text replaces the font in Enhanced for pack strings.

## 11. Runs

`xtask text-dump .work/m10-research/r456/strings.tsv` (1458 rows; summary: help 42, battle 338, str2 366, str3 167, str4 9, str5 2, str6 2, str7 8, phrase 8, ending 5, string
44, unit 105, attack 69, object 40, block 80); `xtask shots story-1 --frames 2616 --peek message_table,message_string,message_state` (table 3, string `$3E`, state `$82`);
`xtask shots .work/replays/secret-raditz.ssdreplay --frames 2265,2317 --peek message_table,message_string,message_number,message_line` (table 2, string `$0C`, message `$A7`);
`--watch message_table,message_state` on `secret-raditz` (table 2 at frame 2241, table 1 at 2671).
