# Chapters in data packs

A data pack can offer a chapter: a start of the story with its own party, levels, place, story flags, money and items. A
chapter does not run new engine code. It is a save slot that the pack synthesizes, and the game's own Continue loads it. This
page covers the chapters file, what a chapter's slot holds, the chapter session with its own saves, and the unlock.

## The manifest

```toml
[[chapters]]
file = "chapters.toml"
```

A pack may name several chapters files. The chapters of all the packs of a set are one list, and each id is unique in it.

## The chapters file

A chapters file holds one `[[chapter]]` table per chapter. The numbers are decimal.

```toml
[[chapter]]
id = "androids"                 # 1 to 32 of a-z, 0-9 and _
name = "The Androids"           # the name the title shows
description = "Goku's family."  # a few lines the details window shows (optional)
needs_completed = true          # waits for the game to be completed (see "The unlock")
map = 64                        # 0 to 47 (an original map) or 64 to 127 (a pack map of the set)
leader = [1024, 768]            # the leader's position, in pixels of the map
follower = [896, 648]           # the follower's position
party = [{ character = 0, level = 30 }, { character = 1, level = 20 }]
flags = [3, 29]                 # story flags set at the start
flagx = [1]                     # extended flags set at the start
items = [{ object = 5, count = 3 }]
zeni = 200
```

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | the chapter's id, unique in the set: its save folder is named by it |
| `name` | yes | the name the title shows; not empty |
| `description` | no, empty | a few lines the Data packs screen shows in the chapter's details |
| `needs_completed` | no, `false` | the chapter waits for the game to be completed |
| `map` | yes | the map the chapter starts on: `0` to `47`, or a pack map of the set (`64` to `127`); the town screens `48` to `63` are refused |
| `leader`, `follower` | yes | two numbers each, `0` to `65535`: the positions in pixels of the map |
| `party` | yes | one to nine members, the leader first; each character (`0` to `11`) once; a `level` from `1` to `100` |
| `flags` | no | story flags `0` to `127` set at the start |
| `flagx` | no | extended flags `0` to `2047` set at the start |
| `items` | no | entries of the item list: an `object` (`0` to `255`) and a `count` (`1` to `24`) of entries of it; at most 24 entries in all |
| `zeni` | no, `0` | the money, `0` to `999` |

A key that is not in this table is refused, so a misspelt key is never a silent default. The levels must be in the game's level
table for the character: a level the table does not have is refused when the chapter is built, and so is a character whose stats
come from a unit row, given a level other than the unit's.

## What the slot holds

The chapter's slot is slot 1 of its save. The game's builder makes the party's records, as it does for the same party in
`xtask save-edit`: each member's record at its level, with its battle power, HP and KI, and its EXP. The party list and its size
are set, and so are the leader's and the follower's positions and the map. The story flags, the money and the items are set as
the table gives them. The byte the map's set-up gives for the countdown is set too (`$01E4`): for a pack map it is `0`.

The extended flags do not fit in the save. They go to the sidecar `ssd.mods` beside the slot, in the same form the
game's mod state uses, keyed by the slot's checksum. A chapter without extended flags has no sidecar.

The slot does not carry what the game keeps outside its save: the names the dialogue fills in, the scene in progress and the
story-transition state (`docs/re/data/title-chapters.md`, section 12). The map's own triggers start a scene when the flags and the position say so.

## The chapter session

A chapter starts from the Data packs screen (Options, Gameplay, Data packs): A on an open chapter asks "Start this chapter?",
and Start restarts the game into the chapter. The chapters listed are those of the packs that are on in the list; a chapter
starts from the set in use, so an applied set is needed first. The command line does the same, in an Enhanced run:

    ssd-recompiled --mode enhanced --chapter androids

It takes the set `mods\mods.toml` enables, or the packs given with `--mods`. An unknown id, or a set that is not used, is an
error that names the reason.

The chapter's save and its sidecars live in their own folder under the saves folder, `saves/chapters/<id>/`, and the main
saves are never touched. A folder with a battery save already continues it. Only a run that persists writes the files (a
windowed run does); any other run starts the chapter from the fresh slot in memory and writes nothing. A chapter that waits
is listed grey and says "locked" until the unlock below; the command line does not check it.

## The unlock

A chapter with `needs_completed = true` is offered once the game was completed. The completion is recorded by the endings of the
game: the two mode inits that run once per ending write a marker in the mod state block; the driver reports it once as an
application event; the app writes the file `completed` in the saves folder, one line with the date of the first completion
(`YYYY-MM-DD`, UTC). The endings are recorded in Enhanced runs only. The unlock rule is `ssd_mod::chapter::unlocked`: a chapter
is offered when it does not wait, or the `completed` file exists, or a cheat unlocks it. The cheat is the Cheats screen's
row Unlock chapters (Story, Off or On), kept with the save's cheats like the other rows.

## The tool

    cargo run --release -p xtask -- chapter-save <pack>... --chapter <id> --out <folder>

writes the slot a chapter starts from, as `<folder>/ssd.sav` and, for a chapter with extended flags, `<folder>/ssd.mods`. It is
the same synthesis as the chapter session, and it refuses to overwrite a file. `save-dump <folder>/ssd.sav` reads the result back.
