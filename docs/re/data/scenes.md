# Scenes: the story engine

How the story of the game is run, found for the crafted saves of `secrets-coverage.md`: a script
language of 51 commands, 82 scripts ("scenes"), a bank of 128 story flags and, per map, a small routine
that starts a scene when the party stands in the right place with the right flags. Everything here was
read from the code of banks `$00`, `$01`, `$02`, `$03`, `$04` and `$05` and checked by crafting a save that
makes the thing happen (`secrets-coverage.md`). Certainty: "seen" = a replay shows it, "code" = read in the
code only.

## Where the pieces are

| What | Where |
|---|---|
| Scene id | `scene_id` (`$7E:01D8`), saved. Written by the code that starts a scene (03:F7DE: sets the id and `state_1648` to `$80`) and by the `scene` command. |
| Scene state | `state_1648` (`$1648`): bit 7 = a scene is starting (its script pointer is loaded at the next frame), bit 6 = fetch the next command. `scene_op` (`$01D9`) is the command being run, `scene_op_state` (`$01DA`) the step of a command that takes several frames, `scene_offset` (`$01DC`) the position in the script. Not saved (except the scene id): a scene never resumes after Continue, with one exception (scene `$11`, see below). |
| Script pointers | 82 words at 06:D109, one per scene; the scripts are in bank 6 (`$A5` holds the pointer, `$A7` the bank). A few scenes share a script (5 and 6, 8 and 9, 11 and 12, 15 and 16...). |
| Command table | 51 words at 03:AC4C, the handlers follow from 03:ACB2. A jump or call address in a script is absolute (a bank 6 address). |
| Per-map trigger code | 03:F1AE, one word per map id up to `$31` (routine 03:F161 picks it), and 03:FB07 for maps 5, 11 and 16 (called from 03:F7A4, the per-frame story check). |
| Game mode | `game_mode` (`$24`); dispatch in 00:8B54 (see "Modes"). |

## The commands

A command is one opcode byte and its operands. `W` is a word (little endian), `B` a byte. A command
that reads a flag number reads the 8-bit id `n`: byte `n / 8` of `story_flags`, the bit `n % 8`.

| Op | Name | Operands | What it does |
|---|---|---|---|
| 00, 32 | start | - | loads the script pointer of `scene_id` and starts it |
| 01 | end | - | the scene is over (`state_1648` = 0) |
| 02 | picture | B | shows map entry picture B (the portrait set of the next message) |
| 03 | object | B, W, W | puts an object (slot B) on the map at the two coordinates |
| 04, 0F | move | B, B | moves an object (a map object, a sprite) along move list B |
| 05 | text | W | shows message W of the map's message table and waits until it is closed |
| 06, 07 | sound, music | B | plays sound or music B |
| 08, 09, 0A | nothing | - | |
| 0B | if flag | B, W | jumps to W when flag B is set |
| 0C, 0D | set flag, clear flag | B | |
| 0E | sprite | B, B, W, W | a sprite object on the map |
| 10 | wait | B | B frames |
| 11 | battle | B, B, W | battle of encounter group B (the second byte is a timer); jumps to W when the party won, goes on when it lost |
| 12 | jump | W | |
| 13 | poke | W, B | stores byte B at bank 0 address W (the scripts use it for `$0EA3` the group, `$0024` the mode, `$164C` and `$164D` the parameters of the level routine, the map object table at `$1180`...) |
| 14, 16, 18, 2D | screen | - | redraw, reset or hide parts of the screen around a message |
| 15 | copy object | B, B | copies one 7-byte slot of the map object table to a sprite slot |
| 17, 27 | camera | B | scrolls to a place (waits) |
| 19 | place party | B, B, B, B | the leader's position (x, y words); the follower is put 128 and 120 pixels behind |
| 1A | zero | W | stores 0 at bank 0 address W |
| 1B | effect | B | runs effect routine B (see below) |
| 1C | add member | B, B | a new party member: character id B at level B |
| 1D | portrait | B, B, B | stores two bytes in the party face row (`$1640` + 2 x the first) |
| 1E | effect set-up | B, B, B | three bytes for the screen effect of mode `$8C`, then that mode runs |
| 1F, 20 | call, return | W | subroutine with one return slot |
| 25, 26 | call, return | W | a second return slot |
| 21 | mode | B | sets `game_mode` to B and plays music `$84` |
| 22, 23 | save, restore | - | the party's positions |
| 24, 28 | fade | B, B / B | screen brightness (fade out `$8F`..., in `$0F`...) |
| 29 | story transition | B | mode 7 with transition target B (`transition_target`), see "Story transitions" |
| 2A | remove object | B | the map object with id B |
| 2B | join | B | character id B joins: its roster flags become `$C0` and it is added to the party |
| 2C | go to map | B, B, W, W | `$164B`, the new map id and the leader's position |
| 2E | scene | B | goes on in scene B |
| 2F | increment | W | adds 1 to the byte at bank 0 address W |
| 30 | battle | W | like 11 with the group already set |
| 31 | party | - | rebuilds the party order list |

`python tools/re/scenes.py` lists the scripts of the cached ROM. Only the length of each command was needed to read the 82 scripts completely; the screen and sprite
commands are named by what they were seen to do, not decoded.

### Effect routines (command `1B`)

110 routines at 03:B786 to 03:D2D0 (table at 03:B6A8, indexed by the operand), each a small state
machine that runs over many frames (variable `$1669` is its phase). Seen: `$2F`, `$30`, `$31` are
Nail's fight against Frieza at Guru's house (`$30` sets the party to Nail alone, level 36, and starts
the battle of group `$57`), `$3C` starts the last battle (group `$56`, Frieza's final form: Goku joins
the party, the enemy's hit points come from `$7E:2B03` and `$7E:2B0A`, the party from the id list at
`$7E:2AA0`), `$38` raises a character's level (`$164C` = the character, `$164D` = by how many).

## Story flags

`story_flags`, 128 flags in the save page (`$0170`-`$017F`). The chapter set-up routines clear them all
(03:EF1A) and set the ones of the party. Flags are used for different things in different chapters.

| Flags | Meaning |
|---|---|
| 0 to 11 | the character is in the party: 0 Goku, 1 Gohan, 2 Piccolo, 3 Krillin, 4 Tien, 5 Yamcha, 6 Chiaotzu, 7 Vegeta, 8 Nail, 9 Tsumuri, 10 Maima, 11 Dende. Set when a chapter starts (03:EC90 and the like) and by the `join` command; the scripts test them to choose the lines. (Flags 10 and 11 also serve as progress flags in the first chapter, 3 as "in a fight" in the first scene.) |
| 0x10 to 0x13 | recruited: Krillin, Yamcha, Tien, Chiaotzu (scenes 5 to 9) - seen |
| 0x14 to 0x17 | set at the start of the Ginyu squad's chapter (scene `$2F`) - code |
| 0x18 | set by Dodoria's scene (`$1A`), tested by scenes `$11`, `$33` and `$49` (their lines differ): the meaning is not settled |
| 0x19 | Dende rescued (set by scene `$16`) |
| 0x1A, 0x1B | progress of the Frieza chapters: `$1A` set by scenes `$33`, `$35` and `$3A` and tested by map 16's routine (it starts scene `$34`), `$1B` set by `$14`, `$15`, `$35`, `$3A`, `$51` and tested by map 11's routine |
| 0x1E | the opening scenes' "after the cave" flag (set by `$13`, `$14`, `$15`, `$17`, `$18`, `$28`, `$32`, `$49`); on map 9 it, with flag 0, starts Cui's fight - seen |
| 0x1F | the Ginyu squad's fight has started (set by the scene of Jeice and Burter); its test selects the next scene of map 16 - seen |
| 0x22 | the Ginyu fight is done (set by scene `$32`) - seen |
| 0x23 to 0x29 | set by scenes `$21` to `$27`, one each; read as the seven Dragon Balls picked up (their objects have the ids of the balls) - code |

Flag 8 (`$0171` bit 0) is used on the last map as "the last fight may start", see below. The words
`$0180`-`$0183` (`event_bits`) are older: a counter, and `$0183` bits that the first chapters' set-ups and
the map-16 and map-22 logic read.

## What starts a scene

1. **A map's own routine** (03:F1AE), looked at every frame the party is free: it compares the leader's
   position (`$01A2`, `$01A4`, pixels) with rectangles and tests flags. Seen to start a scene:

| Map | Condition | Scene |
|---|---|---|
| 0 | flag `0x0B` clear, flag 3 set, Goku at level 5 or more (or Piccolo at 4 or more) | `$02`: the forced fight with Raditz - seen |
| 9 | flag 7 (Vegeta) clear, leader inside x 808-959, y 256-399 | `$16`: Dende's rescue - seen |
| 9 | flag 0 set, flag `0x1E` set, flag `0x1D` clear | `$19`: Cui's fight - seen |
| 9 | flags 0, 3, 7 and `0x1B` set, flag `0x18` clear, leader in x 312-359, y 872-919 | `$1A`: Dodoria - seen |
| 16 | flag `0x1F` clear and the leader outside x 384-1407, y 1536-2559 | `$31`: Jeice and Burter - seen |
| 16 | flag `0x1F` set, `0x22` clear, leader inside x 1856-2111, y 736-991 | `$32`: the Ginyu fight; with flag `0x12` set it starts with Goku's landing - seen |
| 16 | flags `0x1F`, `0x22` and `0x1A` set | `$34`: Nail against Frieza, the Dragon Balls, Porunga - seen |
| 11 | flag 2 set, leader's y at least 980 | `$2E`: Vegeta joins, the fight with Zarbon is armed (group `$38`) - seen to start |
| 24 | flag 8 set, `$01B0` at least 4 | `$3F`: see "The last chapter" |
| 24 | flag 8 set, leader in x 2048-2303, y 2560-2815 | `$40`: Goku's arrival and the last fight - seen |
| 34 | flag 3 clear, leader's y below 200 | `$29`: Nail joins and fights the other Nail - seen |

2. **A map object's message.** Each map has a table of objects (seven bytes: type, message id, x, y, and a
   last byte that scripts poke; at `$1180` once the map is loaded). The messages are one table for the whole
   game (368 words at 07:8000, a message = pages of string numbers, `D00n` selecting the portrait). A page can
   hold a control word `E0nn`: reading it runs handler `nn` of a table of 52 at 01:D43B instead of printing.
   Seen: handler `$10` starts scene 5, `$11` starts the scene with the number of the message minus `$66` (the
   message `$77` is the whole Saiyan chapter, scene `$11`: touching the Saiyans on map 5; `$6F` is the recruit
   scene 9), `$13` scene 4, `$17` and `$18` scenes `$14` and `$15`, `$1C` the scene with the number of the map
   minus 2, `$1E` scene `$3C` (the fusion; the messages `$106`, `$13B` and `$13C` end with it).
   The messages of an object come from its talk table (03:9DF1, one word per object id `$30` + the list id):
   entries test a flag (the low 5 bits: only flags 0 to `0x1F`), set one, or end in a message. The map's object
   list is at 02:EAAC (one word per map id, ids ended by `$FF`), the position and sprite of each at 02:EB67
   (five bytes) and the flag that hides it at 02:ECCF. Maps 23 and 24 list two Namekians at x 1656 and 1672,
   y 1152 (ids `$5F` and `$60`): with flag 2 clear they speak messages `$106`, `$13B` or `$13C` (by flags
   `0x10` and `0x11`), which start scene `$3C`; with flag 2 set they only talk. Scene `$3C` first runs effect
   `$33`, which sets flag 1 when Piccolo's record is in the party. Flag 1 set: Piccolo's level rises (effect `$38`),
   flags `0x10` and `0x11` are set (the two vanish), then, if flag 3 is clear, effect `$5A` removes Dende's record
   from the party; flag 7 chooses the closing lines. Flag 1 clear: lines only, flag 2 set (code only). The flag 1 set path is seen (`secret-piccolo-dende-fusion`).

Scenes `$37` to `$3A` are the Frieza chapters' connected steps (code, `$38` seen): `$37` is started by talking to
object `$5C` (maps 18 to 22) and is Piccolo's fusion with Nail when Piccolo is in the party (his level rises by 5);
`$42` (the first forced Frieza fight, group `$53`) ends by setting the story step `$16`, whose caption table
(04:D977 and the three after it: mode, transition target, map, scene) starts scene `$38`: effect `$35` makes the
party Piccolo followed by the ids of characters 3 to 7 that were not in the party when the chapter set-up of map 22
ran (it keeps them at `$7E:29A0`, image offset `$1A0`), restores their hit points and sets Piccolo's BP by his level;
this is how Piccolo returns after Porunga (`secret-piccolo-returns`). A save loaded on map 22 does not run that
set-up, so a crafted save must write the list (`image:1A0=`) itself.

3. **Another scene** (`scene`, or a battle's win address) and **the chapter set-up** (03:EA1E, run at every story
   transition, not when a save is loaded: `title-chapters.md` 5): map 16 and others set `scene_id` and `state_1648`.
4. **A landing**: with `$01A1` bit 7 set (a map change is pending) and `landing_spot` 4 or 9, the
   per-frame check starts scene 0 or 4.
5. **Random battles**: on maps 22 and 24 the counter `$01B0` decides when the forced fights of the Frieza
   chapter come (03:FC93, 02:D2A1, 02:D2F0).

Loading a save rebuilds the party from the roster (03:E5FC) and does not run the chapter set-up (03:EA55 is one
routine per map id, run after a story transition: `title-chapters.md` 5, and `free-progression.md`, which saw it again
on map 16); the map's routine decides from the flags and the position whether a scene starts by itself, which is why a
crafted save can start almost any fight by placing the party.

## Map ids

Map ids are places, and one chapter uses several. From the set-up routines, the random-battle tables
(02:D6EA) and the screens:

| Map | Place |
|---|---|
| 0, 2 | the first chapter's island and cave (enemies: Saibamen) |
| 1, 7 | King Kai's planet (7: Goku; 1: Piccolo's own stay, TR-09) - seen for 7 |
| 3 | Piccolo's cave, where Gohan trains (talk to Piccolo: scene `$04`) - seen |
| 5 | Earth in the Saiyan chapter (recruits, Nappa, Vegeta) |
| 6, 9, 10, 11 | Namek, first chapter there: village (9), archipelago (11), soldiers of Frieza as enemies |
| 8, 12 to 15 | Namek's caves and the Guru area |
| 16 | the Ginyu squad's chapter (landing field, caves) |
| 17 to 23 | Namek islands, the spaceship's rooms (20, 21) |
| 24 | Namek, the last chapter (random battles with the soldiers, the last fights) |
| 32 to 47 | caves; 34 is the cave of the second Nail |
| 48 and up | no map (the load never returns) |

## Modes

`game_mode` (`$24`; bit 7 is a "just entered" flag) is the main loop's state; 00:8B54 jumps through 19 words.
Seen: `$00` the map, `$02` and `$04` a battle (2 starts it), `$03` the menus (Continue's slot list, the Status
screen), `$07` a story transition, `$0A` a minigame (Dende's rescue ends in a flying chase), `$0C` a stage of
the last fight (Frieza's power-up), `$11` the ending (see below).

### Story transitions

Mode 7 runs one of 26 routines of bank 4 (`transition_target`, table at 00:93EE), the cutscenes between
chapters (a caption on a black screen, a flight, a new map and party). `story_step` (`$1228`) is the
index of the caption: tables in bank 4 (04:9A0E, 9A2A, 9A46, 9A62 and D977 to D9D7) give per caption the
next mode, the next map and sometimes the scene to start. Scenes end with `story transition` (29) to
move on: after Raditz `$01`, after the practice fight `$14`, after the Saiyans `$07`, after Dende's
rescue `$0A`, around the Ginyu squad `$10`, `$11` and `$12`.

## The last chapter and the ending

- A won battle (05:97F3) ends in mode 0 except for three groups: after group `$56` (Frieza's last form) or
  `$6E`, mode `$11`; after group `$6F`, mode `$12`.
- Map 24 (03:FC93 and 03:F561): with flag 8 set, the scene `$3F` (Frieza's earlier form, two battles)
  starts once `$01B0` is at least 4, and scene `$40` when the leader is in the square x 2048-2303,
  y 2560-2815: Goku's arrival (he apologizes for being late) and the last fight, group `$56`. The
  fight needs the party list at `$7E:2AA0` and Frieza's hit points at `$7E:2B03` and `$7E:2B0A` in the
  save image (the previous fight leaves them there), else the screen goes black or the VRAM fills with
  noise - seen.
- **Super Saiyan**: when an ally other than Goku falls in this fight, Goku's record becomes unit `$3B`
  with 4 999 999 BP (record byte `+01` = `$3B`, BP `+07`-`+09`) and his attacks change - seen with Piccolo
  and Krillin at 1 hit point.
- **The ending** is mode `$11` (00:96A5): the story is told in captions over a flight, then the staff
  roll, about 11 000 frames, nothing in it reacts to the buttons. At its end the routine checks the party:
  Vegeta is dropped from it unless Goku's unit is `$3B` (a Super Saiyan); if Vegeta is still there and
  alive a battle starts, group `$6F` (enemy unit `$4B`, "Super Vegeta"), after Vegeta's speech about the true Super Saiyan power - seen. A win sends the game to mode `$12`.
