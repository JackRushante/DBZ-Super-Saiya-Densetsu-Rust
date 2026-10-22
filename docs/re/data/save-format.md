# The battery save

The cartridge has 8 KiB of battery SRAM (`$70:0000`-`$70:1FFF`, `sram_slot_*` in `re/symbols.toml`).
The game keeps three save slots in it (the menus call them 01, 02 and 03). Read by `crates/ssd-save`;
edited by `xtask save-edit`; a replay that starts from a save is made by `xtask replay-from-save`
(README, "Saves"). Everything here was found by reading the save and load routines of bank `$03` and by
comparing the SRAM of the reference replays (`menus`, `shop`) with the memory of the running game; the
statements about what a field means were each checked by editing the field in a save and looking at the
game after Continue.

## The container

| Offset | Bytes | What |
|---|---|---|
| `$0000` | `$100` | Not used by the save routines (all `$FF` in every save seen, as in a new cartridge). |
| `$0100` | `$4F0` | Slot 1. |
| `$0600` | `$4F0` | Slot 2. |
| `$0B00` | `$4F0` | Slot 3. |
| `$1100`, `$1600`, `$1B00` | `$4F0` | The backup copies of the three slots (each `$1000` after its slot). |

A slot is a copy of two areas of the game's memory, with a checksum over them:

| In the slot | Bytes | Copied from |
|---|---|---|
| `$000`-`$3EF` | `$3F0` | WRAM `$7E:2800`-`$7E:2BEF`, the *image* |
| `$3F0`-`$4ED` | `$FE` | WRAM `$00:0100`-`$00:01FD`, the *page* (the 256 bytes of `$0100` with the last two replaced by the checksum) |
| `$4EE`-`$4EF` | 2 | the checksum |

- **Checksum**: the sum, modulo 65 536, of the 16-bit little-endian words of the first `$4EE` bytes (routine
  `03:E6F0` computes it; `03:E6E1` stores it). `$01FE` and `$01FF` of the page are therefore not saved: they
  read back as the checksum.
- **Mark**: eight bytes at `$3E0` of the image (WRAM `$7E:2BE0`), written at every save and compared with a
  constant of the ROM (`03:E655`): the pairs `FE DC BA 98 76 54 32 10`. A slot without the mark is empty;
  the menus show it blank.
- **Backup**: the save routine (`03:E556`, called from the map menu's Save, `01:96F4`) writes the slot, then
  copies all `$4F0` bytes to the backup.
- **Check at start-up** (`03:E70E`, called from `00:8098`): for every slot, if the mark is there and the
  checksum of the first copy is wrong, the backup is tried; a good backup is copied over the first copy, a
  bad one wipes the slot (`$4E0` bytes of zeros) and the slot's bit is set in `$7E:4E20` (one bit per slot).
- **Load** (`03:E5FC`, from the Continue menu): copies the image to `$7E:2800` and the page to `$0100`, then
  rebuilds the party records (see below), restores the follower's position into the direct page and the
  leader's from the saved copy, and fixes a few state bytes for the place.

Writing a slot the way the game does (mark, checksum, both copies) is `ssd_save::SaveFile::set_slot`; reading
one with a wrong main copy and a good backup gives the backup, as the game does.

## The image (`$7E:2800`)

| Offset | Meaning | Certainty |
|---|---|---|
| `$000`-`$17F` | Twelve character records of 32 bytes, **indexed by character id** (`$000` Goku, `$020` Gohan, `$040` Piccolo, ...). The layout is the party record of `characters.md`. Only characters that were in the party at some save are real; the others hold whatever the memory held (the game fills its memory with a random pattern at power-on, so these bytes are noise and the same in every save of one power-on) and are ignored. | decoded |
| `$180`-`$3D7` | Unused in every save seen (zeros); the party order list the game builds at `$7E:2AA0` is not saved. | |
| `$3D8`-`$3DA` | Three bytes copied from the scratch buffer `$7E:4C00` at save time (no meaning; they differ from save to save). | junk |
| `$3E0`-`$3E7` | The mark. | decoded |

How the records get to the party: at save, every party record (`$7E:0B00`, nine of 32 bytes) whose flags
byte has bit 7 or 6 set is copied to the image slot of its character id (`03:E419`, `03:E42A`); the list of
the ids in the party, in order, is written to `$0190` (`03:E4D1`). At load, the ids of that list are read
back and each one's record is copied from the image to the next party slot (`03:E4F4`, `03:E461`). So **the
party is the list at `$0190` and nothing else**: a record in the image that is not on the list is not in the
game. The party size at `$01BB` must equal the length of the list: the game drops the members past it a few
frames after the load (found when a crafted party of eight lost six members until `$01BB` was set).

Record bytes (`characters.md`): `+00` flags (`$C0` in the party), `+01` unit id, `+02` level, `+03` HP,
`+05` KI, `+07` BP (24 bits), `+0A` max HP, `+0C` max KI, `+0E` copied from the level row, `+0F` EXP (24
bits), `+1C` character id. Also seen: `+17` is `$1E` in every record, `+1B` is 1 for Goku, 2 for Piccolo
and 3 for Vegeta (the order they join in, as far as the code that builds Vegeta's record shows), and `+14`
to `+16` grow from zero as the character levels up (not decoded). A record for a given level is built from
the level rows exactly as the game's builder does: the battle power and KI of every row up to the level are
added, the max HP, EXP and byte `+0E` are those of the row of the level (checked against a real save:
BP, max HP, max KI and `+0E` of both members match the rows for their level; EXP is at least the row's).

## The page (`$0100`-`$01FD`)

| Page offset | Address | Meaning | Certainty |
|---|---|---|---|
| `$00`-`$2F` | `$0100`-`$012F` | The item list (`item_list`): 24 words (`$0100`-`$012F`), each an object id (low byte; the object name table, `names.md`) and an attribute byte. The Item menu shows the first `item_count` (at most 24) of them; `docs/re/data/menus.md` has the rest (`xtask save-edit` and `save-dump` stop at ten). The attribute is a cache that routine `01:C535` rewrites from a ROM table by id (`item_attributes`): bits 7, 6 and 5 are the INT (usable in a battle), EXT (usable on the map) and AID (story helper, never usable) marks of the menu, bit 0 is set at run time for "not usable now". The tool takes the byte from that table. | decoded |
| `$40`-`$5E` | `$0140`-`$015E` | The card hand: five cards of six bytes from `$0141` (`cards.md`): a position word (0 for a card that is not in the hand), two values, the symbol and a flag byte (`$80` in a used card). The game has no deck (`cards.md`): cards are dealt at random, so the hand is all that is saved. | decoded |
| `$70`-`$7F` | `$0170`-`$017F` | The **story flags** (`story_flags`): 128 flags, flag `n` at byte `n / 8`, bit `n % 8`. The chapter set-up routines clear them all (`03:EF1A`) and set the flags of the characters in the party (flag = character id, 0 to 11, by `03:EC90`, `03:EB54`...); the scene scripts set, clear and test them (`scenes.md` lists the ones understood: recruits `0x10`-`0x13`, the Ginyu chapter `0x1F`, `0x22`, `0x1A`, the dragon balls `0x23`-`0x29`...). The map routines read them to decide whether a scene starts when the save is loaded, so they are what a crafted save sets (`save-edit --set flags=+3,-0x1F`). | decoded (the meaning of each flag: partly) |
| `$80`-`$83` | `$0180`-`$0183` | The **event bytes** (`event_bits`): `$0180` a small counter and `$0183` bits read by the set-ups of the first chapters and by the map-16 and map-22 logic (on map `$16` bit 0 of `$0183` with five battles forces the battle of group `$53`, then clears it); set by the event handlers of bank `$03` (`$0183` takes `$02`, `$04`, `$20`, `$40` and a mask from `$164F`; `$0181` bit 0). All zero in every save of the first chapter. | partly |
| `$90`-`$9F` | `$0190`-`$019F` | The party: character ids in order, ended by `$FF` (`party_order`). | decoded |
| `$A0` | `$01A0` | The **map number** (`map_id`): which map the party stands on. 0 is the first map (the island of the first chapter). See "Maps" below. | decoded |
| `$A1` | `$01A1` | A gate bit used by the draw code (bit 7 skips a copy). | not decoded |
| `$A2`-`$A5` | `$01A2`, `$01A4` | The leader's position on the map, x then y, words in map pixels (`follow_target_x`, `follow_target`). Not read at load: see `$AA`. | decoded |
| `$A6`-`$A9` | `$01A6`, `$01A8` | The follower's position (`follower_saved`), restored into `$33` and `$35`. | decoded |
| `$AA`-`$AD` | `$01AA`, `$01AC` | The leader's position as saved (`leader_saved`): the save routine copies `$01A2/$01A4` here and the load restores them from here. | decoded |
| `$B0`, `$B1` | `$01B0`, `$01B1` | Counters of the random-battle logic of the map: `$01B0` counts the battles, `$01B1` a sub-count that wraps at 4. | partly |
| `$B2` | `$01B2` | The number of entries of the item list in use (`item_count`, 0 to 24). | decoded |
| `$B3` | `$01B3` | A value routine `01:B782` derives from `$01B2` (`map_param_01b3`, a provisional name). | derived |
| `$B4`, `$B5` | `$01B4` | The zeni (`zeni`), a word, at most 999 (Game Genie `7E01B4FF` is "unlimited money"). The Item menu shows it. | decoded |
| `$BB` | `$01BB` | The party size (`map_param_01bb`, a provisional name from the first notes). | decoded |
| `$BC`, `$BD` | `$01BC`, `$01BD` | State flags of the map mode (`$01BC` has bits that change when the party takes off, lands or fights; `$01BD` a small counter). | not decoded |
| `$C0`-`$CF` | `$01C0`... | Camera and scroll values derived from the positions; the game recomputes them. | derived |
| `$D0`-`$E7` | `$01D0`... | The state of the scene and of the screen effects. `$01D8` is the **scene id** (`scene_id`), the script of the story engine being played or last played (`scenes.md`); `$01D7` bit 7 marks a scene that Continue resumes (the code checks scene `$11`, the Saiyan chapter). `$01D9`, `$01DA` and `$01DC` are the running command, its step and the script offset, which a loaded game does not use (a scene never resumes: the map's routine starts it again if the flags and the position say so). Changing the others is not needed to load a save. | partly |

Everything else on the page was zero in all the saves seen.

## Maps

`$01A0` selects the map and the leader's position is in the pixels of that map. Setting it in a save and
choosing Continue loads that map, and the map's own entry scene runs: some start with a scripted message or
fight, which is how a save can be moved to a later part of the story without playing to it. What the maps
look like at the position of the first save (the map's own content, seen in the screenshots of the headless
`shots` tool; not tracked):

| Map | What loads |
|---|---|
| 0 | The first island, on foot: mountains and two small domed houses. |
| 1, 7 | A grass field crossed by a white-tiled strip. |
| 2, 3, 10, 12-15, 17, 19, 26, 27 | Caves: dark, brown tunnels, purple, red and blue rock. |
| 4, 5, 6 | Mountains by night, the same mountains by day, a green plain by night. |
| 8, 9, 18, 24, 25 | Cliffs by the water, grass by the sea, a river in a desert. |
| 11 | The Namek archipelago; a message from Piccolo ("Hurry to the Dragon Ball") shows at once. |
| 16 | A Namek field; a message ("Where are you going?") and then the battle of encounter group 78, Jeice and Burter of the Ginyu squad, which starts without any input. |
| 20, 21 | The spaceship's rooms. |
| 22, 23 | Namek's islands. |
| 28-31 | Lava caves. |
| 32-47 | The same brown cave. |
| 48 and up | No map: the game never returns from the load (the emulation hangs), so the tool refuses them. |

The first survey (above) set only the map; with the party and flags of a story point the same maps start
their scenes: map 0 the forced fight with Raditz (flag 3 and Goku at level 5), map 9 Dende's rescue, Cui and
Dodoria, map 16 the Ginyu squad, Nail against Frieza and Porunga, map 24 the last fight, map 34 the second
Nail (`secrets-coverage.md`).

A map id is a place, not a chapter: one chapter walks over several maps and a place is used by two chapters
(`scenes.md`, "Map ids", has what each is, from the set-up routines, the random-battle tables and screens).
The map's own routine and the story flags decide what happens in it.

## What identifies a story point

There is **no single story-progress flag** in the save. The point the player is at is the combination of:

- the map and position (`$01A0`, `$01A2`-`$01AD`);
- the party list, the records of the roster (levels, stats) and the party size;
- the story flags (`$0170`-`$017F`), the event bytes (`$0180`-`$0183`) and the item list (the dragon balls);
- a few bytes of the image beyond the roster that scenes leave for later ones: the party list at `$7E:2AA0`
  (image offset `$2A0`, ids ended by `$FF`) and the enemy's hit points of the last fight (`$7E:2B03` and
  `$7E:2B0A`, image `$303` and `$30A`, words) - the last chapter's scene copies them.

Loading a save rebuilds the party from the roster (the chapter set-up of the map does not run: `title-chapters.md` 5) and
the map's routine starts a scene by itself when the flags and the position fit, so a story point is
placed by choosing them: `docs/re/secrets-coverage.md` lists, for each event reached, the fields that put a
save just before it. What is not known: the meaning of every flag, `$01BC`, `$01BD` and the rest of
`$01D0`-`$01E7`. Raw access (`--set page:OFFSET=HEX`, `--set image:OFFSET=HEX`) exists for experiments.

A save the game wrote itself at a late story point (`xtask shots ... --sram-out`, after the map menu's Save)
is the safest start for a new experiment: the flags and the state around them are then consistent.

## Getting from power-on to a loaded save

The replay of a save starts like a player would: the intro runs until the title screen shows "PUSH START"
(frame 1 600 or so), Start opens the menu (GAME START / CONTINUE), Right moves to CONTINUE, Start opens
"Which slot?" with the three slots' parties, Down moves to the slot, A picks it (the status of the party is
shown with "Load game?"), A on Yes loads, and the map is on screen about 200 frames later. `xtask
replay-from-save` prepends exactly that (2 304 frames for slot 1, 18 more per slot below it) to the script.

## Two crafted saves

Both start from a save the game wrote itself (a first-chapter save: Goku and Piccolo at level 2, ten items,
20 zeni) and were checked by `verify-replays --hooks original`, by the screenshot after Continue and, for the
first, by an ignored test that compares the game's memory with the edited slot
(`SSD_ROM_PATH=... SSD_SAVE_PATH=... cargo test -p xtask save -- --ignored`).

1. **A member's level raised.** `save-edit in.sav a.sav --set goku.level=10` then
   `replay-from-save a.sav tools/saves/status.txt a.ssdreplay`: the Status screen shows Goku at level 10
   with 190 HP, 48 KI and 5 142 BP (level 2, 88 HP, 16 KI, 500 BP before); Piccolo is unchanged.
2. **A later part of the story.** `save-edit in.sav b.sav --set party=gohan,krillin,vegeta --set
   gohan.level=30 --set krillin.level=27 --set vegeta.level=28 --set map=16 --set
   items=d.radar,scouter,bulma,bulma --set zeni=300` then `replay-from-save b.sav tools/saves/ginyu.txt
   b.ssdreplay`: Continue loads the Namek field of map 16, the entry scene plays ("Where are you going?",
   "Bastards!") and the battle against Jeice and Burter opens by itself with Gohan, Krillin and Vegeta. The
   levels are plausible values, not the ones a player would have; the event bits and the map's other state
   stay as the first-chapter save left them. Which events a later map expects to have happened is not known
   (see "What identifies a story point").
