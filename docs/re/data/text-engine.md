# The text engine: messages, string tables and what is on screen

What a presentation-only text override (a translation drawn over the original's text) needs: where every string the game
prints is, how the message engine reads it, and which bytes of WRAM say, while a string is being written, which string,
page, line and window it is. Read from banks `$00`, `$01`, `$05` and `$07` with `xtask disasm` and checked headless with
`xtask shots ... --watch` on the `story-1` and `battle` scripts. Certainty: "seen" = a run shows it, "code" = read in the
code only. Variables and tables are named as in `re/symbols.toml`; routines without a symbol are written bank:address. This
page holds addresses only: `xtask text-dump` writes the strings themselves, decoded, to a file outside the repository.

This builds on `names.md` (the tile codes), `menus.md` (the window engine) and `gameplay.md` (the message states and the text
speed).

## The pieces

| What | Where |
|---|---|
| The engine | `01:CBE0`, run once a frame. `message_state` (`$071B`) bit 7 = a message is up, the low nibble the state (jump table at `01:CBFC`) |
| Start a message | `00:89AC` (JSL, Y = the message number): `message_number` (`$072C`) = Y, `message_state` = `$80`, `message_word` (`$072A`) = 0. The scenes, the bank-4 story transitions and the map's Talk and Look come here; `01:B205`, `01:99CD` and `01:D4ED` set the same bytes themselves |
| The lists | `message_lists` (`$0738`, 24 bits) points at a table of words, one per message, each the address (same bank) of the message's *list*. Set to `07:8000` (`message_pointers`, 368 messages) by `00:8078` and `05:97B2`, and to `07:D79E` by the battle (`05:9391`) |
| The string tables | `string_table_pointers` (`07:8F16`): eight 24-bit addresses (word, then bank), one per `message_table` value |

### A message's list

Words, read by `01:D37A` (`message_word` counts them):

| Word | Meaning |
|---|---|
| `D0nn` | `message_table` (`$0733`) = `nn`; the next word is the string. At the start of a message `01:D3DD` also turns `D002`, `D003`, `D004` into the window entry `$08`, `$09`, `$3C` (table at `01:D42E`) in `map_entry_index` (`$0D66`) and opens it |
| `E0nn` | runs handler `nn` (`message_handler` = `$80 + nn`, `01:D433`, table of 52 at `01:D43B`; `scenes.md` lists some) instead of a string |
| `Fxxx` (`FFFF`) | the end: the window closes (`01:CE03`) |
| any other | a string: its index in the current table goes to `message_string` (`$0723`, a word) |

After the last page of a string the press of A (state 3, `01:CDDB`) goes back to state 0, which reads the next word: a
message can be several strings, of several tables.

**The battle's messages** go through a list in WRAM: the word at `07:D79E` is `$14E2`, and bank `$07` addresses below
`$2000` are the first 8 KiB of WRAM. `05:9391` writes `D001` and `FFFF` into `battle_message_list` (`$14E2`) and `05:93A9` the string
index between them (`$14E4`); seen in the `battle` replay (the string index changes at `$14E4` before each turn's message).

**The Item screen's help** (`01:8B20`): `message_table` = 0 and, with `message_state` bit 6 set, `01:CC43` takes the object id
(`menu_item_id`, `$0D7C`) as `message_string`; there is no list (code).

## The string tables

| `message_table` | Table | Strings | What | Text from (tile column, row) | Width (cells) |
|---|---|---|---|---|---|
| 0 | `item_help_pointers` `07:8F2E` | 0 to 41 | the help of the objects (0 to 39), two menu lines | `$05D8` (12, 23) | not measured |
| 1 | `battle_message_pointers` `07:8F82` | 338 | the battle's messages (the same words as table 0 from its index 42) | `$0610` (8, 24) | 17, seen (the window is columns 6 to 25) |
| 2 | `talk_string_pointers` `06:8000` | 366 | dialogue with portraits (`D002`, window `$08`) | `$0512` (9, 20) | 20, derived (to the right edge of table 3's box; not measured) |
| 3 | `story_string_pointers` `07:9226` | 167 | narration, the story transitions' captions, townspeople (`D003`, window `$09`) | `$05C8` (4, 23) | 25, seen (the box is columns 2 to 29) |
| 4 | `wish_string_pointers` `07:9374` | 9 | the dragon scenes (`D004`, window `$3C`) | `$0122` (17, 4) | not measured |
| 5, 6 | `wish_string_pointers_5`, `_6` (`07:9386`, `07:938A`) | 2 each | | `$0512`, `$0608` | not measured |
| 7 | `system_string_pointers` `07:938E` | 8 | short lines whose window is placed per string (`system_string_offsets`, `07:8F06`) | per string | 13 for string 6, seen |

The text offset is `message_window_offsets` (`01:CCB4`, a word per table) except for table 7, where `01:CCA2` takes
`system_string_offsets[message_string]`. Offsets are bytes into the tile-map buffer (`$7E:3000`, 32 tile words a row); a line
is two tile rows (`$80` bytes). The counts are where the next table starts; every pointer leads to a string.

**Correction to `cards.md`.** The favourite-card message (the window beside the card under the pick cursor) is string 6 of table
7 (message `$30`, seen: `message_table` 7, `message_string` 6, `message_text_base` `$05E0` at frame 4766 of the `battle`
script), not an entry of a table at `07:92F8` (that address is inside `story_string_pointers`).

### A string

A string is tile bytes and codes, read by `01:CFB4` (`counter_071d`, `$071D`, is the offset of the next byte):

| Code | What |
|---|---|
| `01` | a space: the cell is skipped, not written |
| `FF` | the line ends; `FF FF` the string ends (state 3, wait for A) |
| `FE FF` | the page ends (state 2, wait for A; the next page starts after the `FF`) |
| `FD` | the string ends and the window closes by itself after a countdown (bit 4 of `message_state`, state 6) |
| `F4 nn` | portrait `nn` (`01:D293`, `$0D01`); in table 1 one more byte (`$07B6`). The first line then starts one cell to the left (`message_indent` = `$8002`: an opening quote hangs outside the text) |
| `FB`, `FA`, `F9` | a name from the slot words at `$1640`, `$1642`, `$1644` (`message_name_slots`): the first byte the index, the second the type, bit 7 a unit (`unit_name_pointers`), bit 6 an object, bit 5 an attack, bit 4 the table at `02:B204` (not decoded). Copied to `message_name_buffer` (`$1600`) and written in state 5 (`01:D0A1`) |
| `F8`, `F7`, `F6` | the `FF`-ended tiles at `$1610`, `$1620`, `$1630` (`message_number_slots`: numbers, such as the damage) |
| `FC` | the tile `$132` |
| `E0`-`E7` | the string `inline_phrase_pointers[code & 7]` (`07:939E`) in place (`01:D18E`); only the first is a phrase of its own |
| `C8`-`CC` | the tiles `$10C`, `$10D`, `$10E`, `$150`, `$151` (table at `01:D097`) |
| `A0`-`DF` | a tile of the second set (`01:BC88`) |
| other | the tile of that number (`names.md`; `E8`-`EF` are capitals) |

Several strings have lines longer than the window measured for their table (`text-dump` counts them in its `over` column);
one battle page starts with `00` where `F4` was meant. These are the patch's, the engine writes them as they are.

## Which string, page, line and window is on screen

While `message_state` bit 7 is set and its low nibble is 1 (writing), 2 or 3 (a page waiting for A), 5 (writing a name) or 6
(a timed wait), these bytes identify what the window shows:

| Question | Bytes |
|---|---|
| Which string | `message_table` (`$0733`) and `message_string` (`$0723`, word): the coordinates of `xtask text-dump`'s key (table 0 index 42 and up is table 1 index - 42). The message around it: `message_lists` (`$0738`), `message_number` (`$072C`) and `message_word` (`$072A`): the string is word `message_word` - 1 of the list |
| Which page | not stored: the number of `FE` codes in the string before the byte offset `counter_071d` (`$071D`). In state 2 the offset is already past the `FE FF` of the page on screen, so subtract one |
| Which line and cell | `message_line` (`$071E`, 0 first) and `counter_071f` (`$071F`, the cell in the line); `01:CF3A` writes the next tile at `message_text_base + $80 * message_line + 2 * counter_071f` (plus or minus `message_indent`) |
| Which window | `message_text_base` (`$0721`, word): the tile-map offset of the text's first cell, fixed per table (per string in table 7). For `D002`-`D004` lists the window entry is in `map_entry_index` (`$0D66`: 8, 9 or `$3C`) |

When a string starts (state 0 after a page or a string, `01:CC0A`) `message_string` changes only after the window has been
cleared and an 8-frame pause (`$0725`), so in state 0 and in the first frame of `$80` it still holds the previous string.
An override should therefore read the bytes in states 1, 2, 3, 5 and 6 only. `message_state` bits 5 and 6 (`$A0`, `$C0`)
are the instant messages that write a whole page in one frame (the Item screen's help, the favourite card).

Seen, `story-1` (and the same frames of `battle` and `menus`, which start with it): at frame 2580 message `$2F` starts
(`message_number` `$2F`, window `$09`), at 2587 the first string (`message_table` 3, `message_string` `$3E`,
`message_text_base` `$05C8`), lines 1 and 2 at 2603 and 2613, the page wait (`$82`) at 2616 after three lines, the next pages
at 2625 and 2673, the last page wait (`$83`) at 2701; the strings `$3F` and `$40` follow at 2817 and 2913 with `message_word`
3 and 4. The decoded string `str3:0x3E` has exactly those three pages of three, two and three lines. `battle`: the turn
message at frame 4996 (`message_lists` `07:D79E`, `$14E4` = string `$95`, `message_table` 1, `message_text_base` `$0610`,
the slots `$1641` and `$1643` = `$80`, units), states `$85` while the names are written, `$93`/`$96` the timed close.

    cargo run --release -p xtask -- shots story-1 --upto 3000 --every 3000 --no-png \
        --watch message_state,message_number,message_word,message_table,message_string,message_line,map_entry_index

## Other text

| What | Where |
|---|---|
| Names | `string_pointers`, `unit_name_pointers`, `attack_name_pointers`, `object_name_pointers` (`names.md`): a length byte, then tiles; the slot codes above write them |
| Menu words | `bank2_pointer_table` (`02:DA16`): a block is the width - 1, the height - 1, the tile-map offset (word) and a tile byte per cell, rows `$80` bytes apart (`00:9C91`; `00:9CD8` reads a tile and an attribute per cell instead). Entries 0 to 79 are blocks (from 80 the words point into code); the text blocks are the ones with an offset and Latin tiles |
| The ending (mode `$11`) | five texts in bank 0 at `00:BD87`, `00:BEC7`, `00:BFB4`, `00:BFF0` and `00:C052` (the immediates of the calls to `00:B2B1` at `00:B071`, `00:B25C`, `00:B3DD`, `00:B45F`, `00:B4BE`), written by `00:BA73` from the pointer at `$7B`: `FF` a line, `FF FF` the next screen, `FF 00` the end. Not the message engine |
| Not reached | all-capital English text at `06:C6B4`-`06:D0CF` and `07:CB6B`-`07:EF00` that none of the tables above points at (apparently an earlier translation left in the ROM; no other reader was searched for) |

## `xtask text-dump`

    cargo run --release -p xtask -- text-dump [out.tsv]

writes every string of the tables above, one row per page, to `out.tsv` (default `.work/text/strings-en.tsv`, which git ignores) and the
lists of the 368 messages as the keys of their strings to `messages.tsv` beside it, and prints per table the entries, rows,
failures (a pointer outside the ROM, a string without an end), rows with an unknown tile and lines longer than the window.
The columns, the keys and the tokens are in `xtask/src/text_dump.rs`. Keys: `help:N`, `battle:N`, `str2:0xNNN`, `str3:0xNN`
to `str7:0xNN`, `phrase:N`, `ending:N`, `string:N`, `unit:0xNN`, `attack:N`, `object:N`, `block:0xNN`, with `/p2`... for
the pages after the first; a string at the address of an earlier one says `{same:KEY}`. The files are derived from the ROM:
never commit them.
