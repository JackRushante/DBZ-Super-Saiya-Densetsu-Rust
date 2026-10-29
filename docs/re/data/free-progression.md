# Free progression: every place the story reads a level, EXP or BP

For the mod "Free progression" (M10 Task 19, the feature `free_progression`, `docs/mods/features.md`): the story should move on what
the party has done, not on how strong it is. This note lists **every place of the game's code that reads the level (record `+02`),
the EXP (`+0F..11`) or the BP (`+07..09`) of a party member and decides something with it**, what each one gates, and what the
feature does with it. It completes and replaces Part B of `kaioken-bp.md` (gates G1 to G12 and the floors and gifts of B.3: the
numbers G1 to G12 of that note are kept in the last column of the table of gates).

Tags: **SEEN** = read in a listing (`xtask disasm`, the address is given) or run headless (the command is given); **INFERRED** = a
conclusion from what was seen. Original code is described, not quoted (a research note quotes at most five consecutive instructions). Addresses are `bank:addr`; `$xxxx`
are bus addresses of bank 0 unless a bank is given; the party records are nine of 32 bytes from `$0B00` (`characters.md`), the
roster image twelve from `$7E:2800` (`save-format.md`).

## 1. The method, and what it covers

1. **The scan** (SEEN, `python tools/re/story_level_reads.py`, which prints the counts below): every instruction of the whole ROM whose operand is a byte of the
   three fields of a party record (`$0B02`, `$0B07..09`, `$0B0F..11` of each of the nine records, absolute and indexed), the same
   fields of the roster image and of the BP snapshot (`$7E:2800..`, `$7E:4C00..`, long addressing), of the enemy slots (`$0E00..`)
   and of the two side records of a battle (`$1400`, `$1440`), and the record-relative forms (`$0002,Y`, `$0007,X`, `$000F,Y`...
   for a record base held in a register). 1 369 operands matched in the 32 banks; 541 of them lie in the banks that hold code
   (0 to 5, the code of 6, 10, 13, 1C and 1D) and 437 in the code ranges of those banks; they fall in **162 routines**. A second
   scan found the `LDY #field` before an indirect load through a direct-page pointer (23 places: all are readers of the
   bank byte of a 24-bit pointer or of a command list, none reads a record) and a third the 328 immediates that load a record
   base or limit (59 of them a party record's: block copies of the party and the roster, loops over the flag byte and the unit
   id, and loops of the battle code; those that read a level, EXP or BP do it through the forms of the first scan).
2. **Every one of those 162 routines was read in context** with `xtask disasm` (M8 X16 unless the listing says otherwise). **82
   of them are data read as code, or the operand bytes of another instruction** (the window around the hit decodes to nothing
   sensible, or the hit is on no instruction of the routine that the tool follows). **80 are code**: 13 hold a gate (section 2),
   9 a floor or a gift (3), 14 are level mechanics (4), 10 displays, 8 snapshot copies, 3 object effects and 23 battle
   mechanics. The enemy slot's bytes `+0F..11` are not EXP: the battle code uses them as something else (INFERRED: screen
   positions; `01:F4F9`, `01:F9EE`, `01:FA3A`).
3. **The story's own code was read apart from the scan**: the 28 callers of `03:F021` (the finder of a party member by character id:
   `A` = id, `Y` = record offset, carry set when absent), which every story compare of a member goes through (SEEN: eight read a
   level right after the call, `02:D479`, `02:D488`, `03:F228`, `03:F235`, `03:F302`, `03:F312`, `03:8685`, `03:FA72`; the others
   test that the member is present, set a flag, change a unit id or save the BP), the per-map trigger routines (the table at
   `03:F1AE`), the per-map chapter set-ups (the table at `03:EA55`), the effect routines the scenes and the scan point at, the 52
   message handlers (`01:D43B`: they lie in `01:D4A3..DBD6` and no hit of the scan is among them), and the 82 scene scripts
   (`python tools/re/scenes.py`): they poke 38 distinct addresses, none in a party record or the roster (SEEN), and reach
   levels only through `$164C`/`$164D` (the level routine's parameters), the effect routines and the `1C` command that adds a member.
4. **Nothing in the story reads EXP**: the EXP bytes are read by the level-up check (`03:841E`), the Status screen and the awards
   of a fight only (section 4).

### Not covered (INFERRED limits of the method)

- A read through an index that already holds the field's offset (`INY INY` then `LDA $0B00,Y`), or through a pointer built in RAM
  from a computed address: only `05:B4A2` was found by reading (kaioken-bp.md 9).
- Code reached only by a computed jump whose table was not listed. Listed and read: the effect routines that the scan or the scenes
  point at (`$35`, `$38`, `$4A`, `$5B`, `$66`, `$67`, `$51` to `$53`), the trigger table, the set-up table, the message handlers' range,
  the command dispatch (`03:AC4C`). The 110 effects were not read one by one: the scan covers their reads of a party record, and
  the state code that follows each `JSR $03:8000` inside an effect is not followed by the disassembler.
- The 59 immediates of a party record's base were read in bank 3 only (SEEN: `03:E373..E38C`, `03:EFB1`, `03:D3E0`: copies
  and a screen position), not one by one in the other banks.

## 2. The gates (twelve: each decides an event, a fight, a flag, a party change or a stat)

"Replacement" is what the feature `free_progression` does (section 6). "Old" is the number in `kaioken-bp.md` B.1.

| # | Site (instruction that reads) | Reads | Condition | Gates | Replacement | Old |
|---|---|---|---|---|---|---|
| S1 | `03:F22C`, `03:F239` (the map 0 trigger `03:F212`: Goku, else Piccolo) | the level of Goku; of Piccolo | flag 3 (the Gohan card) set, flag `0x0B` clear, Goku at 5 or Piccolo at 4 | starts scene `$02`: the forced fight with Raditz (`JMP 03:F7DE`) | the load gives 5 (4) when flag `0x0A` is set (the imposter in the cave is beaten), else 0 | G1 |
| S2 | `02:D47F`, `02:D48E` (`02:D477`, maps 0 and 2) | the same two levels | the same two thresholds | zeroes the encounter level and zone (`$0D8A/$0D8B`): the random battles stop; reached from `02:D428`, called by the zone update `02:D37D` | the routine returns at once: nothing stops them | G2 |
| S3 | `03:F308`, `03:F318` (`03:F300`, map 5) | the level of Piccolo, else of Gohan | Piccolo at 9, Gohan at 10 without Piccolo | the body at `03:F320`: flag `0x1D` and `$0174 = 8`, the recruits not found yet are killed, the random battles of map 5 stop | both compares read 0 (the Remastered story fix does the same at `03:F320`); the walk into the Saiyans (scene `$11`) keeps its own deadline | G3 |
| S4 | `05:8791` (the battle state `05:872A`, substate 2) | the level of the acting record (category 0, class not 6) | at least 47, group `$56` or `$6E`, a friend has fallen (`$14D2`) | the transformation phase `$1A`: message `$68`, the Kaioken state cleared, `03:8683` runs | the compare reads 47 if the level is lower | new |
| S5 | `03:868B` (`03:8683`, called by `05:C45F` in phase `$1A`) | Goku's level | at least 47, group `$56`, the party-size test (`$01BB` against `$0D9A`) | `03:85F0` builds the Super Saiyan: unit `$3B`, level 48, BP from the row | the compare reads 47 if the level is lower | G5 |
| S6 | `03:EDF2` (the map 16 set-up `03:ED9C`) | Goku's level in the roster image (`$7E:2802`) | below 34, 34 to 35, 36 or more | flags `0x10`, `0x11` or `0x12` (`$0172` bits 0 to 2): Goku joins in the first, the second or the last fight of the squad (scenes `$2F`, `$30` and `$32` test them: groups `$4D`, `$4E` and `$51`) | the parameter `ginyu_arrival` (0 = the rule, 1 to 3 = that arrival) puts 32, 34 or 36 in the compare | G6 |
| S7 | `03:C58D` (effect `$35`, Piccolo returns) | the level of the first record (Piccolo) | none: it is the index of a BP table (`level - 33`) | assigns Piccolo's BP from the table at `03:C5BA`, which holds the levels 33 to 40 (eight values); any other level reads past the table (SEEN below) | for a level outside 33 to 40 the BP the record has is stored back | (B.3) |
| S8 | `05:9183` | Dende's level | at least 3, group `$55` | Dende heals one ally in the fight | stays (section 5) | G7 |
| S9 | `01:99BB`, `01:A0DC` | Dende's level (record `$0C20`) | at least 3, `$0C38` bit 7 clear | the map menu's Dende healing | stays (section 5) | G8 |
| S10 | `05:AC3E`; `05:AC4F`, `05:AC57` against `05:AC7D`, `05:AC86` | Gohan's level; his BP and each enemy's | level 11 sets `$14EA` bit 4; BP times 10 against each enemy's BP | Gohan's rage in a fight | stays (section 5) | G9 |
| S11 | `03:8D01`, `03:8D0A`, `03:8D75`, `03:8D7E` (and the four `SBC` of the enemy's BP) | the party's BP against each enemy's | the party below an enemy | the Auto choice: the Kaioken (`$0C36 = $80`, unit `$3A`, command `$33`) or the Spirit Bomb (`$0C37 = $80`, command `$35`) | stays (section 5) | G10 |
| S12 | `05:BCF2`, `05:BCF9`, `05:BCFF` against `05:BCF6`, `05:BCFC`, `05:BD02` (the enemy's condition checks `05:BC5B`) | the BP of the party's category-`$0F` member (or any member) against the enemy's | a carry | one of the conditions of the enemy's choice of action, with a random gate of 1 in 2 or 1 in 4 | stays (section 5) | G11 |

Counts: 12 gates; the instructions that read a level, EXP or BP of a party member in them: S1 2, S2 2, S3 2, S4 1, S5 1, S6 1, S7 1,
S8 1, S9 2, S10 3, S11 4, S12 3: **23 reads of the party's numbers** (S10 to S12 compare with enemy numbers too: 2, 4 and 3 more reads).

Details that the table cannot hold (all SEEN):

- **S1/S2: what the opening leads to.** The only object of map 2 is the imposter Raditz, hidden by flag `0x0A` (`02:EB14` list, hide byte
  `0A` at `02:ECCF`); scene `$03` (`python tools/re/scenes.py 03`) plays his fight (group `$0E`) and sets flag `0x0A` when it is won.
  Flag 3 is set by scene `$00` (the Gohan card at Mr. Popo's house). Scene `$02`, the forced fight, sets flag `0x0B` first. The map 0
  trigger tests flag `0x0B` and flag 3 before it reads a level, and does not test the carry of `03:F021`: with a member absent the `Y`
  is `$0120` and the level read is garbage.
- **S2** stops the battles on both maps with the same two thresholds. The rule of map 5 (`02:D45D`) leaves them alone once flag `0x1E`
  (Nappa met) is set, stops them when flag `0x1D` is set and otherwise runs `03:F300` (S3). Maps 32 and up never fight; map 24 stops
  them once flag 8 is set and clears the battle counter `$01B0`.
- **S4/S5.** The battle state tests the group first (`$56`, or `$6E`), then the category of the acting record (0: Goku's unit ids
  `$00`, `$39`, `$3A`, `$3B`, `$3C`), then that its class byte is not 6 (the form is not built yet), then the level, then `$14D2`
  (set by the KO routine `05:A9AA` when a party fighter of category 9, 1, 15, 10, 11, 12 or 13 falls). Phase `$1A` calls `05:C45F`,
  which calls `03:8683`. The builder `03:85F0` assigns level 48 whatever the level was, so a Goku of level 20 becomes a level 48
  Super Saiyan with 4 999 999 BP exactly as the original's Goku of level 47 does.
- **S6.** The three flags are three arrivals of Goku: scene `$2F` (Recoome and Guldo) makes him join if flag `0x10` is set, `$30`
  (Jeice and Burter) if `0x11` is, `$32` (the Ginyu fight, a landing) if `0x12` is. In the original a Goku with more levels from the
  gravity room arrives later. The chapter set-up runs only after a story transition (`00:939B`), not on Continue
  (`title-chapters.md` 5, SEEN again: a save loaded on map 16 leaves `$0172` as the save has it; `scenes.md` says the opposite and
  is wrong).
- **S7.** For the levels the original reaches (the floors give Piccolo 13 at map 1, then +5 +4 +2 +1 and +20 from scenes: 33 to 40
  with the fusions) the table is right; at level 25 the BP read is 9 374 473 and at 45 it is 41 475 (SEEN, the proofs below).
- **S10 to S12** are read in section 5.

## 3. The floors and gifts (they write levels: not gates)

All raise a level through `03:C64A` (`$164C` = the character id, `$164D` = the levels to add; it finds the party record of that id
and calls `03:851B`). **Measured** (SEEN, `free_progression_real_rom`, test `a_floor_applied_above_its_level_adds_one_level...`):
a floor of 8 applied to a Goku of level 1, 4, 7, 8, 9, 12 or 30 leaves him at 9, 9, 9, 9, 10, 13 and 31: the routine raises to
`target + 1`, and a character already at or above the target gains exactly one level. The code also sets his EXP to the start of
the new level and does nothing at the cap (`03:850E`) (INFERRED from `03:8527..8591`, not run). So **no floor or gift breaks at a
level above the original's range**: it never lowers a level and never raises more than one level beyond its target. Because the
delta is `target - level` modulo 256, a target below the level wraps and still gives the target.

| # | Site | Context | Floor or gift |
|---|---|---|---|
| F1 | `03:EAED` (map 1 set-up `03:EAC4`) | Piccolo alone | floor 13 |
| F2 | `03:EBB1` (map 5 set-up `03:EB54`) | Piccolo, from the roster image (`$7E:2842`) | floor 7 |
| F3 | `03:EC47`, `03:EC55` (map 7 set-up `03:EC25`) | Goku | floor 7 if his level is below 4, else 8 (the routine ends at the target plus one, see above) |
| F4 | `03:EE5F` (maps 18 to 20, `03:EE2A`) | Goku alone | floor 13, then scene `$50` (if `$0180` is non-zero it sets `$0170` bit 1 instead) |
| F5 | `03:FA79` (scene `$3F`'s hook `03:FA3F`) | Goku | to 49, which stops at his cap 47: what made S5 reachable |
| F6 | `03:D211` (effect `$67`) | every party member | to Piccolo's roster level; sets each unit id from `03:D249` |
| F7 | `03:CFC5..CFE1` (effect `$5B`, with the change of school: the unit ids from `03:CFFC`) | the party members of ids 0 to 7 and 9, 10 | +2 each |
| F8 | scenes `$04` (+3, +2 with Piccolo present: Gohan), `$37` (+5, Piccolo), `$3C` (+4 or +2, then +1: Piccolo), `$4C` (+20: Piccolo) | effect `$38` = `03:C64A` | gifts |
| F9 | `03:CBF5` (effect `$4A`) | Goku | floor 11; **no scene runs effect `$4A`** (SEEN: not in the 82 scripts): unused |
| F10 | `05:8E3F..8E55` (a fight that ends with `$01D6` bit 7 set; `00:95A9`, the init of mode `$0D`, sets it, and the run in which the party was wiped went to mode `$0D`) | the first record | sets the EXP to the threshold two levels up (`05:C93A` twice: the start of the next level, read from the table row of the level), lowers the level byte by two and runs the level-up check: +2 levels. It is the **win of a gravity trial** of the spaceship (scene `$45`, groups 91 to 100, units 86 to 95), not a catch-up after a defeat (SEEN: a won trial of the phone's game at Goku's level 48 ran it); past the 48th row it reads the bytes after the table |
| F11 | `00:8A4A`, `03:84B6` | Vegeta | floor 17 when he is built or levelled |
| F12 | `01:E420` (an object), `05:C8D5` (the Porunga card) | the chosen member | +1 level: the object caps at 99, the card at the cap table (47, 48 for Gohan) |
| F13 | `03:CD2E..CD8E` (effect `$52`, one of King Kai's trials, effects `$51` to `$53`) | the first record | adds a trial's EXP, then the level-up check: the only EXP source outside a fight |
| F14 | `03:A587` | the party's highest level | a price band of the training arena (a reward scale) |

Levels given by **joins** (a script constant, the `1C` command, not a read): Krillin 8 (scenes `$05` and `$06`), Yamcha 8 (`$07`), Tien and
Chiaotzu 8 (`$08` and `$09`), Dende 1 (`$16`), Nail 36 (`$29`, `$2D`), Vegeta 25 (`$2E`). They stay.

## 4. Level mechanics, displays, snapshots and battle mechanics (not story: out of scope)

These read or write the three fields but decide nothing in the story. Listed so that the scan is accounted for (SEEN unless said).

- **The level rows and builders**: `00:89B8`, `00:8A07`, `00:8A28`, `00:8A69` (the rows added at a level, unit rows for the ids from
  8), `03:84A5`, `03:84EE` (the cap check, carry set below the cap), `03:851B` (the raise), `03:841E` (the level-up check: it
  compares the EXP with the next level's threshold), `03:85F0` (the builder of forms), `03:B203`, `03:BB46`, `03:BCF2` (new records),
  `01:F719` (the enemy builder).
- **Caps**: the table at `03:850E` (Goku 47, Gohan 48, Piccolo 47, Krillin, Tien, Yamcha, Chiaotzu 48, Vegeta 47, Dende 3, ids 8 to
  10 none), read by `03:84EE`/`03:8476`/`03:8610`; the constants 47 (`03:868E`, `05:C8C5`), 48 (category 9) and 99 (`01:E420`).
- **Displays**: Status and the command screens (`01:A88C`, `01:A8DC`, `01:AC24`, `01:BF8F`, `01:C033`, `01:DCEE`, `01:EEE6`), the BP of
  an enemy (`01:C056`), the Scouter's running count (`01:E69F..E6FF`), the battle HUD (`1D:8479..851B`).
- **Snapshots and objects**: the BP snapshot at `$7E:4C00` (`01:FE64..FEA4`, `03:C695`, `03:C8A7`, `03:E10D`, `03:E556`,
  `05:8336`), the BP objects (`01:DF3E..DF6C`, `01:E4AE..E4BE`, `01:E568..E59B`), the stat objects (`01:E420..E45A`).
- **Battle mechanics**: damage (`05:B6F4..B736`), the special-attack roll (`05:BAFF`, `05:BB18`), the Kaioken (`05:AB60`), the form
  changes (`05:A52E`, `05:A727`), Body Change (`05:D2AD`), the test of 10 000 BP on both sides (`05:DDD3..DDEE`), the order helper
  (`03:829D`), the party and enemy BP sums (`03:804B`, `03:8084`; used by the enemy's chance to flee at `01:F890`, which compares the
  two averages), the pose by level (`05:AF9E`).
- **Records of other kinds** the scan met by their bytes: the enemy slots' `+0F..11` (screen positions), the data tables of bank 2
  (units, command lists, level rows).

## 5. What the feature leaves alone, and why

- **S8, S9 (Dende's healing at level 3).** Dende's cap is 3 (`03:850E`), reached by EXP in the fights he joins: it is an ability of a
  fighter that a growing character unlocks, not a story event, and nothing in the story waits for it.
- **S10 (Gohan's rage).** A battle event: from level 11 a Gohan outclassed by an enemy (his BP times 10 below its BP) does double
  damage. The compare is between the two sides, so an enemy that follows the party keeps the same relation, and a harder difficulty
  makes the rage come more often, which is the point of the setting.
- **S11 (the Auto choice).** The Kaioken and the Spirit Bomb are chosen for Goku when the party is below an enemy in BP: the same
  relative compare, a battle event; with scaled enemies the margin the difficulty gives decides, as intended.
- **S12 (the enemy's choice of action)** is battle arithmetic of the enemy's side.
- **The floors and gifts** only raise levels (section 3), the **caps** (47, 48, 3) are not changed by the feature: a data pack with
  `tables.toml` may (Batch 4a), and the hooks are written so that a level above 47 passes S4 and S5.
- **Joins at a fixed level** (section 3) and the **rewards** (`03:A587`) are not gates.

## 6. What the feature does (Part B), hook by hook

All hooks are `Patch` hooks (`HookMode::Patch`): they cost nothing, never call the original, and decline at once, with `resume_at`
their own key, when the feature's slot is not in the image. Widths are the accumulator's at the instruction ("M8": 8-bit).

| Hook | Key | Instruction it stands for | Width | What it does | Ends with |
|---|---|---|---|---|---|
| `mod_fp_raditz_goku` | `03:F22C` | `LDA $0B02,Y` (Goku's level), 3 bytes | M8 | Goku found (carry clear): A = 5 when flag `0x0A` is set, else 0 | `resume_at(03:F22F)`, after the load it replaces |
| `mod_fp_raditz_piccolo` | `03:F239` | `LDA $0B02,Y` (Piccolo's level), 3 bytes | M8 | the same with 4 | `resume_at(03:F23C)` |
| `mod_fp_encounters` | `02:D477` | the entry of the maps 0 and 2 stop | M8 | returns, the encounter level untouched | the entry's return (`RTL`, cost 0) |
| `mod_fp_deadline_piccolo` | `03:F30B` | `CMP #$09` | M8 | A = 0 | `resume_at(03:F30B)`, its own key |
| `mod_fp_deadline_gohan` | `03:F31B` | `CMP #$0A` | M8 | A = 0 | its own key |
| `mod_fp_ssj_trigger` | `05:8794` | `CMP #$2F` (battle state) | M8 | A = the larger of A and 47 | its own key |
| `mod_fp_ssj_build` | `03:868E` | `CMP #$2F` (builder's guard) | M8 | the same | its own key |
| `mod_fp_ginyu_arrival` | `03:EDF6` | `CMP #$22` after the 4-byte `LDA $7E2802` | M8 | A = 32, 34 or 36 for `ginyu_arrival` 1, 2, 3 (0: nothing) | its own key |
| `mod_fp_piccolo_bp_low` | `03:C59C` | `LDA $03C5BA,X`, 4 bytes (the BP's low word) | **M16** | for a level outside 33 to 40: A = the record's own BP word, which the store after it writes back | `resume_at(03:C5A0)` |
| `mod_fp_piccolo_bp_high` | `03:C5A7` | `LDA $03C5BC,X`, 4 bytes (the high byte) | M8 | the same for the high byte (the record offset is then `Y + 2`) | `resume_at(03:C5AB)` |

Keys are the recompiler's stops too (`xtask recompile` must run again before a recompiled build honours them).

## 7. Proofs (SEEN: commands and numbers)

The ROM-gated test `crates/ssd-game/tests/free_progression_real_rom.rs` runs each gate in the game's own code, staged through a
`JSL` written at `$7F:FE00`, in three worlds: the plain machine, the machine with the redirectors and no pack (it must give the
plain numbers), and the machine of the example pack `examples/free-progression`:
`SSD_ROM_PATH=... cargo test -p ssd-game --release --test free_progression_real_rom -- --ignored --nocapture --test-threads=1`.

Headless runs on crafted saves (`xtask save-edit` on the saves of `tools/saves/secrets.sh`, `replay-from-save` with the named timeline,
`xtask shots <replay> [--mods examples/free-progression]`; the worlds differ only by `--mods`); `tools/saves/free-progression.sh [name...]`
plays all of them and prints what the story did (about five minutes):

| Gate | Save (the `--set` list on the named base) | Without the pack | With the pack |
|---|---|---|---|
| S1 | `secret-raditz`, `goku.level=1 piccolo.level=1 flags=+0xA`, `mash-3000` | no scene in 2 700 frames | scene `$02` at frame 2226, flag `0x0B` at 2229 |
| S1 | `secret-raditz` as it is (levels 12, imposter not beaten) | scene `$02` at frame 2226 | no scene in 3 300 frames |
| S2 | the same replay, `--watch encounter_level,encounter_zone` | level and zone set to 1 and 2 at frame 2132, zeroed in the same frame | stay 1 and 2 |
| S3 | `secret-recruit-krillin`, `piccolo.level=9` (and 30), `recruit-krillin` | at frame 2132 `$0173 = $20` (flag `0x1D`), `$0174 = 8`, `$0172 = $0F`: the recruits are gone, Krillin is not recruited at 2529 | no write to `$0173` or `$0174`; `$0172 = 01` at 2529: Krillin is recruited |
| S4, S5 | `secret-ssj-frieza`, `goku.level=20` and Goku's level-47 numbers (`maxhp=745 hp=745 maxki=138 ki=138 bp=260090 exp=170000`), `mash-7000` | at frame 9 300 Goku is unit `$00`, level 20, BP 260 090 | phase `$1A` at frame 7822; unit `$3B`, level 48, BP 4 999 999 |
| S6 | `secret-ginyu-jeice-burter`, `image:02=0A` (Goku's roster level 10), a quiet map 16 (`flags=none,+1,+3,+7,+0x1F,+0x22`), `--wram 2500:7E0024=86` (mode 6: the set-up runs) | `$0172 = 01` | `$0172 = 04` |
| S7 | `secret-piccolo-returns`, `piccolo.level=25` (and 45), `frieza-flees` | BP 9 374 473 (41 475 at 45) in record 0 after scene `$38` | 11 853 (1 010 033 at 45) |
| S7 control | the same at level 40 | 210 000 | 210 000 |
