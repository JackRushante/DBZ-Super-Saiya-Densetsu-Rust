# Menus, items and the shop

How the map menus, the Item screen and the shop work, found for milestone M4 (HD menus). Everything was read from the
code of banks `$00`, `$01`, `$02`, `$03` and `$05` and checked by playing the reference replays (`menus`, `shop`) and
crafted saves headless (`xtask shots`, `save-edit`, `replay-from-save`; the commands are at the end). Certainty: "seen" =
a run shows it, "code" = read in the code only. Tables are named as in `re/symbols.toml`; routines have no symbol (the
file only holds routines that have a Rust function) and are written as bank:address. Names of things the game draws
(Talk, Look, Item...) are the English patch's.

**Corrections to earlier notes.** The item list holds **24** entries, not 10 (`item_list` is `$0100`-`$012F`; three pages of
eight on screen; pickups, the shop and the battle drop refuse the 25th). `xtask save-edit --set items=` and `save-dump` stop
at ten, which is where the "ten" came from. Seen: a crafted save with 24 entries shows all three pages.

## Input

The NMI handler calls the joypad reader (`00:82D4`, called at `00:8120`): it waits for the auto-read to finish, then for each
pad stores the held buttons, the buttons newly pressed this frame (held now and not in the previous frame) and keeps the
held value for the next frame. Each pad is a 16-bit word; the low byte is `$4218` (A, X, L, R in bits 7 to 4) and the high
byte `$4219` (B, Y, Select, Start, Up, Down, Left, Right in bits 7 to 0).

| Pad 1 | Held | Newly pressed | Previous |
|---|---|---|---|
| word | `$43` (`$44` = `source_byte_0044`) | `$47` (`$48` = `source_byte_0048`) | `$4B` |

Pad 2 is the same at `$45`, `$49`, `$4D`. Menus read only the newly-pressed bytes and have no auto-repeat: `$47` bit 7 is A
(confirm), `$48` bit 7 is B (cancel), `$48` bit 3 up, bit 2 down, bit 1 left, bit 0 right, bit 4 Start. A press is seen by
the frame handler of the frame it happens in. The replays hold a key for 4 frames and wait 14 (the timeline files); seen:
a hold of 1 frame and a gap of 1 frame move the cursor and confirm just as well (the same screen, byte for byte), so
presses can be fed fast.

## Where the menus run

The map has three modes that matter (`game_mode`, `$24`): `$06` the field (a chapter map walked or flown), `$01` a town or
interior (the shop), `$02` a battle. The field's frame handler is `00:984F`; it calls the story, the object and the menu code
every frame. Two bytes of the direct page say what the field is doing:

| Byte | Meaning |
|---|---|
| `$25` | 0 free (the party walks or flies), `$0F` the menu is opening (about 18 frames), 1 a menu is open, 3 and 5 other holds (a landing, a scene; `$1648` bit 7 forces 5) |
| `$26` | which routine the frame handler runs: 0 the menu/field step, 1 closes the menu (`00:98B9`: clears `$25`, `$26`, `$0D00` and `clear_map_menu_block`) |
| `$61` | the context: 0 the map, 1 a battle, 2 the second battle kind (set by the mode routines at `00:8F72` and `00:91B8`; zeroed at `00:8BD0`) |

**Opening** (`00:97FD`, seen): when `$25` is 0 and A is newly pressed, the routine sets `$25` and `$27` to `$0F`, clears the
cursor (`map_entry_value`, `map_entry_0d63`), clears the menu block (`clear_map_menu_block`), plays sound 2 (`04:85B6`) and
refreshes the attribute bytes of the item list (`01:C535`). From then on the menu is a state machine run by `01:881F` each
frame. The field does not move while the menu is open; the map behind it does not scroll.

**The menu state machine** keeps its state in a few bytes of bank 0:

| Byte | Meaning |
|---|---|
| `$0D5F` | the *screen*: an index into a jump table at `01:88AE` (30 words), see below |
| `$0D68` | the step inside the screen (each screen has its own table, dispatched by the routine at `01:BF44`) |
| `$0D60` | 1 while the screen is being drawn or an effect runs; the frame handler then skips the sound for an unused press |
| `$0D62`, `$0D63` | the cursor (`map_entry_value`) and the page (`map_entry_0d63`) of the list being worked |
| `$0D66` | the *window entry* being drawn or touched (`map_entry_index`) |
| `$0D7C` | the id of the object chosen (the item in use) |
| `$0C3F` | the cursor inside the page (written when A is pressed on an item) |
| `$1100`... | one cursor byte per window entry (`map_menu_cursor`; 96 bytes, bit 7 = redraw the cursor) |

Screens (`$0D5F`), seen unless marked:

| `$0D5F` | Handler | What |
|---|---|---|
| 0 | `01:8908` | the command menu: Talk, Look, Fly, Item, Menu (flying: Land, Item, Menu; a town: Talk, Look, Item, Menu, on screen 13) |
| 1 | `01:89C7` | Fly: the take-off (steps 0, 1, 2, 4, 5, 6, about 60 frames); `travel_mode` becomes 2 and the menu closes by itself (seen, M4 task 2) |
| 2 | `01:8A8C` | the Item screen (steps below); the battle's Item command opens the same screen with `$61` = 1 |
| 3 | `01:894A` | the screen after the last A of a Talk or Look message (the message itself is drawn on screen 0 with `$0D60` = 1), then the field (seen) |
| 4 | `01:8F4B` | the Menu list: Cards, Status, Order, Text, Save |
| 5 | `01:8F81` | Land: the landing, with the same steps as 1; `travel_mode` becomes 1 and the menu closes (seen, M4 task 2; not a use-on-member list) |
| 6 | `01:903F` | Cards |
| 7 | `01:90C4` | Status (the full-screen status page, `game_mode` 3) |
| 8 | `01:939B` | Order (the party order; see below) |
| 9 | `01:96AD` | Text speed: five speeds, the choice is saved in `$01D0` (Left and Right move the cursor `$0D62`, 0 to 4; the A that chooses stores it in `$01D0` and the menu closes through screen 3; seen headless) |
| 10 | `01:96CD` | Save: the question (step 0), the slot list (step 1, the slot in `$0D62`), `03:E556` (the battery save of `save-format.md`) on the A that chooses the slot, then "Completed!" (step 2). The Enhanced mode's HD Save screen asks neither question: it answers step 0 with the original's own A (Yes is where the cursor starts), moves the slot list's cursor with Down and presses A (`menu-test diff menu-save`: the same work RAM and battery save as the original's own presses) |
| 13 | `01:970F` | the town command menu (Talk, Look, Item, Menu: no Fly); the shop runs on top of it |
| 17, 21 | | battle command and the battle's item effect (seen: 17 with the battle command window, `$15` while a battle item works) |

The other numbers of the table (11, 12, 14 to 16, 18 to 20, 22 to 29) are the battle's and the story's small screens; not
decoded. The command menu's choice goes through `01:BE70`: the table at `01:89B6` turns the cursor (Talk, Look, Fly, Item,
Menu) into a *window set* (0, 1, 80 = fly, 2, 3); Item with no items opens set `$25` (a "no items" message) instead.

### Order (screen 8)

Four steps (`$0D68`): 0 copies the nine `party_records` to `$7E:3800` and clears the slot count `$0D6B`; 1 takes the picks; 2 asks
Yes or No; 3 returns to the field. The list's rows are the party records in their slots (`$0D62` is the row of the cursor, the size
of the party is `$01BB`). On step 1, A copies the record under the cursor into the next slot of the copy (slot `$0D6B`), clears
bit 7 of its flags (a row taken: the cursor skips it from then on, in both directions), moves the cursor to the next row left
and counts the slot. When one row is left it is taken by itself and step 2 starts (the question's cursor is on Yes, `$0D62` = 0).
B takes back the last pick (the count goes down, the row's bit 7 comes back, the cursor stays where it is) and leaves the screen for
the Menu list when no pick is left. On step 2, A on Yes copies `$7E:3800` back over `party_records`, clears the records past the party
(`01:967A`), redraws the corner portraits (`01:C715`) and goes to step 3, which returns to the field (the menu closes); A on No
gives every row its bit 7 back and starts step 1 again; B takes back the last two picks. The record order is the party order the
game plays with. The list `party_order` (`$0190`) is not touched: the game writes it when it saves (`03:E4D1`), so a reader of the
party in play reads the records (`ssd_game::menu::MenuProbe`). Seen on a party of two and on a staged party of five (`menu-test field menu-order`).

### The window engine

Every box on screen is a *window entry* (0 to `$4E`), drawn as a block of tiles into the tile-map buffer `tilemap_buffer`
`$7E:3000`... which the NMI copies to VRAM (`$7E:4000` is the second buffer). `01:BE70(set)` appends a *window set* (a list of
entry numbers, bank 2 at `$E322`, 49 words, each pointing at a count byte and the entries) to a queue at `buffer_4800`
(`$7E:4800`); `01:A48F` draws one entry per frame, and when the queue is empty it sets `$0D67` bit 7 and takes the next screen
number from `01:B50A` (entry number to screen). Each entry has a *handler* (word table at `01:A4F5`, indexed by the entry
number: a draw routine, a cursor routine or both) and a *geometry record* of three bytes in `map_record_table`
(bank 2 `$E232`: the number of choices, bit 7 = the count is a variable, and the tile-map offset of the first row).

The text blocks (frames, words, digits) are in `bank2_pointer_table` (bank 2 `$DA16`, 128 words): a block is a width, a height
and tile pairs (a tile number and its attribute), blitted into the buffer by `00:9C67`, `00:9CD8` and `00:9CDE`. Seen:
block 0 the five commands, 1 the five Menu entries, 2 the flying menu, 3 Yes/No, 4 the Text speed list, 6 the INT/EXT/AID
label, 7 Use/Drop, `$32` "this item is not usable", `$33` "this item is useful", `$34` "can't use / you are at max".

Entries met in the field and the Item screen (seen from the order the Item screen draws them):

| Entry | What |
|---|---|
| `$00` | the command list (5 rows) |
| `$17` | the Menu list (5 rows) |
| `$02`, `$14` | the party list (rows = party size, `$01BB`) with HP, KI, BP columns: the "use on" list |
| `$0B` | the item list (8 rows a page; `$01B3` rows are filled on the last page) |
| `$0C` | the INT/EXT/AID label with its three arrows |
| `$0D` | the portrait of the chosen object and its price box |
| `$0E` | the Use / Drop window |
| `$15`, `$16` | the message and the confirmation inside battle |
| `$18` | the Zeni window (`string_pointers` entry 22: the word, the money, a `P`) |
| `$08`, `$09` | the Talk and Look message boxes |

The money is shown in **Pt**: the `P` after the zeni (300P) is the unit, and every price below is in the same unit.

## The Status screen

Screen 7 (`$0D5F`) has two pages: page 1 lists the party (steps 0 to 2: the nine record slots, three columns of LV, HP, KI and
BP with the corner picture), and A on a member opens page 2 (his figures and his techniques), where B comes back to page 1. The
HD screen (`ssd-game`'s `menu/status.rs`) shows both at once and reads the party records, so it never presses A: the original
sits on its page 1 while the HD cursor moves, and B (or Start) is a plan of the original's own B presses back to the Menu list
(and out). What page 2's figures are (SEEN, compared on the reference capture of the party with Goku at level 47, Gohan at 40):

| Shown | Source |
|---|---|
| LV, HP, KI, BP | the record's level, HP, KI and 24-bit BP (`characters.md`) |
| PLA. | the record's byte `+0E` (the byte copied from the level row; 8 for Goku, 6 for Gohan in the capture). What it counts is not known; the label is the original's |
| EX | the record's EXP, `+0F` to `+11` |
| EX LV UP. | the EXP at which the next level begins (the level row after the current one, `character_level_rows`) less the EXP; 0 at the character's level cap (`03:850E`: 47 for Goku, whose 48th row, 3370000, is no level), SEEN on Goku at 47 |
| the kanji under the picture | the record's byte `+1B`, the class, the same number as a card's symbol (`cards.md`) |
| the technique lines | the unit's command list (`unit_command_lists`, led by the record's unit id `+01`): the slots after the plain attack, a blank slot (`$FF`) skipped, each named by `attack_name_pointers` |
| KI-USE | the command cost table `03:884B`, one byte per command id (not a named symbol: read at one place, `03:87CC`) |

## The item list

`item_list` is 24 entries of two bytes from `$0100`: the object id and an *attribute byte*. `item_count` (`$01B2`) is how many
are in use (0 to 24; the screens, the shop and the save read it). The list is **not sorted and not stacked**: two Bulmas
are two entries. Slot `n` also has a word at `$7E:2BA0 + 2n` (`item_extra`; 48 bytes, saved with the image): it follows
the entry when the list is compacted; the objects that read it are the wish-granting object (Porunga: a count of wishes
left) and the card-changing ones (a mask of the three choices already used, `01:F2C7`). Added by `01:C671` (the id comes from `$1640`; the caller has
already checked the room; it writes the object, the attribute byte and the count, not the state word, so a new entry keeps the word its slot held), removed and compacted by `01:C639` (called by 12 effect and sale routines; the compaction at `01:C654` moves the state words with the entries).

Who adds: the field object pickup (`01:D6D5`: refuses at 24), the battle drop (`05:8F4D`: refuses at 24, up to three drops in
`$14F6`) and the shop (`03:A914`: refuses at 24 with "you can't carry any more"). The story reads the list too (the Dragon
Balls, the scenes that give or take an object), so the list stays the source of truth for the original code.

### The attribute byte (INT, EXT, AID)

`item_attributes` (`01:C584`, 40 bytes, one per object id) is copied into the second byte of each entry by `01:C535`
whenever a menu opens. The bits (seen on the screen and by using items):

| Bit | Label | Meaning |
|---|---|---|
| 7 | **INT** | usable **in a battle** (the label on the Item screen is INT; code: `$61` non-zero) |
| 6 | **EXT** | usable **on the map** (outside a battle; `$61` = 0) |
| 5 | **AID** | an object that is only helped by the story (the Dragon Balls, the story's Gohan): never usable from the menu |
| 4 | | the effect asks for a card of the hand (Yemma, Baba, ChiChi and Oolong change cards) |
| 3 | | the effect asks for a party member to use it on (healing, stat boosts) |
| 2 | | cannot be dropped |
| 1 | | a variant of 3 and 4 (an object that works on everyone or on the enemy) |
| 0 | | set at run time by `01:C535`: "not usable now" |

`01:C535` sets bit 0 when the entry's usable bits (7 and 6) equal only the one that is wrong for the context: on the map
(`$61` = 0) an object with INT only gets bit 0, in a battle an object with EXT only does. An object with both is usable in
both. Seen, on the map: Bulma (both) heals; Yajirobe, Vegeta (INT only) and a Dragon Ball answer "can't use"; the Scouter and
Bubbles (EXT only) work. Seen, in a battle: the Scouter (EXT only) answers "can't use", Yajirobe (INT only) goes on to the
battle's effect screen. The attribute of the story's Gohan object is replaced by "INT, cannot drop" when the encounter group
is 5 (the first Raditz battle), where he can be called. The three arrows on the Item screen show the bits that are set on the
chosen object (the arrow is drawn by `01:C5AC` for each of bits 7, 6, 5).

### Where names, portraits, prices and effects come from

| What | Where |
|---|---|
| Name | `object_name_pointers` (`02:B0D9`, 40 entries, `names.md`): the objects are the story's people. The English patch repeats and misses a few (the same name twice at 16/17 and 24/25, a quote-marked "Kaio") |
| Help text | a word table at `07:8F2E` (indexed by object id; the table goes on with the battle messages after id 39): pages of text of up to three lines, lines ended by `$FF`; `$FD`, `$FA`, `$FB`, `$F8`, `$FC` are slots a message fills in |
| Price (Pt) | `item_prices` (`01:CB8B`, 40 bytes, `$FF` = no price): bought and sold at the same figure (`03:A797` adds it to the zeni when selling) |
| Effect | `item_effect_table` (`01:DC1E`, 33 words for ids 0 to 32): run by `01:DBDC` every frame while `$073B` bit 7 is set |
| Portrait | graphics in the ROM, compressed (below): `object_portrait_pointers` (`02:A4EF`, three bytes an object) and `object_portrait_palettes` (`02:A356`) |

Seen prices: Bulma 3, Puar 4, Roshi 6, MrPopo 10, Shenron 60. The price box is drawn by `01:CACB` with the figure from the
price table; the objects without a price (Dragon Balls, Korin?, the story objects) show a small kanji for "aid" in the
box instead of the figure. The Porunga object shows the number of wishes it has left (`item_extra`) in the same box.

**Portraits (found by the M4 spike).** The picture of object `n` (0 to 39) is at the 24-bit pointer of `object_portrait_pointers`:
three bytes an object, the word first and the bank byte last. `01:8B00` stores the chosen object in `$0D01` and sets `$0D75`
to `$90`; the routine at `01:C98E` reads the pointer of that id from the table `$0D75` selects, `01:CA98` sets `$83`/`$85`
from it, and `00:C559` decompresses it into `$7E:9000`. The stream: a 16-bit output size (1024 bytes for every object), then groups of eight items
led by a flag byte read least-significant bit first. A set bit is a literal byte. A clear bit is a two-byte back reference:
the low nibble of the first byte is the length code, and the copy is that code plus three bytes (the routine stores the code
plus two and its loop copies one byte more); the other twelve bits, the high byte and the high nibble of the first, are the
distance back from the output. A source before the start of the output reads as zero. `00:C68C` then regroups each 16-byte
block from bitplane pairs into the SNES tile layout, so the 1024 bytes are 32 tiles of 4 bits a pixel, 32 bytes each.
The first picture of a set is tiles 0 to 7 and 16 to 23 in a 4 by 4 grid, rows `0 1 2 3`, `16 17 18 19`, `4 5 6 7` and `20 21 22 23`
(32 by 32 pixels); tiles 8 to 15 and 24 to 31 make a second picture in the same arrangement. **Which of the two an object shows**
is one bit per object in a table of five bytes, `object_portrait_halves` (`02:A58F`, least significant bit first: object `n` is
bit `n mod 8` of byte `n / 8`; `01:C9ED` reads it with the object id shifted right by three): 0 the first picture, 1 the second.
Two objects usually share a set and take one half each, not in the order of their numbers (objects 10 and 13 share one: Yemma
shows the second half and Oolong the first; 23, 24 and 32 share one: the Dragon Radar shows the second half, the frog the
first). Checked against the original's Item screen for objects 0 to 32 (the picture box of a frame matched pixel by pixel
against both halves). The palette is the sixteen words at the bank-`$09` address that `object_portrait_palettes` gives the object;
colour 0 is transparent. Checked against the original's screen: for four objects, every colour in the portrait box lies in
the palette of that object, and the grid reproduces Bulma's picture.

**Corner faces.** The field menu's corner portraits are drawn from the face table `face_portrait_pointers` (`02:A3B4`, three
bytes per face index: the 24-bit address of the compressed set, word then bank). The decoder is the same as the objects'
(`01:CA98`, `00:C559`), and the set's first picture (tiles 0 to 7 and 16 to 23) or its second (tiles 8 to 15 and 24 to 31)
is drawn: the second for the indexes 09, 0A, 0C, 50 and 61 (measured by the tiles in video memory after the menu opens, with
one character alone in the party). The face index is the second byte of the character's party record (`$0B01 + 32 × slot`,
the slots of the nine party records): Goku 00, Piccolo 01, Gohan 09, Krillin 0A, Tien 0B, Yamcha 0C, Chiaotzu 0D,
Vegeta 50, Dende 61. The face index of Nail, Tsumuri and Maima has not been seen in a party yet. The corners are drawn in
the colour row 6 of the field menu, which holds the palette of object 28 (`09:D200`): an exact match of each face's pixels
against the rendered frame of the Namek party (five corners) and of Goku's corner. Piccolo's corner is drawn in the colour
row 7, object 29's palette (`09:D220`), as the frame of the starting party with Piccolo shows (one-unit rounding aside).
The palette of Dende's, Vegeta's and the other faces that were not in a measured frame is row 6, not measured. The catalogue reads the table
and the palette; `menu-test items` (`pictures`) checks the faces and names the three that are not known.

**Help text.** `item_help_pointers` (bank `$07`) gives the text of each object: lines of tiles, each line ended by `$FF`, a
page ended by an empty line. The help window shows the first page (at most three lines). The codes the catalogue cannot
spell (`$FC`, `$FD` and the like, filled in by messages at run time) show as an ellipsis.

### The item effects

The effect routine of an object is the word at `item_effect_table[id]`. `01:DBDC` first runs the helper of bit 3
(`01:F128`, the member) or bit 4 (`01:F13B`, the card), then jumps to the effect. An effect is a state machine
on `$073C` (the step) with `$073D` as a counter, `$0748` (the offset of the target's record in `party_records`, a multiple
of `$20`) and `$0740` (the value being reached); a heal counts up 11 points a frame and stops at the maximum
(`01:DD2A`). When it is done it removes the entry (`01:C639`) and the menu closes. Seen: Bulma on a member with 50 HP gives
75 HP (25 more), `item_count` goes from 10 to 9, the screen returns to the field, all in about 25 frames.

The help texts say what each does; the groups:

| Ids | Effect |
|---|---|
| 0 to 2 | restore HP: all, 50, 25 |
| 3, 4 | restore KI: all, 10 |
| 5, 6 | raise battle power 25 and 50 percent (battle) |
| 7 | paralyses an enemy for one attack (battle) |
| 8 | restores HP and KI |
| 9 to 13 | change a card or the whole hand (10 to 13 ask for a card, bit 4; 9 asks for a member by its attribute although its text speaks of a card: not understood; `$7E:2BA0` keeps a mask of the three choices already used) |
| 14 | restores HP and KI for the whole party |
| 15 | Porunga's three wishes (see "Porunga" below) |
| 16, 17 | the Scouter: information on the enemy |
| 18, 19, 26 | call Yajirobe, Vegeta, Gohan to help in a battle |
| 20 | Oozaru transformation (battle) |
| 21 | Bubbles: no random battles for a while |
| 22 | Escape: go to a place to rest, or leave a cave (`01:8CAD`) |
| 23 | the Dragon Radar (`01:8C9F`) |
| 24, 32 | the Namekian Frog |
| 25 | evens out the cards between attack and defence |
| 27 to 31 | story objects with effects in battle |
| 33 to 39 | the seven Dragon Balls: no effect |

## The Item screen (`$0D5F` = 2)

Steps of `$0D68` (seen from the watch of the variable; handlers in the table at `01:8A92`):

| Step | What |
|---|---|
| 0 | opens the windows: the list, the label, the portrait, the Zeni window, the help window |
| 1 | the list: up/down move `$0D62` (8 rows a page, `$0D63` the page, page edges in the table at `01:8B74`: 8, 16, 24 entries); a move stores the entry under the cursor (`$0D7C` = id from `item_list`, `$0C3F` = row, at `01:8AF4`), and A takes that id as it is (an entry written under the cursor without a move is not read: the HD's stage moves the cursor with Up, or Down and Up); A asks Use or Drop (set 4), B leaves |
| 2 | the Use/Drop choice (`$0D62` 0 = Use, 1 = Drop). Use: refused with "can't use" if bit 0 is set (step 8), else bit 3 goes to the member list (`01:F202` walks the party from the last member down, `$20` bytes a step) or bit 4 to the card choice (`01:F22F` walks the five cards of the hand, six bytes a step), otherwise the effect runs at once (`$073B` = `$80`). Drop: refused if bit 2 is set; else the "Drop item?" Yes/No (step 6) |
| 3 | the message after a use, then back to step 1 |
| 4 | an effect is running (`$073B` bit 7); the screen waits |
| 5 | the member list: the party with LV, HP, KI, BP columns; A starts the effect on that member |
| 6 | the Yes/No of Drop; Yes removes the entry (`01:C639`) |
| 8 | the "can't use" message, then back to step 1 |

On the map the whole screen leaves with B (back to the command menu, screen 0) and after an effect it closes the menu.

## The shop

The shop is a state machine in bank 3 (`03:A3B0`), over the town command menu. Its state: `$07AF` (the mode), `$07B0` (the
step, table at `03:A3D3`), `$07B2` (the price of the object being bought or sold), the stock at `$7E:2220` (words: id and
attribute; the stock list of the shop or, when selling, a copy of `item_list`) and its length at `$7E:2261`.

- **Which shop.** `03:97C0` looks the place number `landing_spot` (`$01BD`) up in a list of 30 places at `03:982F`; the
  index picks a pointer in the table at `03:9851` to a stock list (object ids ended by `$FF`; about a dozen different
  lists; the places repeat them). Seen: the Baba's house of the first town sells Kami, Roshi, Bulma, MrPopo, Puar and
  Shenron on the first page.
- **Flow.** BUY or SELL, then the list (six rows a page with the portrait and price box of the chosen object, the buyer's
  money under Baba's face), "What do you want?", "That'll be 10 Zeni." (the figure from `item_prices`), Yes/No, "Thank
  you!" and "Anything else?". Refusals: "You can't carry any more" (24 entries), "You don't have enough money", and "You
  have nothing to sell" when the list is empty.
- **Buy** (`03:A914`): refuses at 24; subtracts the price from `zeni` (the money cannot go below 0: a carry refuses), appends
  the stock entry (`$7E:2220`) to `item_list`. **Sell** (`03:A790`): adds the price to `zeni` (capped at 999 = `$03E7`), removes
  the entry (`01:C639`). The price is the same for both. Seen: with 30 zeni, buying MrPopo (10 Pt) leaves 20 and `item_count`
  goes from 9 to 10.

- **The Sell list in the bag's terms** (seen, the `shop` replay's Baba and a crafted stack). Talk to the shopkeeper asks
  Buy or Sell (`$07AF` 0, the question waiting in `$071B` = `$91`); the choice moves to mode `$81` (low bits 1, the lists), and the Sell list is
  that machine's step 2 (`03:A73B`: it reads the cursor `$0C3F`, an index of the list over its pages, and the object
  `$0D7C`; an entry whose attribute has bit 2 is refused with a message). Buy's lists are steps 7 to 9. The stock at
  `$7E:2220` is a copy of `item_list` made when Sell is chosen, so the cursor's row is the keeper's row, and each row is a
  stack of the bag. A sale (`03:A790`) takes `item_count` down by one and clears the attribute byte of the entry (`menu_row`'s) at the A,
  shows its messages in step 4 and takes the marked entry out of the list (`01:C639`, `03:A899`) about a hundred frames later,
  then goes back through steps 0 and 1, which redraw the Sell screen from the stock copy, and to step 2. Seen: between the
  A and the removal the list has its entries in place and the count one short, so the keeper must not read it as the last
  entry gone (it did, in 0.3.6: the sale took the last object of the bag, and gave the sold one back). The stock copy is
  compacted by the sale too, so without the keeper's write (`Keeper::sync_shop`) a stack with units left lost its row from
  the screen at the first sale (the second one sold another object). The names fill the window's two columns (nine tiles
  each), so the list has no room for a count: the HD draws it on the price box (the purple strip at pixels 64..96 by
  118..135, under the object's portrait), joined to its right.

## The Cards screen (`$0D5F` = 6)

The routine at `01:903F` has three steps (`$0D68`). Step 0 marks the five slots of the hand face down (the flag byte of each
slot, `$7E:0140 + 6 n`, becomes `$40`), puts the first slot's position word at `$05D4` (the others follow, 8 bytes on each) and
draws the five backs: the pink-and-yellow card with the dragon is a card face down. Step 1 turns one card over every 16 frames
(the flag goes to 0 and the slot is drawn by `01:C18A`, from `card_hand`), and when the fifth is over it clears `menu_busy`.
Step 2 waits for A, which jumps to the routine that follows a message (`01:894A`): the whole menu closes and the field is free.
Any other press only sounds. The original has no cursor over the hand.

A slot is six bytes from the flag: the flag (bit 7 empty, bit 6 face down), the position word, the first value (attack), the second
value (defence) and the symbol (`cards.md`). The position word is an offset into the tile-map buffer at `$7E:3000` (32 tile words
a row, so a row is `$40` bytes): the card is 4 x 6 tile words from there, row by row, each word a tile number in the low ten
bits, the palette row in bits 10 to 12 and the flips in bits 14 and 15 (seen: the hand of the `menus` and crafted-save runs reads
back as the cards the screen shows). `01:C18A` builds the card from blocks of the bank 2 pointer table: the card's body (`$27`), the
ball of the first value (`$0F` + value), the kanji (`$20` + symbol; `$43` for symbol 7) and the ball of the second value (`$17` +
value). The kanji square is in palette row 0 (blue on cream) or row 6 (black on yellow).

### The HD Cards screen

In Enhanced mode the Menu list's Cards entry presses the original's A and the HD draws what the original dealt
(`ssd_game::menu::cards`): the face-down slots as backs, the turned-over ones from the tile words the original wrote, so every
kanji, ball and numeral is the game's own (`ssd_hd::CardArt::compose` draws a card or a part of it from the art of those tiles,
the pack's card art that the battle's HD cards use). Left and Right move the HD cursor over the hand and the original stays on
its screen. The numbers in the panel are the slot's values (a Z ball, value 8, counts as 10 in battle: `05:B7A5`), the kanji's
reading and meaning are the battle tooltip's, and the party row shows each member, ringed when the card favours him or her.

Who a card favours is the battle's own test (`05:B6F2`): the fighter's battle power is raised by a quarter (the power shifted
right twice and added) when the card's symbol (record `+16`) is the fighter's class (record `+1B`) or is 7. `06:E8EE` copies the
class from `unit_stats` byte 0 of the fighter's unit (record `+01`) at the start of every battle, so the HD takes it from the
same table through the character's unit id (`party_unit_ids`; `ssd_catalogue::menu_labels`): Goku, Krillin, Tien, Yamcha and
Chiaotzu are class 1 (Kami), Gohan and Piccolo class 2 (Ma), and Vegeta, Nail, Tsumuri, Maima and Dende class 3 (Waku). The
test is skipped for a fighter whose id category (`id_category_table`, `0D:8200`) has bit 7 set; no party member has it.

B goes back to the Menu list. The original's screen takes only A, which closes the menu, so the HD plan presses A, waits for the
field, opens the menu again and chooses Menu: the Menu list comes back with its cursor on Cards. Start presses A and leaves the
menu closed.

## Facts for the HD menus

- The game's menu logic reads only `$47`/`$48` (newly pressed): feeding virtual joypad presses is a complete way to drive it.
- State to read for drawing, all in WRAM: `$25` (a menu is open), `$0D5F` and `$0D68` (screen and step), `map_menu_cursor`
  and `$0D62`/`$0D63` (cursors), `party_records` (names by `$7E:2800` ids; HP at +3, KI +5, BP +7, max HP +10, max KI +12),
  `zeni`, `item_list`, `item_count`, `item_extra`, `text speed` (`$01D0`).
- A screen takes ~3 frames to open and the queue draws one window a frame; presses made before the windows are drawn are
  not lost (a replay with 1-frame presses matches the slow one), but a macro should check `$0D5F` after each move.
  Measured in M4 task 2: the queue is empty when bit 7 of `$0D67` is set, about two frames after the menu opens; a Down
  press 17 frames after the opening did not move the cursor, so a macro waits for that bit before each press.
- Cursors (seen): the list's cursor starts at 0 when a list opens (a choice of the command window resets it); B on a
  sub-screen returns to the command window with the cursor on the row that was chosen (Item stays on 3); B on the command
  window closes the menu. The flying window has three rows and Down wraps from the last to the first; the field's five
  rows wrap the same way, and so does the Menu list.
- Messages (seen): after Talk or Look the message is drawn on screen 0 with `$0D60` = 1 and waits for A; it takes about
  200 frames to finish its text, and A then closes it. The town's shopkeeper talks in several messages, one A each.
- Save (seen, M5 check on the `menus` replay and the cheats scenarios): the Save opens on step 0 (its question); the first A
  goes to the slot list (step 1), where the cursor `$0D62` (`map_entry_value`) holds the slot, 0 to 2. The A that chooses the
  slot writes it, and the completion (step 2, "Completed!") shows one frame later, with the cursor back at 0. The third A
  returns to the field: the menu closes by itself. A fourth A on the free field opens the menu again.
- The Item list keeps `$0D60` = 1 while it is shown; its step (`$0D68`) is 1 when the list takes presses.
- The corner portraits of the party and the Zeni box are drawn by the original; the HP/KI bars under the portraits and the
  help window of the command menu do not exist in the original.

## The bag and the capacity spike (M4, task 3)

The Enhanced mode keeps the items in a bag of 60 stacks of up to 99 units (`ssd-menus::bag`, both options
in `Ruleset`). The original list is a projection of it, written by `ssd-game`'s keeper after every change.

**Capacity spike: decided by the count, no hooks.** The three checks that refuse an add compare `item_count`
with 24 (`01:D6D5` the field pickup, `05:8F4D` the battle drop, `03:A914` the shop: `CMP #$18` then `BCC` to the
add, else the refusal). The projection keeps the count at most 21 while the bag has a free slot, so the three
checks never refuse a pickup, a drop or a purchase that the bag can take, and a battle's three drops still fit.
When the bag is full (60 stacks) the projection shows 24 entries, so the same checks refuse every add by the
original's own code (its "can't carry any more" in the shop, its refusal on the field); the item stays where it
was. A hook would have needed the recorder to allow hooks (`Recorder` refuses a hooked machine), so the count is
the simplest rule that keeps Classic untouched. Known difference: while the bag is full the refusal is by count,
so a unit that would only join a stack that has room is refused too.

**What the keeper does.** It diffs the list it last wrote with the list in work RAM (a longest common
subsequence over object and state word): an added entry is a unit for the bag, a removed one is a unit gone from
its stack (a use, a drop, a sale, the story). The list is then written back as the bag projects it, with the
stack the HD staged first. The state word of an entry (`item_extra`, `$7E:2BA0`) travels with its stack. The
objects 9 to 13 (card changers) and 15 (Porunga) stack by their word: a unit joins a stack that holds the same word, and a
unit whose word changes (a wish used, a card choice) leaves its stack; the keeper takes that unit off its stack and adds it with the new word.
Dragon Balls (33 to 39) and the story objects (attribute bit 5) are key items: listed first, never dropped.

**The original's Use, from the Item list (seen with `menu-test items`).** Choose the entry (A on the list, the
Use/Drop window is step 2). A window takes its next press only after it has settled: a press in the first
frames after it is drawn is lost, so the plan waits about two seconds. Use (cursor 0) then shows either the
party list (step 5) or goes on to a message (step 3). The party list is shown for some objects only (Turtle
asks; Bulma, whose attribute has bit 3 set, does not), so the plan takes it only when it appears. The message
waits for A (busy 1 means it waits, busy 3 that it is still written); A starts the effect (step 4), whose result
may wait for another A (step 7). After the effect the menu closes by itself, or stays on the command window,
which B closes. Each unit of a multi-unit use reopens the menu from the field after a short pause.

**Drop.** Drop (cursor 1) asks Drop? with Yes and No (step 6); Yes removes the entry and the list comes back
(step 1), so a drop of several units stays on the list.

**The Dragon Radar (found in M4 task 4).** Its use is not like the others: with Use chosen on step 2 the effect
(`01:8C9F`, then `$073B` = `$80`) runs at once on the choice step, with no party list and no message, and the screen it
draws is its own (effect 23 at `01:EBE4`: step 0 sets `$133E` = `$80`, step 1 runs the radar's states of `1A:F11A` until
`$133E` is clear, then it clears `$25`, `$0D60`, `$0D68`, `$073B` and `$073C` itself and the menu is closed). In its
interactive state a new press of A (`$47` bit 7) only changes the view and a new press of B (`$48` bit 7) leaves it. The
Radar is a keepsake (attribute `%01000100`: usable on the map, cannot be dropped) and nothing in its effect removes its
entry, so it stays in the bag. The HD's plan therefore takes the Use when the effect runs on step 2, waits for the
screen to be drawn and leaves it with B. Bubbles also runs from step 2 (`$073B` goes `$80`, then `$C0` while its message
shows) and the menu closes back to the command window or the field by itself.

**The battle's Item list (M10 fix, found with `xtask disasm`, `shots --watch` and a crafted save).** A battle sets
`context_61` (`$61`) to 1 and holds the field (`field_state` is 2 while it runs). Its command window is screen 17 (Atk.,
Item, Run, Menu; the cursor is on Item after the choice). Choosing Item writes screen 2 at `01:A4E6` (the program counter
of that write) and the list's step 1 at `01:8AAB`: the Item screen of the field, with `$61` = 1. The list shows every entry, not only the battle's: an object without the battle bit (INT, bit 7) gets
bit 0 from `01:C535` and answers "can't use", so the list is not filtered. A (Use or Drop, step 2), then Use: the
effect runs on screen 21 (the battle's action screen), idle and waiting for one A; that A runs the effect, removes the
entry (`01:C639`; Yajirobe's count went from 3 to 2) and the battle is back on screen 17. B on the list goes back to
screen 17. The HD's battle use (`Driver`, the Items screen over a battle) follows the field's Use steps and reads the
effect's screen as taken; from the Use on, the original's screens are the player's (the HD covers the battle's list only on
its step 1, not on the use-on list of step 5).

**Presses in quick succession (measured on the battle's Item list).** The list ignores a press that comes within three frames of
the one before: a Down held for two frames and released, and then an Up on the next frame, leaves the cursor where the Down put it,
and so does an A on the frame after that Up. The HD's plan for a use of a stack moves the original's cursor to the top of its
list (Down, then Up, when the list has more than one entry) and presses A for the Use or Drop choice, so it waits for the state of
each press and then lets four frames pass before the next one (`PRESS_GAP`, eight in the code). Without the wait each of the
two lost presses cost the 150 frames the plan allows before it repeats one, and a use from a bag of two stacks or more took
327 frames. After the Use of a battle object that works on a fighter (BP, HP, KI), the original draws its party window (step 3,
the cursor on the first member, A takes it) over the list, with the object's help line in the box at the bottom left; it is the
original's own "on whom" question, and the HD leaves it to the player.

**Stale `menu_screen` while the menu opens.** `menu_screen` (`$0D5F`) is not cleared when a menu closes: the Radar's
exit leaves it at 2, and the next opening shows `$25` = `$0F` with that 2 for a couple of frames before it is set to 0. A
probe that reads the screen at `$0F` sees the Item screen; the curtain therefore goes up on `$0F` itself (a menu opened
from the field always opens on its command window) and reads the screen only once the menu is open.

**The party's BP.** A record of `party_records` holds the battle power in three bytes at `+7` to `+9` (24 bits); the
use-on list shows it as its BP column.

**Sidecar.** `saves/ssd.bag` (bincode, versioned) holds one bag per save slot with the slot's checksum word. A
bag binds to the save the game runs on when the slot's list, count and money equal work RAM (after a Continue
the two are the same). The sidecar's bag is taken when its checksum equals the slot's; a save changed by another
way rebuilds the bag from the list. A save writes its bag when the slot's checksum changes. The cheats of a save are
written at every Save's completion, also when the checksum is the same (README, "Cheats").

**Replays (format 2).** A replay of the Enhanced mode is format 2: the mode byte, then for each frame the joypad
and its host events (a work RAM write, or an application event: the bind, which carries the bag's bytes). The
replay is re-played with the same keeper; the final state (work RAM and the bag) is compared by hash.

## Open

- What `$25` = `$0F` counts (about 18 frames, then 1) and `$0D67`/`$0D60` precisely; screens 5, 11 to 16 and 18 to 29.
- The face index of Nail, Tsumuri and Maima (see "Corner faces"); the use of the second picture of each object (tiles 8 to 15
  and 24 to 31).
- The kanji mark of the price box of the objects with no price (tile `$2F`, drawn by `01:CACB`) and the run-time slots of the
  help texts (`$FC`, `$43`, filled in by messages): the menu shows nothing and an ellipsis there.
- The effects of ids 27 to 31.
- What the second kanji-like icon in the price box means for each object (a "no price" mark, seen on the Dragon Balls,
  Gohan and Korin).

## How to see it again

    cargo run --release -p xtask -- shots menus --every 150 --upto 6900 --sheet 6       # the field menu and its screens
    cargo run --release -p xtask -- shots .work/replays/shop.ssdreplay --every 1000 --sheet 10   # the shop (about 8 minutes)
    xtask save-edit in.sav out.sav --set items=bulma,scouter,yajirob --set page:B2=18   # a crafted item list (page:00= for 24)
    xtask replay-from-save out.sav timeline.txt out.ssdreplay; xtask shots out.ssdreplay --frames ... --peek 0x7E0D5F,0x7E0D68

`--peek` takes the variables of `re/symbols.toml` or hex bus addresses, `--watch` prints the writes of a byte (with the
program counter after `--watch-from N`), which is how the routine of each write above was found.

### Porunga

Read with `xtask disasm` over the effect (`01:E2A5`, a step table) and `xtask shots` on a battle state, the Use of
Porunga in a battle as the pure original plays it. The state word of the object (`item_extra`) is the wish points in its
high byte and a bit per wish used in its low byte (`$80`, `$40`, `$20` for the first, second and third). The story gives
the object with 150 points (`04:D37E` writes `$96` to the high byte, then the entry), and each wish takes 50 (`01:E302`:
the high byte minus `$32`, the wish's bit set). After a wish the effect finds the entry again by its row (`01:8BC1`: the
page `$0D63` times 8 plus `menu_row` `$0C3F`) and asks for the next wish while points are left (`01:E346`); at 0 points it
removes the entry (`01:E350`) and the use ends. An entry with a word of 0 (the item spawner's, before it set the word)
wraps at the first wish (0 - 50 = 206) and never reaches 0: the window stays with the three wishes used. The bag gives a
Porunga with a word of 0 the story's word (`ssd_menus::bag`).

The wish window (window set `$23`: entries `$33`, drawn by `01:B205`, and `$16`) places the cursor through `01:F2E4`, a
word table at `01:F2F6` of the tile-map offset of each wish (`01:F248`, called by Porunga's effect only), and the marks of
the wishes used at the same three offsets (`01:B238`, `01:B244`, `01:B24F`). The offsets are the tile rows 4, 8 and 10,
the layout of a first wish of two lines; the English text has the wishes on the rows 4, 6 and 8, so the cursor on the
second wish is drawn on the third and the one on the third on the empty row under it. The Remastered fix (`cheats.rs`,
`story_porunga_wish_*`) moves the cursor and the marks to the rows of the text.

The HD's use stages Porunga at the top of the list; the keeper (`menu::keeper`) holds that entry there until the use is
over, as the effect finds it by its row while its word changes.
