# Cards

The game has no list of cards. A hand of five is dealt at the start of the game by the routine at
01:C236 (and again as cards are played), and each card is a random combination:

- first value byte: `card_value_weights[random_a & 31]`
- second value byte: `card_value_weights[random_b & 31]`
- symbol: a random number from 0 to 6 (a draw of 7 is repeated, so a dealt card never has symbol 7).

The three numbers come from one routine, 01:C257 (returns the values in `$10` and `$11` and the symbol in
`$12`). Three callers deal cards with it: 01:C236 (the first hand), 01:C2AE (refills the hand after a card is
played, from 01:C31E and 03:812D) and 06:E837 (the battle set-up, which refills every empty slot of the hand at the
start of each battle). `card_value_weights` (01:C28E) is a table of 32 bytes; the values
it holds are 1 to 8, in non-decreasing order, so the chance of a value is its count: values 1 and 8 three times in
32, 4 and 5 five times, the others four times each.

`card_hand` (`$7E:0141`) holds the hand as five cards of six bytes. Counted from `$7E:0140` a card is: a flag byte
(bit 7 set = the slot is empty, the card was played), a position word (where the card is drawn; it moves when the
hand shifts), the first value, the second value and the symbol. The saved game keeps the hand.

## What the numbers mean (found for the cheats, `battle.md`)

Both values are battle numbers, drawn in two corners of the card; the symbol is the kanji in the middle.

| Part | On the card | In battle |
|---|---|---|
| First value (1 to 8) | the ball in the **top-left** corner, drawn as that many dots | the **attack** number: the multiplier of the attacker's power when the card's owner attacks |
| Second value (1 to 8) | the ball in the **bottom-right** corner, drawn as a kanji numeral | the **defence** number: used when the card's owner is attacked, and for the counter-attack (`$14C2` bit 1) |
| Symbol 0 to 6 | the kanji | when it equals the fighter's class (`unit_stats` byte 0 for enemies; 1 for Goku, Krillin, Tien, Yamcha and Chiaotzu, 2 for Piccolo and Gohan) or is **7**, the fighter's battle power counts 25 percent higher, in attack and in defence (05:B6F2; the class is copied from `unit_stats` byte 0 into record `+1B` at the start of each battle by 06:E8EE) |

A value of **8 is the Z ball**: the corner shows a Z instead of dots or a numeral, and the battle code counts an 8 as
**10** (05:B7A5, used for the attacker's and the defender's number). The weights give a Z ball a 3 in 32 chance per
corner. In the battle hand of the reference run, the cards with first value 8 show a Z in the top-left ball and the card
with second value 8 shows a Z in the bottom-right ball (seen).

Symbol 7 is never dealt by 01:C257; the battle test accepts it as a wildcard (a card whose symbol is forced to 7 gives
the 25 percent as if it matched, seen in `battle.md`). The symbols are tiles, not text: nothing in the ROM names them,
so the catalogue keeps their numbers. In the reference hand symbol 1 is the kanji for god, 6 for super and 0 for world.

## Where a card goes

When the player picks a card for a fighter (the Pick command, 01:9CFC, the A press at 01:9D26 to 01:9D31) the card's
two values and its symbol are copied into the fighter's party record (`+14`, `+15`, `+16`), the hand slot is marked
empty (flag `$80`) and the slot's index is stored at `+1A`. At the start of the fighter's turn 05:AE81 (party) or
05:AE16 (enemy) copies the whole record into a *side record* (`side_record_a`, `side_record_b`; `+14`/`+15`/`+16` are
the card in play) and the damage routine reads the card from there. During the attack animation 1D:83A9 reuses the
first two hand slots to draw the attacker's card and the defender's card (the enemy fights with cards too: values
`+14`, `+15` of its record, which the enemy turn code fills; the source is the table at 0D:83D9, read by 05:BC5B,
not decoded).

## How a card is drawn (seen, `battle` replay, frames 4750 and 5250)

A card is **4 x 6 tiles** (32 x 48 pixels) of the 2-bit UI set, written to a BG tile map (BG3 in the command screen,
where the hand sits in the bottom row at tile column 10 + 4 x slot; the map menu's Cards screen and the attack
animation use the same tiles on other layers), and the same
pieces make every card. The pieces, as (column, row) in the card:

| Where | Tiles | Palette |
|---|---|---|
| frame: top edge (2, 0), top right corner (3, 0), right edge (3, 1) to (3, 3), left edge (0, 2) to (0, 4), bottom left corner (0, 5), bottom edge (1, 5) | 6 tiles of the set (`$94`, `$95`, `$A5`, `$92`, `$A2`, `$A4`), plus the plain black tile `$03` inside | row 3: white, orange, black |
| first value, the ball in the corner (0, 0) to (1, 1): a 2 x 2 block | one block per value: 1, 4, 5, 6, 7 and Z have their own; 2 and 3 reuse tiles of the 4-dot ball and `$FE`/`$FF` | row 3 |
| symbol, the square in the middle (1, 2) to (2, 3): a 2 x 2 block | one block per symbol (below) | row 0 (blue ink on cream, orange ring), row 6 (black on yellow, pink ring) for some cards |
| second value, the ball in the corner (2, 4) to (3, 5): a 2 x 2 block | `$B0`... in steps of 2: one block per value 1 to 8, the kanji numeral or Z | row 3 |

The balls overlap the card's corners (their tiles hold the frame around the sphere), the kanji square fills its 16 x 16
pixels (a one-pixel orange ring and the glyph, which is white on blue for 必). The played card shown beside a portrait
is the same kanji block between two frame pieces (`$117`, `$95`; `$A2`, `$118`). The pick cursor is the tile `$111`, an
arrow in row 3, drawn in the tile row above the hand, over the first tile column of the chosen card's kanji, and it
blinks. The HD cards (`ssd_hd::cards`) find these tiles by their content, so every combination is covered by about
a hundred tiles of art.

## The symbols (kanji) and their tooltip

The symbol number a card carries (the third value byte) picks one of seven kanji blocks (the dealing routine's table
lists the tiles in this order). The class of a fighter, `unit_stats` byte 0 (`units.md`), is the same number: a card whose
symbol is the fighter's class (or 7) adds 25 percent to its battle power (`battle.md`). Readings and meanings are our own
notes on the kanji; the classes were read off the unit table with `xtask catalogue --kind enemies`.

| Symbol | Kanji | Reading | Meaning (tooltip text) | Tiles at the top left of the kanji | Fighters of this class (seen) |
|---|---|---|---|---|---|
| 0 | 界 | Kai | world, realm | `$F4` | King Kai and a few party members in special scenes |
| 1 | 神 | Kami | god | `$F8` | Goku, Krillin, Tien, Yamcha, Chiaotzu, Yajirobe |
| 2 | 魔 | Ma | demon | `$F2` | Piccolo, Gohan |
| 3 | 惑 | Waku | doubt, bewilderment | `$F6` | almost every enemy: Raditz, the Saibamen, Nappa, Vegeta, Frieza's army |
| 4 | an emblem, not a kanji | none | the emblem of the Ginyu Force (class 4 is Ginyu, Burter, Guldo, Jeice and Recoome) | `$FA` | the Ginyu Force |
| 5 | 必 | Hitsu | sure, certain (as in a sure-kill) | `$F0` | none seen |
| 6 | 超 | Chou | super | `$FC` | a few special units (Goku, Gohan, Piccolo, Vegeta in other forms) |

The Enhanced picture shows the tooltip (two lines: the reading, then the meaning, with the kanji or emblem on the
left; the texts are in `crates/ssd-hd/src/tooltip.rs`) while the cursor is over a card and for about a second after it
blinks off.

The classes of the party's fighters, read through `party_unit_ids` from `unit_stats` byte 0 (what 06:E8EE copies into record `+1B`): Goku,
Krillin, Tien, Yamcha and Chiaotzu 1; Gohan and Piccolo 2; Vegeta, Nail, Tsumuri, Maima and Dende 3 (`xtask catalogue --kind enemies`,
units 0, 1, 9 to 13, 80, 34 to 36 and 97). The favoured fighters of a card are therefore decided by its symbol alone: 1 favours
the Kami fighters, 2 Gohan and Piccolo, 3 Vegeta and the later fighters, 0, 4, 5 and 6 nobody in the party (7 everyone). The
HD Cards screen shows this under each card (`menus.md`, "The HD Cards screen").

### The original's own message for a favourite card

While the pick cursor is on a card whose symbol is the fighter's class (the card that adds 25 percent), the original opens
its message window beside the card: "Attack everyone!" (seen in the `battle` replay at frames 4772 to 4780, Goku's turn with
the 神 card under the cursor; the text is string 6 of string table 7, `system_string_pointers` at `07:938E` (message `$30`,
see `text-engine.md`), next to "You ran!" and "You were unable to escape!"; the table also holds a longer wording for the same
event, which says the card is a favourite and attacks every enemy). The window is the command screen's message window: it starts at the right
edge of the card under the cursor (x 112 for the first card), its top edge is at y 167 (7 pixels above the cursor's tile row)
and it runs to the bottom of the screen, covering the cards after the one under the cursor. The Enhanced tooltip never
covers it: the tooltip ends 9 pixels above the cursor's tile row, so it is above the window's top edge, and its right edge is
the hand's (five cards from the hand's left edge, the cards the message hides included), so it does not slide left over the
command window either.

## Forcing Z cards (cheat research)

Setting `$10` and/or `$11` to 8 at the return of 01:C257 makes every dealt card a Z in the attack corner, the defence
corner or both, for every fighter; the hand then shows the Z balls and the damage rises by (10 + 4) / (value + 4) on
the attack side (seen: a hand forced to 8 and 8 turned the first hit of the reference battle from 19 to 29).
For one fighter only, the card cannot be changed when it is dealt (the hand is shared); set the side record's `+14`
(attack) or `+15` (defence) to 8 at the entry of the damage routine instead.
