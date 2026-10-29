# Battle: damage, EXP, zeni, encounters and enemy groups

How a battle turn turns cards and battle power into damage, where EXP and money change, how random encounters are
rolled and how an enemy group becomes five enemy records. Found for milestone M5 (the cheats screen): every place
a multiplier or an override must act. Read from the code of banks `$00`, `$01`, `$02`, `$05` and `$1D` and checked
headless on the reference replays (`battle`, `landing`, `secret-raditz`, `secret-frieza-*`) and on crafted saves
(`xtask save-edit`, `replay-from-save`). Certainty: "seen" = a run with a patched WRAM byte shows it, "code" = read
in the code only. Routines have no symbol (the file only holds routines that have a Rust function) and are written
as bank:address; tables and variables are named as in `re/symbols.toml`.

## State of a battle

The battle works on two *side records* (32 bytes each, the layout of the party record in `characters.md`): `side_record_a`
(`$7E:1400`) is the party fighter whose turn it is and `side_record_b` (`$7E:1440`) the enemy. A turn copies the fighter's
record from `party_records` (`$0B00`, the offset is the word `$14C3`) or the enemy slots (`$0E00`, offset `$14C5`) into
its side record (05:AE81, 05:AE16) and copies it back when the turn ends (05:9D5B to 05:9D8A). Everything below reads and
writes the side records. Fields used (offset in the record):

| `+` | Meaning |
|---|---|
| 00 | flags; bit 6 = in the battle |
| 01 | unit id (`unit_stats` row; party members use the rows of `party_unit_ids`) |
| 02 | level |
| 03..04 | HP; 0A..0B maximum HP |
| 07..09 | BP (24 bits) |
| 0F..11 | EXP (party records only) |
| 14, 15, 16 | the card in play: first value (attack), second value (defence), symbol (`cards.md`) |
| 17 | the *defence factor* (`unit_stats` byte 9, 20 to 50), copied at the start of the battle |
| 1A | the hand slot the card came from |
| 1B | the fighter's class, `unit_stats` byte 0 (06:E8EE copies it for the party at the start of the battle) |
| 1E | the *attack factor* (4 in every record seen) |

| Variable | Address | Meaning |
|---|---|---|
| `active_side` | `$14C2` | bit 0: 0 = the party fighter acts (side A attacks, side B defends), 1 = the enemy acts. Bit 1: the *counter-attack* (after a dodge the defender may strike back, with the card's second value; 05:BC4D toggles bits 0 and 1, the chance comes from the BP difference). Bit 3: a special attack (05:BB46; its power comes from the attack table through 03:8752) |
| `battle_hit_flag` | `$14CC` | 0 = the attack is dodged ("Goku dodges the attack!": damage is computed, then not applied), non-zero = it hits |
| `battle_hit_class` | `$14CD` | the severity of a hit, drives the animation: 2, 3 or 4 by the damage against the defender's maximum HP (below), 1 a graze, 0 a miss |
| `battle_damage` | `$14CF` | the damage of the turn, 24 bits (it is the number the message prints) |
| `battle_repeat` | `$14DF` | how many attacks in a row the same side has made on the same target (05:B760): 1, 2, 3... |
| `battle_exp_award` | `$14F3` | the EXP of the battle so far, 24 bits (sum of the defeated enemies) |
| `battle_drops` | `$14F6` | three bytes: the objects the enemies drop (`$FF` = none) |
| `enemy_count` | `$0EA0` | enemies still standing |
| `enemy_formation` | `$0EA5` | which screen formation the group uses (computed by 01:B8DE from the count) |

## The damage (party to enemy and enemy to party)

One routine computes it for both directions: **05:B7FC** (entry with `Y` = 3) and **05:B801** (`Y` = 2) share the body at
05:B804. They are called by the battle's attack states (05:8AF1 calls B7FC; 05:899C and 05:8BED call B801). The hits of the reference
fights were computed with the shift of B801. Seen: the first hit of the reference battle (Goku on a
Saibaman, 19 damage) and the second (Piccolo, 15) are reproduced to the unit by the formula below with the numbers in
the records.

With `BPa` and `BPd` the attacker's and defender's battle power, each *plus 25 percent* if the fighter's card symbol equals
its class or is 7 (05:B6F2, `cards.md`):

1. `va` = the attacker's card value, `+14` (or `+15` for a counter-attack, `$14C2` bit 1); a Z (8) counts as 10 (05:B7A5).
   `K` = the attack factor `+1E` (for a special attack the factor of the attack table), plus twice `$14D3` (1 when the
   target dodged); doubled when the attacker is the party's unit 9 (Gohan) and `$14EA` bit 6 is set.
2. `P = BPa * (va + 4) * K + (BPa - BPd)`: the BP difference is added when the attacker is the stronger and subtracted
   when it is the weaker (the sign decides the branch of step 5, 05:B807 and 05:B87C).
3. `vd` = the defender's card value, `+15`, Z = 10; `n = $14DF`. `F` = the defender's `+17` (05:BBBC adjusts it in a few status cases, `$14C7` bit 6; plain `+17` in the reference
   fights). `D = (BPd * (max(vd - n, 0) + 1 + 4) * F / 10) >> (Y + 1)`: a shift of 3 for B801, 4 for B7FC.
4. **damage = P / D**, stored in `$14CF`..`$14D1` (saturating to `$FFFFFF`; 1C:806C multiplies 24 by 16 bits to 40 and
   1C:8136 divides 40 bits).
5. `$14CD`: when the attacker is the stronger (the normal case) 4 if the damage reaches three quarters of the defender's
   maximum HP, 3 if it reaches an eighth, else 2; the weak attacker gets 0 for a damage of 20 or more and 1 below. Some
   story fights then force or limit it (group 5, Raditz; groups `$38` and `$39`, Zarbon).

Worked check (seen, `battle` replay, hit 1): Goku BP 416 with a card of value 5 and symbol 1 (his class), so 520; the
Saibaman BP 326, card value 4, `F` = 30, `n` = 1, `K` = 4: P = 520 x 9 x 4 + 194 = 18 914; D = 326 x (3 + 1 + 4) x 30 / 10
>> 3 = 978; 18 914 / 978 = 19. Hit 2 (Piccolo, BP 408, symbol 3, no match, `n` = 2): P = 13 138, D = 855, damage 15.
Forcing the symbol to 7 on hit 2 gives 19 and to 0 on hit 1 gives 15, which is the 25 percent (seen).

### Where the damage reaches HP

05:A435, the attack's resolution step (called from the states at 05:89B8, 05:8BB3 and 05:8C70 once the damage is known),
does the following in order: if `$14CC` is 0 it returns (dodge, nothing is applied); if the damage `$14CF` is 0 it returns;
else it calls **05:BDDC**, which takes the defender's side record (`05:B6C0`: the *other* side from `active_side` bit 0),
subtracts `$14CF`/`$14D1` from its HP word (05:BE04 to 05:BE0A) and returns carry clear when the HP went below zero
(code: when `$1700` bits 0 and 1 are 1 or 2 it first redirects the damage to the other side's record, and at 3 it applies none). BDDC
has one caller (05:A435 at 05:A46B) and is the only place a hit lowers HP: watching the HP bytes over the reference battle
and the Raditz fight shows no other writer than BDDC, the KO routine 05:A9AA (HP set to 0) and the record copies. The story
protections run after it (05:A484 to 05:A4A6: the Ginyu fight, group `$51`, sets the enemy's HP to 1 instead of letting it
die until its scripted moment).

Direction: the defender is the enemy slot (`$1440`) when `active_side` bit 0 is 0 (party attacks) and the party fighter
(`$1400`) when it is 1. The message "N damage!" is printed afterwards from `$14CF`, so a change made to `$14CF` before
BDDC subtracts shows in the message (seen: damage doubled, "38 damage!" printed, enemy HP 55 to 17).

**Where a damage or defence multiplier acts.** At the entry of 05:BDDC: if the defender is side B multiply `$14CF`..`$14D1`
by the damage cheat; if it is side A divide by the defence cheat (keep at least 1 when it was positive, saturate at
`$FFFFFF`). Seen (patch at 05:B9BD, the store of the result, equivalent): damage x2 in the party-to-enemy direction gave 38
and 30 where 19 and 15 were; defence /2 in the enemy-to-party direction turned Raditz's first hit on Goku from 16 into 8 and
Goku's HP from 210 to 202 instead of 194.

## EXP and money at the end of a battle

- Each defeated enemy: 05:A9AA (the KO routine, called when an enemy's HP reaches 0) calls **1D:8C33**, which adds the enemy's
  EXP (`unit_stats` bytes 11 to 13, 24 bits) to `battle_exp_award` (`$14F3`..`$14F5`).
- At the end of the battle the state at **05:8FAA** runs once per frame (index `$14F9` steps through the nine party
  records, `$20` bytes a frame): for every record with flag bit 6 set (and unit id not `$4A`) it adds `$14F3`..`$14F5` to the
  record's EXP (`+0F`..`+11`, 05:8FC8 to 05:8FDA), then calls 03:841E (the level-up check). Every fighter gets the full
  amount; there is no split. `$14F3` is cleared at 05:970A (battle start) and 05:97E3 (battle end).
- **EXP multiplier**: scale `$14F3`..`$14F5` once, at the return of 1D:8C33 (or after the last enemy dies, before 05:8FAA
  starts to add). Seen: x3 at the return of 1D:8C33 turned the Saibaman's 9 EXP into 27 for Goku (`$0B0F`, 9 to 27) and
  the victory message ("N XP gain", read from `$14F3`) printed 27.
- **Money.** A battle gives no money: nothing in the battle code writes `zeni` (`$01B4`), and in the 48 036 frames of the
  `story-1` replay (about seven battles, the level-up of the first chapter) the byte is written only at power-on (seen). The only writers in the ROM are
  01:D7F9 (a map object event adds the amount in `$0D53`, capped at 999), 03:A790 (selling an object adds its price, capped at
  999), 03:A640 (a credit of half a product plus a figure, not decoded; capped), and the payments 01:988A (`zeni` minus
  `$10C6` when `$01D6` bit 6 is set), 03:A66B and the shop's buy (03:A914, `menus.md`). The cap 999 (`$03E7`) is a constant
  of each of those routines. A "maximum zeni" cheat is therefore a plain write of 999 to `zeni` (a word), at the moment
  the cheat is switched on and again whenever the money is shown or spent (every frame is simplest). The battle drops
  objects, not money (05:8F4D, up to three objects in `$14F6`, refused when the list is full).
  The ROM holds six writers of `zeni` in all (a scan of the store instructions to `$01B4`: 01:98A5 and 03:A68A payments, 01:D80A, 03:A661, 03:A7AC, 03:A975), and the one that is a gift is
  01:D80A, the tail of 01:D7F9, a target of the script event table at 01:D43B (event number 20, read from `$072E` by 01:D433 and 01:BF44): it adds the word `$0D53` to the zeni, capped at 999. Two scripts set
  the amount (03:9D0E and 03:A226 store 40 in `$0D53`). The money multiplier of the difficulty presets scales `$0D53` at the entry 01:D7F9 and lets the event run (seen: 40 added to 100 zeni gives 180 at x2, 120 at x1/2, 140 at x1).

## Enemy groups

`encounter_group_index` (`$0EA3`) names a row of `encounter_groups` (five unit ids, `$FF` empty). Two routines turn it into
battle records; the standard battle set-up 01:F4A2 calls them one after the other, and 00:961A and 05:9852 (a form change
of Frieza, groups `$54` and `$6E`) call the same pair:

1. **01:F692** clears the five enemy slots (`$0E00` + `$20` x slot) and copies the row: for every id that is not `$FF` it
   sets the slot's flag byte (`+00` = `$40`), its unit id (`+01`) and a copy of the id at `+1C`, and counts them in
   `enemy_count` (`$0EA0`); then 01:B8DE derives the screen formation `$0EA5` from the count.
2. **01:F719** fills the records from `unit_stats` for every slot whose flag is set: `+02` level (row byte 1), `+03` and `+0A`
   HP (bytes 2..3), `+05` and `+0C` KI (7..8), `+0E` (byte 10), `+17` (byte 9), `+1B` (byte 0), and `+07`..`+09` BP: the
   row's BP minus its two low decimal digits read as one byte, plus or minus a figure from a small table (01:F7F2, picked by
   the digits), decided by the random generator (the Saibaman row says 441 and the record had 326, seen).

The formation is a function of the count alone: 1 enemy uses slot 0, 2 use slots 1 and 2, 3 use 0, 3 and 4, 4 use 1 to 4, and 5
use all (slot 0 is the centre; the `catalogue --kind encounters` rows show it: a two-enemy group is `-, X, Y, -, -`). 01:F719
even starts at slot 1 when the count is even, so a patch that changes the count must also move the units into the slots of the
new count's formation.

- **Random or story battle.** The only caller of the group pick `02:D34A` is `02:D250`, in the normal-battle branch of the
  roll that starts a battle (`02:D241`); a script's battle pokes `$0EA3` and the mode itself, and the story branches of the roll
  (carry set from `02:D289`) never pick. So a hook at the pick's `RTL` (`02:D37C`) sees every random encounter and nothing else,
  and a hook at the builder's `RTL` (`01:F7F1`) that finds the mark knows the battle it builds is random. The scaling with offsets
  does that (`free-progression.md`, "The offsets"); the cheats' own mark (`RANDOM_ONLY`) is the same idea, kept in the cheat block and
  set only while a battle cheat is on.
- **Enemy power (HP and BP)**: at the return of 01:F719 (01:F7F1) multiply, for every slot with flag `$40`, the HP word `+03`
  and the maximum `+0A` (saturate at `$FFFF`) and the BP `+07`..`+09` (saturate at `$FFFFFF`; the largest row BP,
  10 000 000, times 5 would overflow 24 bits). It runs at every battle start and at every form change, which is wanted. Seen:
  x2 turned the Saibaman's HP 55 to 110 and BP 326 to 652, and the first hit on it from 19 into 9.
- **Enemies per battle**: at the end of 01:F692 (before 01:B8DE, or call 01:B8DE again): take the group's units, repeat or
  drop them to the wanted count (1 to 5) and write them into the slots of that count's formation, set `enemy_count`. Seen
  (two Saibamen from a group of one): slot 0 emptied, slots 1 and 2 given flag `$40`, id 4 and `+1C` = 4, `$0EA0` = 2 at the
  `JSL 01:B8DE` of 01:F6D7: the battle shows two Saibamen with 55 HP each. Apply it to random encounters only: story groups
  (bosses, the scripted fights) must keep their units. A random encounter picks its group at 02:D34A, which runs only on the
  normal-battle path (below), so a flag set there tells 01:F692 which battles may change.

## Enemy scaling: what the battle code keeps absolute (found for the mod `enemy_scaling`)

The hooks of `enemy_scaling` (`docs/mods/free-progression.md`) sit at the `RTL` of the record builder (`01:F7F1`) and at the `RTL`
of the EXP award (`1D:8C66`). What the code around them showed, read in the listings and run on the replays named:

- **The EXP award** (`1D:8C33`) takes the defeated slot from `battle_enemy_offset` (`$14C5`, divided by 32), the unit of that
  slot from the byte table at `$7E:17C1` (one byte a slot), and adds the 24 bits at byte 11 of that unit's row (14 times the unit)
  to `battle_exp_award`. `X` still holds the row's offset at the `RTL`.
- **A fighter who is knocked out keeps bit 7 of his flags.** The KO routine (`05:A9AA`) clears bit 6 of the side record's flags
  (`$C0` becomes `$80`) and the copy back puts them in the party record: `secret-ssj-frieza` shows `$0B20`, `$0B40` and
  `$0B60` going from `$C0` to `$80` as Piccolo, Krillin and Vegeta fall. The end-of-battle EXP loop tests bit 6, so a fighter who is down is
  paid nothing, while the party list (bit 7 or bit 6) is unchanged for the whole battle.
- **Four enemies withdraw at a floor of HP** (`05:A4F5`, called from the hit step `05:A435` for a side-B target that is still alive):
  Frieza's first, second and third forms (units 53, 54 and 55) at 300, 20 and 50, and Zarbon's first form (unit 30) at 30. When the
  HP after the hit is at or below the floor it becomes the floor, the record's flags are cleared and the EXP award is called
  (`05:A528`, as the KO routine does), which ends the fight with the enemy gone and its reward paid. A floor above the unit's
  maximum HP is therefore harmless (the first hit that leaves the enemy alive ends the fight).
- **The second form's fight** (group `$54`) is judged at its end by an absolute HP (`05:9802`): below 21 the story goes on
  (game mode 0), otherwise the generic checks follow.
- **The Ginyu duel** (group `$51`) keeps its enemy at 1 HP when a hit would kill him, until `$15F1` is 2 (`05:A48C`).
- **The special attack's chance** (`05:BA8B`) is `10 + the attacker's level + its +0E byte - the card (a Z is 10) - the defender's
  level`, capped at 24, against the random byte masked with `$1F` (`$0F` for two animations), so the levels of the two records
  matter and a record's level is a number the battle reads. The same `+02` is read for the choice of an attack animation
  (`05:AF9E`: `level - 1`, capped at 47, over 6, with the card value) and compared with 47 (48 for Gohan) in `05:D12A`.
- **The game raises Vegeta to level 17 when it builds his record** (`00:8A46`): on a crafted save with Gohan, Krillin and Vegeta
  at level 1, the scaling read the mean 6.33 (1, 1 and 17) at the first battle, as the HP it gave the first form of Frieza shows.

## Random encounters

The map's per-frame code (00:976A, every frame) calls **02:D1F6**; the zone logic runs from 03:F163 every 64 frames
(`$4F & $3F` = 0, 12 309 calls in the reference traces against 214 for the zone code). Variables: `encounter_timer` (`$AA`),
`encounter_level` (`$0D8A`), `encounter_zone` (`$0D8B`), `battle_counter` (`$01B0`, `$01B1`), `map_id` (`$01A0`).

- **Zone and level** (02:D37D, 02:D38B): the leader's position is looked up in a list of rectangles per map
  (`encounter_zone_pointers`, pointers at 02:D4A4, rows of x range, y range and a zone number); the zone number goes to
  `$0D8B`, and `$0D8A` becomes the byte of `encounter_zone_levels` (02:D62A, four bytes a map, indexed by map x 4 + zone) plus,
  when the byte is not 0, 1 when flying (`$01BC` bit 0) and 1 more then if `$01BC` bit 6 is set. The byte is 0 (no encounters) or 1 in the ROM. With `$0D8B` = 0
  or `$0D8A` = 0 there are no encounters.
- **The roll** (02:D21D to 02:D23D): only on frames where `$51` bit 0 is clear, and while walking (a d-pad bit of `$44` held) or
  flying (`$01BC` bit 0). `$AA` counts down one a frame (it is reloaded with `$3B` = 59 at 02:D221 when it goes negative: a roll
  every 60 counts, 120 frames in the reference flight); at the roll `r` = (`$51` & 15) + 1 and the battle starts when
  `$0D8A >= r`, else `$0D8A` grows by 1 (the chance of a battle rises roll after roll until the zone logic resets it).
- **Starting the battle**: 02:D241 calls **02:D289**. It returns carry *set* when a story battle was started (maps `$16` and
  `$18`, 22 and 24, below) and carry *clear* for a normal one; then 02:D245 branches on the carry. For a normal battle
  02:D247 adds 1 to `$01B0`, 02:D34A picks the group (the map's pointer at 02:D6EA (`encounter_group_pointers`), rows from 02:D74A: eight
  group numbers per zone, one chosen by `$51` & 7), the screen transition starts and a few more bytes are set.
- **Story battles ride the same roll.** On map 22 (`$16`) with bit 0 of `$0183` set, the roll that comes with `$01B0` of 5 or
  more starts Frieza's first form (group `$53`, scene `$42`) and clears the bit; on map 24 (`$18`), with story flag 0 set and
  flag 8 clear, `$01B0` of 3 or more (a coin flip at 3 and 4, `$4F` bit 0) gives scenes `$3D` (group `$59`) and then, at the
  fourth, `$3F`. `$01B0` counts the *normal* battles of the map, so a map-22 or map-24 chapter cannot advance without
  normal encounters being counted.
- **Random battles off, without touching story battles.** Wrap 02:D289: after it returns with carry *clear*, set carry (the
  caller then returns at 02:D284 and nothing starts); on maps `$16` and `$18` also add 1 to `$01B0`, as 02:D247 would have,
  so the counter the story reads still advances. Seen: on the `landing` replay the roll at frame 5 430 (`$0D8A` = 6, carry clear,
  group 3) is suppressed and the flight goes on with no battle; on `secret-frieza-forced-battle` (map 22, counter 5) and
  `secret-frieza-random-encounter` (map 24, counter 3) the roll arrives with carry set and Frieza's group starts in the very
  same frame as without the patch (frames 2 484 and 2 724; the screens match the unpatched run frame for frame).
- **Encounter rate x1/4 to x4.** The rate is the roll interval. Faster: after the reload of `$AA` cap it (`$AA` = min(`$AA`,
  59 / m)). Seen: capping at 29 doubled the rolls (every 60 frames instead of 120: 5 070, 5 130, 5 190 against 5 070, 5 190,
  5 310), capping at 14 gave one every 30. Slower than 1: the cap cannot be used (the countdown is a signed byte: a value
  of 128 or more is already negative); skip the roll instead, i.e. in the wrapper of 02:D289 veto a normal battle with
  probability 1 - m (the veto is the one proved above; the probability is the wrapper's own random number, not the game's).
- **Chain battles** (code): after a normal battle started in the air (`$01BC` bits 0 and 6 set) 02:D262 to 02:D281 may queue up to
  two more in `$01BF` (bit 7 plus a count drawn from a table at 02:D49C); while it is queued 02:D3EF clears `$AA` and sets
  `$0D8A` to 15, so the next roll fires at once. A veto at 02:D289 therefore also stops the chain. Not decoded: `$01BE`, and which
  byte the Bubbles object ("no random battles for a while", `menus.md`) sets.

## Using an object: where it leaves the list

An object used from the Item screen, on the map or in a battle, is removed by its effect routine's last step: it zeroes the
entry (`$0100,X` and `$0101,X`, `X` the entry's offset) and calls **01:C639**, which compacts the list over every entry whose
attribute byte is 0 and subtracts 1 from `item_count`. 01:C639 has 13 callers; the ones that are item effects are, by the return
address (`JSL` return pushed on the stack): **01:DDF3** (the common tail: healing objects, the stat objects, Scouter, Bubbles,
Yajirobe, Korin, Shenron, the card objects), **01:EB33** (Vegeta), **01:EBB3** (Escape), 01:E35E (the end of Porunga's wishes),
01:DD8A. The others are not uses: 01:8E4F (Drop), 03:A899 and 03:A8C8 (the shop), 01:D844 and 01:DBD1 (the story removes an object
by id), 01:D9FD, 01:C6AB and 1D:8AB9 (the battle's helper that takes back the last entry).

Seen with `ret` of the call stack at 01:C639 on crafted saves (the object first in the list, a battle with Raditz or the first
map): ids 2 Bulma, 5, 6, 7, 9, 10, 14, 16 (Scouter), 18 (Yajirobe), 21 (Bubbles), 25 and 8 (Korin) return to 01:DDF3; 19 (Vegeta)
to 01:EB33; 22 (Escape) to 01:EBB3. Ids 15 (Porunga), 20, 23 (Dragon Radar) and 24 were not consumed in the window run (the
effect or the "can't use" message was still going) and 01:DD8A, 01:E35E were not reached.

**Endless items**: wrap 01:C639 and, when the return address is one of the effect tails above, put the entry back instead of
compacting: write the object id (`$0D7C`, `menu_item_id`) and its attribute byte (`item_attributes[id]`, bit 0 clear) at
`$0100,X`/`$0101,X` (`X` is still the entry's offset at the entry of 01:C639) and return without the `DEC item_count`. Seen
(Bulma in a battle, hook at the 01:C639 entry): Goku healed 50 to 75, `item_count` stayed 7 and the battle's command menu came
back. The story's removals (Drop, shop, 01:DBD1) stay original.

## How to see it again

These were made with a throwaway patch of `xtask shots` (not committed; see the plan's task 0): a list of *patch points* ("at
bank:address, once, write or scale these WRAM bytes, optionally set the carry or return from the routine") applied while a
replay plays one CPU cycle at a time. Without it the same facts are seen with what exists:

    xtask disasm 05:B7FC                       # the damage routine (use --m8 --x16)
    xtask shots battle --frames 5119 --peek 0x7E14CF,0x7E14CD,0x7E14C2,0x7E1400
    xtask shots battle --upto 5600 --watch 0x7E1443,0x7E14CF --watch-from 5000   # which pc writes the enemy HP and the damage
    xtask shots secret-raditz.ssdreplay --upto 5200 --watch 0x7E1403,0x7E14CF --watch-from 3300

`--watch ... --watch-from` prints the program counter after each write: 05:B9B5 stores the damage, 05:BE0A subtracts it from
the HP, 1D:8C5A adds the EXP, 05:8FD2 adds it to a record.

## The cheat block, the battle wrappers and the scripted-loss gate (M5, task 1)

**The block.** The cheats keep their settings in 32 bytes of work RAM at `$7F:FF00` (`ssd_game::cheats::BLOCK`). Bytes
`+00` to `+09` are a num/den pair per battle setting, in this order: damage dealt, defence, EXP, enemy power, enemies
per battle (a multiplier of `num / den`, so x1/2 is `1, 2`). `+0A` is the scripted-loss gate and `+0B` the
random-encounter mark that `02:D34A` sets and `01:F692` clears. `+0C` to `+0F` are the marker `SSDc`: the power-on
fill of work RAM is random, and the wrappers trust a block only with the marker (a block without it is neutral).

**Where it lives.** The block was chosen from a scan of the 28 reference replays (every CPU write, and every change of
work RAM at a frame boundary, WMDATA included). `$7F:F001` to `$7F:FFFF` is never written. The first choice,
`$7E:1F00`, is cleared by the game as it boots, so a write before boot is lost. It is written at a frame after boot
(frame 300 of a replay) and survives the Continue of a save (checked on `secret-nail-porunga`) and the battle. It lies
outside the save image (`$7E:2800` onwards).

**The wrappers.** Each battle wrapper runs the original code it wraps (`Ctx::call_original`) and changes only what its
cheat asks for, so a neutral block is the identity: a standalone battle with the table installed and a neutral block
gives the original's first hits (19, 15 and 6) on the battle replay. A wrapper is gated (`mark_gated`) with no cost of
its own. On the battle replay the wrappers ran seven times with the cheats on, and none was sent back to the original
by a timing event (the window margin is 8 master cycles at a zero cost).

**Measured** on the battle replay (the Saibaman fight, from power-on): the first hit 19, the EXP award 9, the enemy HP
55 and BP 326, one enemy. With damage x2 the hit is 38 and the enemy's HP falls to 17. With EXP x3 the award is 27.
With enemy power x2 the HP is 110 and the BP 652. With enemies x2 the battle has two enemies.

**The gate's states.** The mode byte `$24` of a battle is not only 2 and 4: a battle passes through `$82`, `$84`
and `$8C` (the flag bit `$80` set) and `$0C` (one frame between the two phases of the Nail fight, at frame 5285 of
`secret-nail-porunga`), and it returns to the map (`$00`) when it is over (frame 6556). The gate therefore goes armed
(set by the group pick `01:F692` for a scripted group), in battle (the first battle state), and off on the map. An
earlier gate that came off on the first non-battle frame came off in the middle of the Nail fight (frame 4104, a
state `$8C`), and the cheats then applied to the rest of it.

**Group spike** (`cargo run --release -p xtask -- cheat-spike`): every `secret-*` replay played in Classic and with the
extreme battle cheats (damage x10, defence x10, EXP x10, enemy power x5, enemies x3), from power-on, with the gate on
the scripted groups.

**Verdicts** (the spike above, the scripted groups of the plan, `secret-*` replays, from power-on; "finished" is a
battle that leaves the battle states before the replay ends):

| Group | Scripted fight | Replays | Classic | Extreme cheats | Verdict |
|---|---|---|---|---|---|
| `$57` | Nail alone against Frieza | `secret-nail-porunga` | finished, frames 3755 to 6556, map 11 to the next map | gate on: the same battle, the same frames, the same end | **in SCRIPTED_LOSS** (the loss is the story) |
| `$05` | Raditz | `secret-raditz` | not finished in the replay | not finished in the replay | no break shown; cheats apply (unproven: the fight runs past the replay) |
| `$38` (`$39`) | Zarbon | `secret-zarbon-vegeta-joins` | not finished in the replay | not finished in the replay | no break shown; cheats apply (`$39` is played by no replay) |
| `$51` | Ginyu | `secret-ginyu-goku-duel` | not finished in the replay | not finished in the replay (map 16 in both) | no break shown; cheats apply (unproven) |
| `$56` | final Frieza | `secret-ending-normal`, `-super-vegeta`, `secret-ssj-frieza` | the ending (map 24, mode `$11`) at 4575 and 5005 | the ending in both, at 5613 and 5077 | no break shown; cheats apply |
| `$53` | Frieza's first form (not on the plan's list) | `secret-piccolo-returns` | finished at 7659 | finished at 8349 (the replay ends in mode 3 in Classic, in mode 8 with the cheats) | no break; cheats apply (explained: the recording's presses are out of step, not the story; see below) |

The `$57` row is the one that needed the gate: the first gate, which came off on the first non-battle state, came
off in the middle of the fight (frame 4104), and the cheats then changed the fight (see above). The gate now stays
through every battle state and comes off on the map.

The `$53` row was a false alarm, found by reading the "end mode" column right. It is the mode byte at the *end of the
replay*, not the mode a battle leaves: in both runs the battle ends the same way (`$82` fade, then mode 8, the
victory state, then the map, a fade, and mode 3 for the scene). The recording's presses were made for the Classic fight,
so with the cheats the fight is won at another frame (damage x10 and enemy power x5 together: 8349 against 7659), and
the recording ends in the middle of the post-battle script (mode 8) instead of at its scene (mode 3, 690 frames
later in the cheat run). Played on with the same presses shifted by the 690 frames (`cheat-spike` prints this as
"continued from the cheat run's end of the fight"), the extreme cheats reach the same mode sequence, the same map and the same party as
Classic (Piccolo, Tien, Yamcha, Chiaotzu): the story does not expect a loss and the cheats do not break it, so the
group is not gated. Single cheats on the same recording: damage x10 (ends 7194), defence x10 (7656), EXP x10 (7659) and the
Easy preset (5176) all continue to the same state; enemy power x2 (Hard) and x5 end in mode 13 (not the victory path; taken to be a lost fight), because the
recorded presses do not win against stronger enemies; a player who wins the fight gets the same continuation.

### Identity verdicts of three secret replays (found 2026-10-15)

`verify-replays --cheats-identity` reported DIVERGED on `secret-king-kai-planet`, `secret-piccolo-snake-way` and
`secret-dodoria`. None of it was the cheats: the same three diverged without `--cheats-identity` in the non-recomp
build (Rust hooks), and with all hooks on the original (`--hooks original`, with or without the wrappers) all three
were green.
- King Kai planet and Snake Way: `update_travel_and_follow` declared one cost for a call whose real cost is 6 master
  cycles lower when bit 1 of `$01E7` is set (see `docs/re/routines/update_travel_and_follow.md`). The hooked machine
  arrived 6 cycles late at the next event. Fixed in the routine's cost.
- Dodoria: the recording ends inside `fill_bank7f_pattern` (a 2-million-cycle call, about 6 frames, with the NMI off).
  A stops on the last recorded frame, still inside the original's run of it, and B, whose Rust version of the call is
  atomic, ends 5 frames later. That is the lockstep's tail window, not a routine difference; the last window of a
  replay now accepts B past the end (the NMIs are still compared).
Non-recomp runs of these replays end UNVERIFIED (routines called but never run in Rust), as for most replays.
