# Features

A feature is a behaviour the game has by name. A pack selects it in its manifest and gives its parameters; the pack
holds no code, so the behaviour itself is part of the game. A feature the pack does not select is off, and nothing
changes for it.

## The manifest

```toml
[[feature]]
name = "ssj_kaioken"          # the feature's name
max_factor = 10               # its parameters, as the other keys of the table: all are required
hp_cost_percent = 10
```

Each feature is given once per pack. A name the game does not have is refused, and the refusal lists the names it knows.
A parameter is a whole number in a range, or one of the names the feature gives to its values (`difficulty = "hard"`): a
parameter that has names takes only them, and one that has none takes only a number. A parameter that is missing, that the
feature does not have, that is of the wrong kind or that is out of its range is refused, and the refusal names the feature
and the parameter.

## Known features

### `ssj_kaioken`

The Kaioken for Goku in the Super Saiyan form. The pack's command list for that form (`tables.md`, unit 59, with
`commands_only = true`) adds the Kaioken to the battle menu; the feature gives the form the Kaioken's rules:

- the Kaioken stays on through the transformation: the Super Saiyan form is kept while the Kaioken is used and when it
  stops;
- the multiplier of the Kaioken is capped, so its factor is at most `max_factor`;
- while the Kaioken is on, every move Goku makes costs `hp_cost_percent` of his maximum HP. The cost leaves him at least
  1 HP; a fighter at 0 HP is left at 0.

The Kaioken's KI cost, its boost and its end are the game's own, as the original gives them.

| Parameter | Range | Meaning |
|---|---|---|
| `max_factor` | 1 to 11 | The largest factor of the boost. The factor is `1 + m / 2`, where `m` is the multiplier the game takes from its table (up to 20); the cap is `m` at most `2 * (max_factor - 1)`. Eleven is the table's top, so it caps nothing. |
| `hp_cost_percent` | 0 to 100 | The percentage of Goku's maximum HP that each move costs while the Kaioken is on. `0` is no cost. |

The feature is for the Enhanced mode only; the Classic game is not changed. The example pack `examples/ssj-kaioken`
(`pack.toml` and `tables.toml`) is the feature with its command list.

### `enemy_scaling`

The enemies follow the party's level. Each enemy of a battle is set to the strength it would have if it had been made for the
party's level, and pays the EXP of that strength:

- the party's level is the mean of its three highest fighters, and an enemy's own level is the level of its record;
- its HP and KI are multiplied by `q`, its BP by `q^3`, the EXP it pays by `q^2`, and its level becomes the party's with the
  difficulty, where `q = difficulty * (party level + 5) / (enemy level + 5)`;
- with the party at the enemy's own level and the `normal` difficulty every number is the original's, and the numbers rise and
  fall with the party's level, bosses included, with no floor at the original's numbers;
- the fights the party must lose (the scripted losses of the cheats' list) are left as they are;
- a BP past 24 bits is kept as the mod state's wide BP (`state.md`);
- the enemy power cheat, the Options difficulty and the EXP cheat act on the scaled numbers, after it.

| Parameter | Values | Meaning |
|---|---|---|
| `difficulty` | `easy`, `normal`, `hard` | The factor on the party's level that the enemies are made for: 7/8, 1 or 9/8. Easy is BP x0.67, HP and KI x0.875, EXP x0.77; hard is BP x1.42, HP and KI x1.125, EXP x1.27. |

The feature is for the Enhanced mode only; the Classic game is not changed. `free-progression.md` is the design with its
numbers, and the example pack `examples/enemy-scaling` (`pack.toml`, no data files) is the feature alone:

```toml
[[feature]]
name = "enemy_scaling"
difficulty = "normal"
```

### `free_progression`

The story moves on what the party has done, not on how strong it is. The original asks for a level at several turns of its story:
a fight is forced, random battles stop, fighters are lost, a transformation is allowed, a character arrives, all when a level is
reached. The feature gives each of those turns an event of the story in place of the level. Everything else of the story,
and every fight, stays the original's.

What changes:

- **Raditz.** The forced fight with Raditz on the first chapter's map starts when the imposter in the cave has been beaten (after
  the Gohan card, as the original asks), whatever the levels of Goku and Piccolo. Before, it started when Goku reached level 5 or
  Piccolo level 4.
- **Random battles** of the first chapter's map and cave do not stop at those levels any more.
- **The recruits of the Saiyans' chapter** are not lost when Piccolo (or Gohan) reaches a level: they wait for the party to walk
  into the Saiyans, which is the original's other way to lose them, and which stays. (The Remastered story fix already does this;
  with the feature it holds on its own.)
- **Super Saiyan.** In the last fight Goku turns Super Saiyan when a friend falls, whatever his level. Before, he had to be at
  level 47. The group of the fight and the party are still asked for. He becomes the same level 48 Super Saiyan.
- **Goku's arrival at the Ginyu squad.** The original sets one of three arrivals from Goku's level (more training meant a later
  arrival): during the fight with Recoome and Guldo, with Jeice and Burter, or at the last fight. The parameter `ginyu_arrival`
  gives the arrival.
- **Piccolo's return after Porunga.** The original sets his BP from a table of eight values for the levels 33 to 40, and reads
  a wrong value at any other level. At another level the feature leaves his BP as it is.

What stays, and why: the **floors and gifts** that raise a level (they never lower one, and never raise more than one level above
their target), the **caps** (the data pack tables may change them), **Dende's healing** at level 3, **Gohan's rage** at level 11 and
the **Auto choice** of the Kaioken and the Spirit Bomb, which compare the two sides of a fight and unlock an ability: none of them
moves the story. `docs/re/data/free-progression.md` lists every place the story reads a level, EXP or BP, with what each one does.

| Parameter | Range | Meaning |
|---|---|---|
| `ginyu_arrival` | 0 to 3 | Goku's arrival in the Ginyu squad's chapter: `0` is the original's rule (his level decides), `1` the first fight (Recoome and Guldo), `2` the second (Jeice and Burter), `3` the last (Captain Ginyu, with Goku's landing). |

The feature is for the Enhanced mode only. The example pack `examples/free-progression` (`pack.toml`) enables it with
`ginyu_arrival = 3`. It does not change the enemies, their stats or their rewards: a party that reaches a forced fight early meets it
at the strength it has.
