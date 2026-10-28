# Data packs

A data pack changes the game in Enhanced mode, from files the game reads when it starts: maps, scenes, text, tables, art and
chapters. The Classic game is never changed by packs, and a pack holds no code: what data cannot express is a routine of the
game that a pack selects by name.

This page covers the pack itself, how a player installs and switches packs, and the index of the pages on each part.

## The pack

A pack is a folder with a `pack.toml` at its root, or the same folder as a `.ssdpack` zip (with `pack.toml` at the zip's root).
The manifest has one `[pack]` table, and the files of the pack's parts are named by the entries that follow it:

```toml
[pack]
id = "free.progression"                      # 3 to 64 of a-z, 0-9, '.', '_' and '-'; unique in a set
name = "Free progression"                    # the name the Data packs screen shows
version = "0.9.2"                            # major.minor.patch
game = "A8CF4F2A"                            # the CRC32 (hex) of the ROM the pack is for
author = "The project"                       # optional: who made the pack
description = "Story without level gates."   # optional: a few lines for the details window

[[depends]]                                  # optional: a pack that must be in the set, at least at this version
id = "super.kaioken"
version = "1.0.0"
```

A pack that the checks refuse is not used. The checks read the ROM's CRC32 and size, the ids and versions, the files, the
slots every part fills, the data's fit in the image, and the dependencies; a refusal names the pack and the reason.

## Installing a pack

Packs live in the `mods` folder of the data folder, `%LOCALAPPDATA%\dbz-ssd-recompiled\mods\` (the `SSD_DATA_DIR` variable
overrides the folder). Put a pack there as a folder or as a `.ssdpack` file:

    %LOCALAPPDATA%\dbz-ssd-recompiled\mods\free-progression\pack.toml
    %LOCALAPPDATA%\dbz-ssd-recompiled\mods\super-kaioken.ssdpack

A `.ssdpack` with HD art (faces, map tiles, pose variants) is unpacked once into `cache\mods\` of the data folder, where
the Enhanced picture reads it.

The Data packs screen's **Install a pack...** line installs a pack without opening the folder: A opens the system's file
picker (on Android the Storage Access Framework, where the mods folder is private to the app), and a file that is a pack is
copied into the mods folder as `<id>.ssdpack` (a file of the same id is replaced). The pack joins the list, off; a file that
is not a pack is refused and the screen says so.

Then, in Enhanced mode (`ssd-recompiled --mode enhanced`), open Options, then Gameplay, then Data packs. The screen lists
every pack of the folder:

- a filled box is a pack in use, an empty box one that is off, and a red cross one the checks refuse (its details say why;
  it stays off);
- the details window shows the pack's name, version, author, description, what it needs and its size;
- A switches the pack under the cursor on or off;
- Y held with Up or Down moves the pack in the order, which is the order the packs load in;
- X asks to apply the set. Apply writes it and restarts the game to the title: save first, unsaved progress is lost. B goes
  back to the Options.

The set is kept in `mods\mods.toml`, which names the enabled packs by id, in their load order:

    enabled = ["super.kaioken", "free.progression"]

A missing file enables none. A windowed run loads the set at its start. If the set is refused, or names a pack the folder does
not hold, the game starts without packs and the screen says why. The command line overrides the file: `--mods <folder or
.ssdpack>` (repeatable) runs the given packs, and `--no-mods` runs none. A run that does not persist (a headless run, a UI
snapshot, a benchmark or a replay) reads no set file.

## The parts of a pack

Each part has its own page. Its file is named in the manifest by a `[[...]]` entry.

- `chapters.md`: the chapters a pack offers, the chapter session with its own saves, and the unlock;
- `tables.md`: the units, attacks, encounter groups and the characters' levels;
- `features.md`: the named features a pack selects, and their parameters;
- `free-progression.md`: the enemies that follow the party's level, with the design and its numbers;
- `maps.md`: the maps, made in Tiled, and their collision;
- `triggers.md`: the triggers that start a pack's scenes, and the talk of the pack's maps;
- `text.md`: the messages and names;
- `art.md`: the HD art of the pack's faces and maps, and its pose variants;
- `state.md`: the mod state a pack keeps in a save: the extended flags and the wide battle power.
