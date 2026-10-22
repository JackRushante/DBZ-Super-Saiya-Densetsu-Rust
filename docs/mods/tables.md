# Data tables in data packs

A data pack can add or replace the game's tables of fighting data: the units (enemies and the other fighters) with
their stats, names and command lists; the attacks; the encounter groups; and the characters' levels, with their caps.
It does this with one file, `tables.toml`, that the pack's manifest names. The game does not change: each row is
checked when the pack loads and laid out in the bytes the game already reads, so a battle, a level-up and a status
line use the pack's numbers exactly as they use the original's.

## The manifest

```toml
[[tables]]
file = "tables.toml"
```

A pack may name several tables files. A row given twice, in one file or in two files of the pack, is refused.

## Ids

Each table is numbered. An id inside the original range replaces that original row; an id past it adds a new one.

| Table | Original ids | Replaces | New ids |
|---|---|---|---|
| `[[unit]]` | 0 to 104 | the unit's stats, name and command list | 105 to 254 |
| `[[attack]]` | 0 to 68 | the attack's name, KI cost and animation | 69 to 254 |
| `[[group]]` | 0 to 111 | the encounter group | 112 to 254 |
| `[[character]]` | 0 to 11 | the character's levels and cap | (none) |

A new unit needs a `sprite_like` and a command list. A replaced unit keeps its command list and its sprite unless the
file gives them.

## Units

```toml
[[unit]]
id = 105
name = "Guard"
class = 1          # 0 to 6
level = 5          # 0 to 255
hp = 300           # 0 to 65535
bp = 1000          # 0 to 16777215
ki = 50            # 0 to 65535
defence = 30       # 0 to 255, the defence factor of the damage formula
tie = 7            # 0 to 255, the tie-break when two fighters act at the same time
exp = 0            # the EXP the unit gives when it is beaten, 0 to 16777215
sprite_like = 4    # an original unit (0 to 104) whose battle picture the unit uses

[[unit.command]]   # the command list, slot 0 first
id = 0             # a command id, 0 to 255 (255 is a blank line)
attr = 16          # 0 to 255, the animation number of the move
factor = 4         # 0 to 255, the attack factor of the move (255 is none)
```

To replace only the command list of an original unit (0 to 104), give `commands_only = true` and the list. The stats, the
name and the sprite stay the original's: a stat, a name or a `sprite_like` next to it is refused, and a new unit (105 and
up) cannot be given this way.

```toml
[[unit]]
id = 59
commands_only = true

[[unit.command]]
id = 0
attr = 16
factor = 8
```

To add moves after an original unit's own list, give `commands_append = true` and only the new slots: the game reads the
unit's list from your ROM when the pack is loaded and adds them at the end, so the pack holds nothing of the original. The
two keys cannot be given together.

```toml
[[unit]]
id = 59
commands_append = true

[[unit.command]]
id = 0x33
attr = 0xFF
factor = 255
```

The name is plain text of one to fifteen tiles: letters, digits, a space and `! ? . , ' -`, the ellipsis and the quote
marks. A name with a `{...}` slot is refused.

A new unit borrows its battle picture from `sprite_like`: the pose set of the original unit is used, and the unit's own
row gives its stats. The command list of a unit is what the battle menu and the counter-attacks of the enemy read.

## Attacks

```toml
[[attack]]
id = 69
name = "Guard Blow"
ki = 10                 # the KI cost, 0 to 255
animation_like = 0      # an original attack (0 to 68) whose effect and animation the attack uses
```

`animation_like` borrows the effect and animation of an original attack: an attack that uses an effect of the
original (for example the Kaioken's) plays it. The KI cost and the name are the attack's own. Attack `0` (the plain
attack) shows the name of attack `$44`, as the original does: to rename the plain attack, give a name to `68`.

## Encounter groups

```toml
[[group]]
id = 112
units = [105, 105]     # 1 to 5 unit ids, in order
```

The units are placed where the game puts a group of their number: one enemy in slot 0; two in slots 1 and 2; three in
slots 0, 3 and 4; four in slots 1 to 4; five in all. The other slots are empty. A group is used by a battle the scripts
start by its number, or picked by a random encounter of a map that names it.

## Characters: levels and caps

```toml
[[character]]
id = 0           # 0 to 11
cap = 100        # the level cap, 1 to 100

[character.curve]
hp = { a = 80, b = 12, c = 1.0 }     # the max HP at level L: a + b * L^c
ki = { a = 4, b = 0, c = 1.0 }       # the KI the level adds
bp = { a = 0, b = 100, c = 1.2 }     # the BP the level adds
exp = { a = -10, b = 10, c = 2.0 }   # the EXP at which level L begins
tie = 0                              # the tie-break byte, the same for every level

[[character.row_override]]           # optional: a level whose values change
level = 48
hp = 900
```

Each value is `a + b * L^c` for each level `L` from 1 to 100, rounded to the nearest whole number (halves away from
zero). The packer refuses a formula that gives a value outside its field (`hp` and `ki` 0 to 65535, `bp` and `exp`
0 to 16777215) at any level, and a curve whose level 1 has an EXP other than 0: level 1 begins at 0 EXP.

Instead of a curve, a character may give its rows one by one: exactly 100 `[[character.row]]` tables, level 1 first,
each with `bp`, `exp`, `hp`, `ki` and `tie`, with the same ranges and the same level 1 rule. A `[character.curve]` and
rows cannot be given together, and `row_override` goes with the curve only.

The BP and the KI of a level are added to the character's BP and KI when the level is reached; the HP is the maximum
for the level; the EXP is the threshold of the level. A character with a cap below 100 never reads the rows above it.

## What the tables do not change yet

Some routines read a unit's row from the original table by its id, outside the routines the tables redirect. For a unit
with a pack row they still read the original row:

- the class of a unit, which the battle reads by unit id (the card that matches the class gives the bonus);
- the defence factor that a routine sets in a party record from the unit it names;
- the EXP that a defeated enemy adds to the battle's total.

Other limits:

- the promotion of a character (its level is set to the cap plus one) reads the original cap and the original level rows
  for the values it copies;
- an effect that adds one level to a character stops at 99;
- a cheat that multiplies the enemies of a random encounter rebuilds them from the original group.
