# Free progression: enemies that follow the party

In the original game an enemy has fixed stats: a party that trains outgrows it, and a party that does not cannot pass the
bosses. The feature `enemy_scaling` (a [named feature](features.md) of a data pack) makes the **enemies follow the party**:
each enemy of a battle is set to the strength it would have if it had been made for the party's level, and it pays the EXP of
that strength. A `difficulty` moves that level up or down. It is part of the "Free progression" mod; the other part, the story
that no longer waits for levels, is another page. Instead of a difficulty a pack may give **two offsets in levels**, one for
the random battles and one for the story and boss battles ([the offsets](#the-offsets)); the released pack does.

The feature is for the Enhanced mode only and is off unless a pack enables it (`examples/enemy-scaling` is a pack that does).
Classic is not changed. This page is the design with its numbers; `features.md` has the parameter.

## The rule

Write `u = level + 5` for a level. For a battle:

- `P` is the **party's level** (below), `L` the **enemy's own level** (below), and `d` the difficulty's factor: 7/8 for easy,
  1 for normal, 9/8 for hard;
- `q = d * (P + 5) / (L + 5)`;
- the enemy's **HP and KI are multiplied by `q`**, its **BP by `q^3`**, the **EXP it pays by `q^2`**, and its **level becomes
  the party's level with the difficulty**, `d * (P + 5) - 5`, rounded, at least 1 and at most 999 (the party's level cap, [the level cap](#the-level-cap); the record's own byte saturates at 99);
- every number is rounded to the nearest whole number (halves up), a number that is not 0 stays at least 1, and each is
  capped at the width it is kept in (HP and KI 16 bits, the EXP award 24 bits, BP 40 bits).

All of it is integer arithmetic, so that a replay plays the same on every machine. At `P = L` and the normal difficulty `q`
is exactly 1 and every number is the original's. `q` grows with `P` and with the difficulty, so every number does (a test
runs every party level from 1 to 100 at every difficulty, for six enemy levels, and requires it); there is no floor at the
original's numbers.

### The worked example

An enemy of level 20 with HP 300, KI 100, BP 20 000 and EXP 2 000 (our own numbers, not a row of the game), at the normal
difficulty, against a party whose level is:

| Party level `P` | Enemy's level | HP | KI | BP | EXP |
|---:|---:|---:|---:|---:|---:|
| 1 | 1 | 72 | 24 | 276 | 115 |
| 5 | 5 | 120 | 40 | 1 280 | 320 |
| 10 | 10 | 180 | 60 | 4 320 | 720 |
| **20** | **20** | **300** | **100** | **20 000** | **2 000** |
| 30 | 30 | 420 | 140 | 54 880 | 3 920 |
| 40 | 40 | 540 | 180 | 116 640 | 6 480 |
| 47 | 47 | 624 | 208 | 179 978 | 8 653 |

How far the same law moves an enemy made for another level (`q^3`, the BP factor, at the normal difficulty; HP and KI move by
`q`, the EXP by `q^2`):

| Enemy's level `L` | `P` = 1 | 5 | 10 | 20 | 30 | 40 | 47 |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 5 | 0.22 | **1** | 3.4 | 16 | 43 | 91 | 140 |
| 20 | 0.014 | 0.064 | 0.22 | **1** | 2.7 | 5.8 | 9.0 |
| 40 | 0.0024 | 0.011 | 0.037 | 0.17 | 0.47 | **1** | 1.5 |

## The offsets

A pack may set the target level by **offsets in levels** from the party's level instead of a difficulty (the two ways are
exclusive, `features.md`). Two kinds of battle, each with its own offset, `random_offset` and `boss_offset` (whole numbers, -10 to
+10):

- a **random battle** is one the map's roll starts in its normal branch, where the game picks the group of the zone;
- a **boss battle** is every other: the battles a script starts (the story's fights, the bosses, the Saiyans, Ginyu, Frieza), the
  story branches of the roll (Frieza's first form, the Ginyu map's fights) and the next form of a boss.

The target is `P + offset` where `P` is the party's level (the mean of the top three, in sixths), kept between level 1 and level
999, and everything else is the rule above with the target in place of `d * (P + 5) - 5`:
`q = (P + offset + 5) / (L + 5)`, HP and KI times `q`, BP times `q^3`, EXP times `q^2`, and the record's level is the target,
rounded. A target below 1 is 1 (and `q` is taken from that level, not from the lower number), so a party at level 3 with `-10`
meets enemies made for level 1. An offset of 0 is the normal difficulty, number for number.

| Party level `P` | random, `-3` | story and boss, `+3` | random, `-5` | story and boss, `+5` |
|---:|---:|---:|---:|---:|
| 1 | 1 | 4 | 1 | 6 |
| 10 | 7 | 13 | 5 | 15 |
| 30 | 27 | 33 | 25 | 35 |
| 46 | 43 | 49 | 41 | 51 |

For an enemy made for level `L`, against a party of level 46 (the BP of a level 8 Nappa row is 4 000 and of a level 10 Vegeta
row 18 000, HP 220 and 240; the numbers are read from the run described below):

| Enemy | Original | `difficulty = "normal"` (the target 46) | offsets (the target 49 for the two fights, 43 for the Namek random) |
|---|---|---|---|
| Nappa (group 20) | level 8, HP 220, BP 4 000 | level 46, HP 863, BP 241 513 | level 49, HP 914, BP 286 689 |
| Vegeta (group 21) | level 10, HP 240, BP 18 000 | level 46, HP 816, BP 707 472 | level 49, HP 864, BP 839 808 |
| Namek random (group 30, first enemy of the pair) | level 4, HP 70, BP 1 700 | level 46, HP 397, BP 309 337 | level 43, HP 373, BP 257 896 |

(These are the numbers before the cheats. The enemy power cheat, the Options difficulty and the damage and card cheats act after the
scaling, as always.)

**How the game tells the two apart.** The roll of the map (`02:D241`) calls `02:D289`, which says whether a story battle starts
(carry set: Frieza's first form on map 22, the scenes of map 24) or a normal one. A normal one
runs `02:D34A` to pick the group of the map's zone, and `02:D34A` has no other caller (`battle.md`). A script's battle sets the
group and the mode itself (the scene language's poke) and never runs it. The feature marks the pick, at its `RTL` (`02:D37C`), and
the builder of the enemy records (`01:F7F1`) takes the mark: a battle whose builder finds it is a random battle, any other is a
boss battle. The mark is taken at once, so a builder run again for the same battle (the form changes of Frieza, `00:961A` and
`05:9852`) or for the next battle finds none, and the kind is kept for the EXP award of the defeated enemies of the same battle
(`1D:8C66`). The mark and the kind are kept in six bytes of the mod state block (`state.md`, `$7F:FB50`), with their own marker.

## The party's level

`P` is the **mean of the levels of the three highest fighters** of the party, read from the nine party records when the
enemy records are built. It is kept in sixths of a level, which hold the mean of one, two or three levels exactly (the sum
times 6, 3 or 2), so that a party whose mean is the enemy's level gives `q = 1` exactly.

Why the mean of the top three, and not every fighter or the highest:

- the mean of every fighter lets a weak straggler (a new recruit, Dende) pull every enemy down, which the player can use;
- the highest alone makes one fighter's level the whole party's: a character who trained alone sets the difficulty for the
  other four;
- the top three follows the fighters that carry the fight, ignores the stragglers, and with fewer than three it is the mean of
  those there are.

A fighter is a record **in the party** (the flags have bit 7 or bit 6: a fighter who is knocked out keeps bit 7 and still
counts, so the level does not change in the middle of a fight, and the EXP of an enemy defeated late is scaled by the party the
enemy was built for) **with a level**. The characters whose level is not the party's progress are left out unless they are all
there is: Nail, Tsumuri and Maima (8 to 10, whose stats are their unit's), Dende (11, who stops at level 3), character 12 (which
the level-up check leaves out as well) and the temporary record the story gives the borrowed Vegeta (unit `$4A`). A party made
of those alone is the mean of what they are.

## The enemy's own level

`L` is the level of the enemy's record when the feature runs: the level the unit's row gives it (`units.md`, byte 1), or a
pack unit's row. It is the level the enemy was **made for**, and the rest of the rule takes it as the party level at which
the enemy is as the original has it.

The encounter zone table (`02:D62A`) is not a source of levels: it is 48 maps of four bytes, every byte is 0 or 1, and
`battle.md` has what it does (a zone's chance of a battle, with 0 for none), so the game has no "level of the map". What it
does have is the unit rows, and their levels agree with the levels the story asks of the party where the code shows both
(`kaioken-bp.md`, B.1): the first fight with Raditz (a row of level 5) is forced when Goku reaches 5 or Piccolo 4; the Saiyans
of the Earth chapter (rows of levels 8 and 10) come at the deadline of Piccolo's level 9; Goku's duel with the last of the Ginyu
squad (a row of level 36) is set up by his level of 36 or more (the flags of map 16 step at 34 and 36); the last form of Frieza
(a row of level 48) is fought by the Goku of level 47 who becomes a Super Saiyan.

## Why this law

The law is the one the game's own tables follow, fitted by shifted power laws in `u = level + 5` (the fits are of the game's
level rows and unit rows, read from the cached ROM; nothing of them is stored here):

| Number | Party (Goku's level rows, levels 1 to 47) | The enemies' rows (levels up to 36) | The law |
|---|---|---|---|
| BP | `u^2.8` (shift 4) or `u^3` (shift 5); a typical error of 21 percent | `u^3.2`, a wide scatter | `u^3` |
| HP | `u^1.0` (an error of 4 percent) | `u^0.9` to `u^1.25` | `u` |
| KI | `u^1.0` (10 percent) | `u^1.2` to `u^1.35` | `u` |
| the EXP the next level needs | about `u^2`, a wide scatter (the table is uneven) | the EXP an enemy pays, `u^2.4` | `u^2` |

So the enemy that follows the law keeps its **place on the party's own growth curve**: for the six enemies tried (rows of
levels 5 to 40) and every party level from 5 to 47, the BP of the party's strongest fighter over the BP of the scaled enemy
stays between 0.6 and 1.8 times what it is at the enemy's own level (a factor of 3 in all), where the same enemy unscaled
drifts by a factor of 240. HP follows the party's HP and the EXP follows what a level costs, so an enemy of a given kind is
worth about the same share of a level at every level.

The BP is the stat that matters: the damage of a hit is `BPa * (va + 4) * K / (BPd * ...)` (`battle.md`), the ratio of the
two BPs, and the fight's length is the HP over the damage. That is why BP follows the party's cube and HP only its first power.

**The defence factor is not scaled.** It multiplies the defender's BP in the same formula (`D = BPd * (...) * F / 10`), so it
is a constant ratio of the exchange that the BP's scaling already moves with the enemy; scaling it too would count the same
level twice.

**The enemy's level field is set** because the game reads it: the special attack's chance is
`10 + attacker's level + the +0E byte - the card - defender's level` against a random number (`05:BA8B`). With the original level
the party's special attacks would fire on three hits in four (the chance is capped at 24 of 32) as the party outgrows an enemy,
and the enemy's never, and the reverse for a weak party, so the scaling would be undone by the double damage. With the enemy's
level the party's, the chance depends only on the difficulty and the cards, as it does in the original when the two levels are
equal. The game also reads the acting fighter's level to pick a band of attack animations (`05:AF9E`: `level - 1`, clamped at
47, over 6), so a scaled enemy is drawn as an enemy of that level.

## Rewards

- **EXP** is scaled by `q^2` where the original adds it (`1D:8C33`, one defeated enemy, or one that withdraws at its floor of
  HP, below), from the unit's own row. A stronger enemy pays more, and a weaker one less, and the EXP cheat multiplies the
  scaled award afterwards.
- **Zeni**: a battle pays none (`battle.md`: the story's events and the shops are the only writers), so there is nothing to
  scale.
- **Drops** are the original's.

## Bosses and scripted fights

Every battle is scaled, the bosses and the story's fights with the rest (with the offsets, at their own offset, `boss_offset`, and the
random battles at `random_offset`), so that the story stays passable at any level: a
party below a boss's level meets a boss weakened in the proportions of the table above, and a party above it a boss
strengthened in them. **There is no floor at the original's numbers**, because that floor is exactly what keeps a party that
did not train from passing a boss: the feature exists to remove it. (The floor that remains is 1: a number that is not 0
never becomes 0.)

The forms of one boss are separate battles with separate rows (the four forms of Frieza are rows of levels 40, 42, 45 and 48),
each scaled by its own level against the same party, so they keep their order and come closer: the last form is 5.7 times the
first in the original and 3.5 times at any party level.

The fights the party **must lose** stay as the original has them: the battles of `cheats::SCRIPTED_LOSS` (today the group
`$57`, Nail against Frieza) are not scaled, neither their records nor their EXP, exactly as the battle cheats are suspended for
them. For any other fight the enemy is made for the party, so a fight the story expects the party to lose to the boss's strength
(the last Frieza, before Goku's transformation) is lost in the same proportions at every level.

**The numbers the story keeps.** A few fights have absolute HP numbers in the original's code. After a hit that leaves him
alive, Zarbon's first form (unit 30) and the first three forms of Frieza (units 53, 54 and 55) withdraw from the fight when their
HP is at or below 30, 300, 20 and 50 (`05:A4F5`: the HP becomes that number and the fighter leaves); the second form's fight ends
the story's way when his HP is below 21 at the end (`05:9802`); the Ginyu duel keeps its enemy at 1 HP until its scene
(`05:A48C`). With the scaling these numbers are **scaled with the enemy**: each is the original's number times the `q` that scaled
that enemy's HP (at least 1), so that it keeps its place in his HP (30 of Zarbon's 232 is 13 percent at any level), and a
scaled enemy is **never killed by a hit that crosses its floor** ([the HP floors](#the-hp-floors)). Run on the first fight with
Frieza with the party at level 1 the story went on to the same scene as the original (`secret-piccolo-returns`).

The transformations are not scaled: the Super Saiyan Goku has the fixed BP the transformation assigns (4 999 999), so a party
that is far below the last Frieza's level meets a Frieza weakened by the law and a Goku who is as strong as ever, and a
party far above it (with the level cap raised) a Frieza the Super Saiyan may not beat. With `level_cap` the form follows Goku's
level instead ([the level cap](#the-level-cap), "Bosses at high levels").

## The level cap

The parameter `level_cap` (48 to 999, `features.md`) lifts the party's level cap: the game stops a character at 47 or 48 (its cap
table), and with `level_cap = 999` the party goes on to level 999 on the curves the game's own rows follow. The released pack
(0.4.0) gives 999. A pack without the parameter keeps the game's caps, and is read exactly as before. It is for the characters
that have a level table (Goku, Gohan, Piccolo, Krillin, Tien, Yamcha, Chiaotzu and Vegeta); Dende stops at level 3 and Nail,
Tsumuri and Maima have no levels.

### Storage

- **The level.** A record keeps its level in one byte, which every routine of the game reads. A level above 99 is kept as a
  **wide level** of 16 bits in the mod state (`state.md`, `$7F:FB60`), one per character, with its own marker. The record's byte then
  **saturates at 99**: the original's screens show two digits, so they read "99", and no 8-bit sum of the special attack's roll
  (`10 + level + byte`) can wrap. The wide level is the level only while the byte agrees with it (`min(wide, 99) = byte`), so a
  routine that wrote the byte by itself leaves the byte as the truth.
- **The EXP** is 24 bits in the record. Level 999 needs 63 million, so bits 24 to 31 are in the same state, one byte per character;
  the 24-bit add of a fight's award puts its carry there (`05:8FDD`).
- **The BP** is the wide BP of `state.md` (24 bits in the record, 16 in the block): a party BP at level 999 is 1.9 billion.
- **HP and KI** fit their 16 bits (the largest value would be 14 384 HP at 999), but the original's windows and the HD screens carry
  four digits, so **the party's maximum HP and KI are cut at 9 999**. Goku's HP reaches the cut at level 697.
- **Byte `$0E`** of the record (the battle's tie-break, read in the special attack's 8-bit sum) is cut at 100.
- **The enemies'** level is made for the party's wide level; the record's byte saturates at 99 as the party's does, so the special
  attack's chance (`10 + the attacker's level + ... - the defender's level`) is the original's difference of two equal levels.

### The extension

The research fits the game's rows by shifted power laws in `u = level + 5` (BP `u^3`, KI and HP `u`, the EXP a level needs `u^2`,
the table above). Past the last level the character reaches in the original (its cap, `L0`: 47 for Goku, Piccolo and Vegeta, 48
for the others) the law goes on **from the character's own last values**, so the curve is continuous where the table ends:

| Quantity | At level `L > L0` |
|---|---|
| the BP built up (the sum of the rows) | `BP(L0) * (u / u0)^3`; the BP a level gives is the difference of two totals |
| the maximum KI built up | `KI(L0) * (u / u0)`, at most 9 999 |
| the maximum HP | `HP(L0) * (u / u0)`, at most 9 999 |
| the EXP the level begins at | `EXP(L0) * (u / u0)^2` |
| byte `$0E` | `b(L0) * (u / u0)`, at most 100 |

with `u = L + 5`, `u0 = L0 + 5`, `BP(L0)` and `KI(L0)` the sums of the rows up to `L0`, and the rest the row of `L0`. All integer
arithmetic, rounded to the nearest (halves up): `ssd_menus::levels`, shared by the game's hooks and the save editor. Goku
(`BP(47)` = 260 090, KI 138, HP 745, EXP 170 000, byte 8):

| Level | BP | max HP | max KI | EXP the level begins at |
|---:|---:|---:|---:|---:|
| 47 (the game's cap) | 260 090 | 745 | 138 | 170 000 |
| 48 | 275 386 | 759 | 141 | 176 601 |
| 100 | 2 141 320 | 1 504 | 279 | 693 140 |
| 300 | 52 482 334 | 4 370 | 809 | 5 848 465 |
| 999 | 1 872 038 440 | 9 999 (14 384 uncut) | 2 664 | 63 373 787 |

The game's own rows are used up to its cap (so levels up to 47 or 48 are the original's, byte for byte: a test runs the level-up
check and a floor on the original and on the pack and compares the record). For a character whose cap is 47 the row of 48 is not
a level to reach in the original (it is the Super Saiyan's), and the extension begins at 48 from level 47's values.

### Where it acts

| Key | What the hook does |
|---|---|
| `03:841E` (entry) | the level-up check after a fight, replaced for a character with a table: while the EXP reaches the next level's start and the level is below the cap, the level goes up and the row's BP, HP, KI and byte `$0E` are applied, HP and KI refilled, bit 7 of `$1F` set; the original's own rules otherwise |
| `03:851B` (entry) | the raise by a number of levels (the story's floors and gifts, the Porunga card), with the original's rule: the target plus one, at least one level; the byte saturates, so a floor's target below 99 is one level and a gift of `n` levels is `n + 1` |
| `05:8FDD` | the carry of a fight's 24-bit EXP add goes to the EXP's high byte |
| `03:867A` | the end of the Super Saiyan's builder, see "Bosses" below |

They decline without the parameter and for a character without a table. A pack that also redirects the level tables
(`tables.md`) gives way to the level cap for those eight characters.

### Bosses at high levels

The enemy scaling is unchanged except for its two limits: the target level `T` is clamped at **999** (it was 200), and the party's
level `P` is read from the wide levels. The forms of one boss and the HP thresholds work as the section above has them. The decisions:

- **The Super Saiyan Goku** is built with the fixed BP of the row of level 48 (4 999 999), HP 800 and KI 189. At a party far above
  47 that Goku would be a fraction of the last Frieza, who has scaled with the party. With the level cap the form **follows Goku's
  level `L`**: BP times `((L + 5) / 52)^3`, HP and KI times `(L + 5) / 52` (at most 9 999), so that the form's place on the curve is
  the one it has at level 47, and the record keeps the level byte it had. A Goku of 47 or less is the original's Goku, exactly.
  At level 100 the form has 41 million BP, at 300 one billion.
- **The transformation's gate** reads a level of 47 or more: with a wide level it is met as ever, and a party below 47 still gets
  the transformation (the first part of the mod).
- **The scripted loss** (Nail against Frieza, the group `$57`) is not scaled, as before.
- **The HP-threshold fights** (Zarbon's first form, the first three forms of Frieza, the Ginyu duel) keep their floors at the same
  place in the boss's HP: the thresholds (30, 300, 20, 50 HP; 1 HP) are scaled by the `q` of the enemy, and a hit that would take
  the boss from above its floor to 0 leaves it at the scaled floor, where the story takes it away ([the HP floors](#the-hp-floors)).
- **The EXP of the fights.** The EXP an enemy pays is scaled by `q^2` and the battle's total is 24 bits (16.7 million): a party at
  level 999 against enemies made for level 5 earns about a million an enemy, so a total that wraps needs a dozen of the strongest
  bosses in one fight, which no battle has.

### The displays

The original's screens (the field menu, the Status page, the battle's level-up message) show the record's byte as two digits: they
read "99" from level 99 on, and the BP they show is the low 24 bits of the wide BP. The Enhanced screens read the real values:
the HD Status screen (the level, the whole BP, the EXP with its high byte, and the EXP to the next level on the extended curve up to
the cap) and the Save screen's leader level read them from the mod state (`mods::state::level_in`, the same rule as the game's
reader), and the save editor reads and sets them: with a pack whose `level_cap` is set (read from the image the game runs) it takes a
character with a level table (Goku to Vegeta) up to the cap on the curves above (`ssd_menus::levels`, through `Growth::stats_at`), with the
BP as the curve plus the member's delta, and writes the wide level, the EXP's high byte and the BP's high part into the slot's entry of
`ssd.mods`. Without such a pack the editor is the game's own. The Order, Cards and Items screens show no level or BP.
## Difficulty

`difficulty` is the older way to set the target (the offsets above are the newer, and the released pack uses them since its version
0.3.0). A pack written with `difficulty` alone is read exactly as it was: the same bytes in the image, the same numbers, no mark
written, one factor for every battle. It is one of three names, each a factor `d` on the party's `u = P + 5`:

| `difficulty` | `d` | HP and KI | BP | EXP | The enemy's level is |
|---|---:|---:|---:|---:|---|
| `easy` | 7/8 | x0.875 | x0.67 | x0.77 | about 3 levels below the party's at level 20 |
| `normal` | 1 | x1 | x1 | x1 | the party's |
| `hard` | 9/8 | x1.125 | x1.42 | x1.27 | about 3 levels above the party's at level 20 |

It is a factor and not a number of levels so that it is the same at every level: three levels are 40 percent of the BP at
level 20 but 120 percent at level 5 and 18 percent at level 47 (the game's growth per level falls and rises), while the factor
is 1.42 everywhere. At the party's level 20 an enemy made for level 20 is, in the worked example, at level 17 / 20 / 23 for
easy / normal / hard, with BP 13 398 / 20 000 / 28 477.

## The cheats and the Options difficulty

The scaling **comes first and the cheats multiply it**: the enemy power cheat and the Easy and Hard presets of the Options
screen act on the records after the feature has written them, and the EXP cheat on the scaled award. They are a choice of the
player's, so a visible setting is never dead, and they combine as numbers do (hard in the pack and the Hard preset is both).
Because an enemy that scales past 24 bits of BP keeps the high 16 bits in the mod state block (below), the enemy power cheat was
made to multiply that wide BP whole when the block is live.

## Where it acts

| Key | The routine | What the hook does | Mode and resume |
|---|---|---|---|
| `01:F7F1` | the `RTL` of `01:F719`, the builder of the five enemy records | scales every record the builder filled (level, HP and maximum HP, KI and maximum KI, BP) | `Patch`; `resume_at(01:F7F1)` |
| `1D:8C66` | the `RTL` of `1D:8C33`, the EXP of one defeated enemy | replaces the unit row's EXP the original added to the battle's EXP by the scaled one | `Patch`; `resume_at(1D:8C66)` |
| `02:D37C` | the `RTL` of `02:D34A`, the pick of a random encounter's group | marks the battle as random (only for a pack with offsets) | `Patch`; `resume_at(02:D37C)` |

Both are reached with M8 X16 and read and write work RAM by bus address, so they depend on neither the direct page nor the
data bank. Both decline at once (they only resume) without the feature in the image, in a battle of the scripted-loss list,
and without a fighter in the party.

`01:F7F1` is after the data tables' redirector of `01:F7DF` (a pack's unit row is already in its record) and after the original
has drawn its small correction of each BP, and it is **one stop for all five slots**, which the per-slot tail `01:F7DF` (owned by
that redirector) is not. It mirrors the builder's loop (an odd enemy count starts at slot 0, an even one at slot 1) and leaves a
slot with no flags, or with a level of 0 (the row of a party member used as an enemy), as it is. The level the scaling takes as
the enemy's own is the record's, so a pack's unit is scaled from its own row. The EXP hook takes the level from the unit's row
of the original table, so a pack's unit (for which the game reads no row) pays what it paid.

Both are `Patch`es and not `Rust`-mode wrappers like the cheats': a `Rust` wrapper at the entry of a routine is needed only to act
**after** the original (`call_original`), and these keys are inside the original's own run, at its last instruction, which
they let run.

Four more `Patch`es, inside the hit step `05:A435` and the end of a fight, keep the HP-threshold fights ([the HP floors](#the-hp-floors)):

| Key | The instruction | What the hook does | Mode and resume |
|---|---|---|---|
| `05:A46E` | the `BCC` after `05:BDDC` (the hit's HP subtraction); carry clear = the HP went below zero | for an enemy of the four floors, a hit that took the HP to 0 or below sets the HP to the scaled floor and the carry | `Patch` (M8, X16); `resume_at(05:A46E)` |
| `05:A51D` | the `CPY $1443` of `05:A4F5` (`Y` = the original's floor) | `Y` becomes the scaled floor, which the compare and the store of the HP use | `Patch` (M8, X16); `resume_at(05:A51D)` |
| `05:A49F` | the store of the 1 HP the Ginyu duel's enemy is kept at (`A` = 1) | `A` becomes the scaled 1, until the swap (`$15F1` = 2) | `Patch` (M16, X16); `resume_at(05:A49F)` |
| `05:9808` | the `BCC` after the `CPY #$15` of `05:9802` (the second form's fight, group `$54`, judged by the HP of its enemy) | the carry is set from the scaled floor (HP above it) instead of 21 | `Patch` (M8, X16); `resume_at(05:9808)` |

### The HP floors

The original lets four enemies withdraw when a hit leaves them at or below a floor (`05:A4F5`: Zarbon's first form 30 HP, Frieza's
first, second and third forms 300, 20 and 50): the HP becomes the floor, the record's flags are cleared and the reward is paid,
and the story goes on with the next scene or form. The floor is judged only for an enemy **still alive after the hit**: a hit that
takes the HP to 0 or below goes to the knock-out (`05:A9AA`) with no floor, and the fight ends as a victory. The original does not
clamp the HP at the floor (only the Ginyu duel's enemy is kept alive, at 1 HP, until the swap). With HP of the original's size a
hit seldom crosses a floor in one blow; a scaled enemy has `q` times the HP, and the party's hits have grown with its level.

With the scaling on, the four parts are:

- **The floor is scaled**: the original's floor times the `q` of the enemy (the same law as its HP, from the level of the unit's
  row and the party's level), rounded, at least 1: Zarbon's 30 becomes 182 against a party of level 150 (HP 1 410), Frieza's
  300 becomes 1 053 (HP 1 756), 20 becomes 67 and 50 becomes 158. The floor never exceeds the HP, since the original's floors are
  below the units' HP and the law is monotonic. At `q = 1` it is the original's.
- **A hit cannot skip the floor**: when the hit takes such an enemy to 0 or below, the HP is set to the scaled floor and the
  original goes on as for a hit that left it alive, so that `05:A4F5` makes it withdraw and its reward is paid. A hit that leaves
  the enemy above the floor changes nothing, and one that leaves it between 1 and the floor withdraws it there, as in the original.
- **The end of the second form's fight** (`05:9802`) reads the HP the withdrawal left, the scaled floor, against the scaled floor
  and one: the story goes on exactly as for the original's 20.
- **The Ginyu duel's guard** keeps the enemy at the scaled 1 HP (`q`, at least 1) instead of 1 until the swap, and the swap
  (Ginyu's own command, not a test of his HP: the reads of the enemy's HP found in the code are the displays) and the death
  that follows it are the original's.

Unscaled fights, the scripted loss (group `$57`) and a game without the pack are left as they are: every hook above declines
without the feature in the image, and the real-ROM test runs the original's hit step on both machines.

`1D:8C66` reads the party again when an enemy is defeated. It finds the party the build used: a fighter who is knocked out keeps
bit 7 of his flags, and no level changes before the battle ends. (The final fight of the story is two battles with a scene
between; its second `01:F719` reads the party that is left, so Frieza's BP there follows that party, and his HP is the saved
one the scene copies.)

### The wide BP

A BP past 24 bits is kept as the mod state's wide BP is (`state.md`): the low 24 bits in the record, and the high 16 in the
slot's part of the block at `$7F:FB00 + $12 + 2 * slot`. The block is made live (its used bytes and scratch zeroed, then the
marker) only when an enemy needs it, so a run whose BP all fit 24 bits never touches it; once live, every build writes all five
high parts (zero for a BP that fits), so a part from a stronger battle does not stay. The damage routine and the copies of the
wide BP (`state.md`) then take the enemy's high part as they take any record's. The readers that show a BP, such as the battle
HUD digits, take the low 24 bits (`state.md`).

## What is not covered

- Fodder is scaled like a boss. A row's BP is about the same multiple of `u^3` for the regular enemies (1.3 to 2.3), so at a
  party level far above a weak enemy's own it is about as strong as a boss made for that level, where the original keeps it
  weak: the order of two enemies of very different levels is not kept far from both of their levels. The game has no "level the
  story meets this enemy at" to use instead, and the lever for a gentler game is `easy`.
- The party's own growth is the game's, unless `level_cap` extends it ([the level cap](#the-level-cap)), and the law is fitted to Goku's. Five of the characters grow along curves that are 0.3
  to 0.4 times his BP at the same level, and two (Piccolo and Vegeta) pass it in the top levels; the law does not see it.
- The levels the story asks of the party (the forced fight with Raditz, the end of the Saiyans' chapter, the Super Saiyan
  transformation, the flags of map 16) are not touched: that is the other part of the mod.
- A scaled BP past 24 bits meets the routines that `state.md` lists as not yet wide (the form changes `05:A54A` and `05:A622`,
  the power-up `05:A727`): they wrap it at 24 bits. It takes a party above level 50 (a raised level cap) to reach it: 51 at
  hard and 58 at normal for the largest BP of the original's rows. The level cap's own level-up and Super Saiyan hooks write the wide BP
  whole; the form changes and the power-up of those lines are still the original's.
- The party's HP and KI are cut at 9 999, a gift of
  EXP that carries past 24 bits outside a fight's award (a King Kai trial) is not carried to the high byte, and the catch-up after a
  wipe (`05:8E3F`, which lowers the level byte by two before the check) re-adds the BP of the levels the byte lost: a rare path at a
  level above 99.
- The recompiled build must be regenerated: the keys are stops of the recompiler (`02:D37C` came with the offsets).
- The party's level is read from the records as the game holds them: a character the game raises on building the record
  (Vegeta is raised to level 17) counts at that level.
