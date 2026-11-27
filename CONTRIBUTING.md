# Contributing

Thanks for looking. The project is a Rust rewrite of a 1992 game, and the one rule that matters more
than any other is: **nothing from the game goes into this repository.** Not graphics, not text, not
tables, not disassembly, not screenshots of the original, not a save, not a replay. The game is read
from the player's own ROM at run time.

Everyone works with their own copy of the game. ROMs, patched ROMs and anything extracted from them
(graphics, music, text dumps) are never shared in issues or discussions, and a pull request
that contains any of them is not accepted.

## What may be committed

- Our own code, tools, tests and notes. Research notes (`docs/re/`) describe routines in our own
  words and may quote at most five consecutive instructions of the original code; a short quote is
  not a disassembly.
- A table of 16 bytes or more that comes from the ROM is never pasted into a source file (as
  `0x..` literals or as `b"\x.."`); read it from the ROM at run time and name it by symbol.
- Pictures only in `assets/` (art made for this project) and `docs/media/` (the pictures of the
  README).
- Scratch output goes to `.work/` (never tracked) and is deleted when it is no longer needed.

`cargo xtask publish-check` enforces this (it needs the ROM cached once with
`ssd-recompiled --rom <your rom>`; `--allow-no-rom` skips the ROM scan, for CI). Run it before every
push.

## Before a pull request

Pull requests go to `main`, one topic each.

```
cargo fmt -p ssd-rom -p ssd-oracle -p ssd-game -p ssd-app -p ssd-catalogue -p ssd-save -p ssd-hd -p ssd-hdpack -p xtask -p ssd-recomp -p ssd-menus -p ssd-mod -- --check
cargo clippy -p ssd-rom -p ssd-oracle -p ssd-game -p ssd-app -p ssd-catalogue -p ssd-save -p ssd-hd -p ssd-hdpack -p xtask -p ssd-recomp -p ssd-menus -p ssd-mod --all-targets -- -D warnings
cargo test -p ssd-rom -p ssd-oracle -p ssd-game -p ssd-app -p ssd-catalogue -p ssd-save -p ssd-hd -p ssd-hdpack -p xtask -p ssd-recomp -p ssd-menus -p ssd-mod
cargo xtask publish-check
```

- Do not run `cargo fmt --all`: the vendored emulator under `third_party/jgenesis` must stay
  byte-identical to upstream, apart from the patches listed in `third_party/jgenesis/PATCHES.md`
  (each patch is its own commit and has an entry there).
- Tests that need the ROM read `SSD_ROM_PATH` and are `#[ignore]`d; run them with
  `cargo test -p ssd-rom -p ssd-oracle -p ssd-game -p ssd-app -p ssd-catalogue -p ssd-save -- --ignored`.
- Anything that can change what the original game does (the emulation core, a Rust routine, the
  recompiler, `re/symbols.toml`) must keep the reference replays identical: see the developer guide
  (`docs/DEVELOPER.md`), "Developer commands", for `make-replays`, `verify-replays` and
  `standalone-diff`. Changes to the Enhanced mode only (menus, cheats, HD drawing, art) and to docs
  and tools need no replay run.
- A change to `crates/ssd-hd` is checked with `cargo run --release -p xtask -- hd-parity`.
- A change to the HD art (an art store, `index.json`) is checked with `cargo run --release -p xtask -- hd-catalogue extract`
  and `hd-catalogue verify --art <art store> --check` (README, "Developer commands"): a flag on a used entry that
  `tools/hd-catalogue-accept.toml` does not accept fails it; list only false alarms you looked at, with a reason.

## Adding story content

New story content (chapters, maps, scenes, battles, text, art) is made as a data pack for the Enhanced
mode, not as changes to the game's code: a folder with a `pack.toml`, or the same folder zipped as a
`.ssdpack`. The format and each of its parts are described in `docs/mods/` (start with
`docs/mods/README.md`), and `examples/` holds working packs. Packs are checked with
`cargo run --release -p xtask -- mod-check <pack>` and built with `mod-build` (`docs/DEVELOPER.md`,
"Developer commands"). A pack holds no data from the ROM: its files are made from scratch, and the
checks refuse a file that contains 16 bytes of the ROM. When a piece of the story needs something the
format cannot express yet, open an issue that describes it before writing code.

## Reporting a bug

A report is most useful with:

- the version (the release you downloaded), the platform and device, and the mode (Classic or
  Remastered), with the data packs that were on;
- what you did, what you expected and what happened instead;
- the screenshot taken at the moment with F12 (or the debug panel's "Save screenshot"): the `.png`
  and the `.ssdstate` saved beside it, which reproduces the moment exactly. On Windows they go to
  `Pictures\DBZ SSD Recompiled\`; on Android, and without a Pictures folder, to `screenshots\` of
  the data folder;
- the log: on Windows the text of the console window the game was started from; on Android
  `adb logcat -s ssd`;
- when the problem is in a save, the save file (`saves\ssd.sav` of the data folder,
  `%LOCALAPPDATA%\dbz-ssd-recompiled\` on Windows).

Do not attach the ROM or anything extracted from it.

## Checking the interface without a window

The tests and the tools never open the game's window or press real keys. Check the UI with an
offscreen snapshot instead, for example
`cargo run -p ssd-app -- --headless-frames 600 --ui-script ui.txt --ui-snapshot ui.png`
(`docs/DEVELOPER.md`, "UI snapshots").

## Style

English in code, comments, docs, commit messages and UI text. No feature that is not asked for, no
speculative generality; when something is a bug, collect the data first.
