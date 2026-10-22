# tools/re — dynamic coverage Mesen2 -> Ghidra

Reverse-engineering tools for DBZ Super Saiya Densetsu (SNES, LoROM SlowROM, code in banks
`$00-$1F`). **This repository copy is canonical**; `C:\Tools\re-dbz` is the old location and is no
longer maintained. The repository holds scripts only: no ROM, no trace, no label file, no Ghidra
database. Everything the tools produce goes to `.work/re/` (or outside the repository) and is
never tracked (`.gitignore` and `cargo xtask publish-check` refuse it).

The game's names and addresses live only in `re/symbols.toml`. `cargo run --release -p xtask --
export-mlb [out-dir]` turns them into `dbz.mlb` (Mesen2 labels) and `symbols.lua` (read by
`trace.lua`), by default in `.work/re/`.

## Tool locations (environment variables)

| Variable | Used by | Value on this machine |
|---|---|---|
| `MESEN_DIR` | `run_trace.sh` | `C:/Tools/re-dbz/mesen-headless` (required; the script cleans its save folders) |
| `GHIDRA_HOME` | `run_ghidra.sh` | `C:\Tools\ghidra_12.1.4_PUBLIC` |
| `JAVA_HOME` | `run_ghidra.sh` | `<the JDK 21 folder>` |
| `GHIDRA_PROJECTS` | `run_ghidra.sh` | `C:\Tools\ghidra-projects` |

`mesen-headless/` is a portable Mesen 2.1.1 instance, separate from the user's own. Its
`settings.json` enables I/O access for scripts (`AllowIoOsAccess`). It is not in the repository.

## 1. Headless trace (Mesen2 `--testRunner`)

```bash
export ROM="$LOCALAPPDATA/dbz-ssd-recompiled/cache/rom/canonical.sfc"
tools/re/run_trace.sh "$ROM" run1 36000 mash      # intro + repeated Start/A
tools/re/run_trace.sh "$ROM" run2 36000 walk      # map exploration (long directions + A)
DBZ_SEED=1  tools/re/run_trace.sh "$ROM" run3 36000 auto
DBZ_ROUTE="right:1200,down:1200,left:1200,up:1200" tools/re/run_trace.sh "$ROM" run4 54000 explore
```

To trace what a reference replay does, write its input timeline with `cargo run --release -p xtask -- shots <script>
--mesen-inputs .work/inputs.txt` and run `DBZ_INPUTS=<its absolute path, forward slashes> tools/re/run_trace.sh "$ROM" run5 <frames> idle`
(Mesen's timing is not the oracle's: the scenes reached are close, not identical). `cargo run --release -p xtask -- routines`
then builds the routine table from every run of `.work/re/` (see the README, "Headless reverse-engineering tools").

Output in `.work/re/<run>/`: `exec.csv`, `calls.csv`, `jumps.csv`, `prgrom.cdl`, `summary.txt`.
All runs start from boot and are deterministic for a given mode/seed/route. For several runs in
parallel give each its own `MESEN_DIR` (a copy of `mesen-headless/`).

- `trace.lua` requires `DBZ_OUT` and `DBZ_SYMBOLS` (`run_trace.sh` sets both) and holds no game
  address: the variables it reads come from `symbols.lua` by name (`in_battle`, `travel_mode`,
  `map_menu_cursor`). It registers an exec callback. For each PC it saves the M/X/E flags, D and
  DBR, resampled at hits 1, 16, 256, 4096 and 65536. It also saves the targets of JSR/JSL and of
  indirect jumps, validated against the operand to discard interrupts, and the PRG-ROM CDL.
- Input modes (`DBZ_MODE` / fourth argument):
  - `mash`, `walk`: fixed cycles.
  - `auto` (`DBZ_SEED`): reproducible random pilot reading the WRAM (`in_battle`). In battle it
    presses A/directions/B (commands, Pick/Auto, cards, items, escape); outside it alternates
    long moves, A, B, Start.
  - `explore` (`DBZ_ROUTE`): long-range travel. On the ground it opens the menu and moves the
    cursor (`map_menu_cursor`: 0 Talk, 1 Look, 2 Fly, 3 Item, 4 Menu) to Fly; when flying
    (`travel_mode` = 2, walking 1) it holds the route directions, pressing A then B every ~5 s;
    in battle as `auto`.
  - `battle`: rhythmic A plus cursor moves only; `idle`: no input.
- Fast exploration without a trace (~300 frames/s): `DBZ_TRACE=0`. Other variables: `DBZ_LOAD` /
  `DBZ_SAVE` (Mesen savestates: they contain the game's WRAM/VRAM), `DBZ_INPUTS` (timeline
  `"<n> <keys>"` + `@mode`), `DBZ_SHOT_DIR` / `DBZ_SHOT_EVERY` (screenshots), `DBZ_RAMDUMP` (WRAM
  at the end). Savestates, screenshots and WRAM dumps belong in a scratch folder under `.work/`,
  never tracked.

### Importing the labels into Mesen (manual)

`.work/re/dbz.mlb` holds one label per line (`SnesPrgRom:<offset>[-<end>]:<name>` for routines,
tables and sync points, `SnesWorkRam:<offset>[-<end>]:<name>` for variables). In the Mesen2
debugger open *Labels* (Debug > Workspace > Import labels) and pick the file, with the ROM loaded.

## 2. Headless Ghidra project

```bash
export GHIDRA_HOME='C:\Tools\ghidra_12.1.4_PUBLIC'
export JAVA_HOME='<the JDK 21 folder>'
export GHIDRA_PROJECTS='C:\Tools\ghidra-projects'
tools/re/run_ghidra.sh "$ROM"      # default traces: every folder under .work/re/
```

It recreates `$GHIDRA_PROJECTS/dbz_trace` with these steps:

1. `DbzMirrors.java`: creates the initialized ROM banks `$00-$1F`, the low WRAM and I/O mirrors,
   and makes the ROM read-only.
2. `DbzApplyTrace.java` (the trace folders are its argument, no default): sets DP=0, DBR and
   M/X/E for every observed PC, disassembles and creates functions.
3. Runs auto-analysis.
4. `DbzInfo.java`: prints statistics.

`DBZ_BASELINE=1` creates only the functions at the vectors, for comparison.

In the Ghidra GUI the same scripts run from the Script Manager (folder `ghidra_scripts`) on a
**fresh** import of the ROM; then run *Analysis > Auto Analyze*. `DbzDecomp.java` prints the
decompiled C of the functions at given addresses (diagnostic).

## 3. SLEIGH patch `patches/ghidra-snes-native-dp.patch`

Makes the direct page address computation (`ref_direct`) native. The original read the E flag
from `contextreg` at runtime and the decompiler showed it as `in_register_00000040`, with double
OR/ADD branches on every DP access. The game is in native mode from the 4th RESET opcode on.

Applied on 2026-10-07 to the installed extension
(`%APPDATA%/ghidra/ghidra_12.1.4_PUBLIC/Extensions/ghidra-snes`) and rebuilt with
`support/sleigh.bat`. If the extension is reinstalled, apply it again (`git apply -p1` from the
extension folder, Ghidra closed) and rebuild.

## 4. Ghidra GUI + MCP

The scripts of GhidraMCP are enabled by the user variable `GHIDRA_MCP_ALLOW_SCRIPTS=1`, read when
Ghidra starts. For incremental coverage, run `DbzApplyTrace.java` through MCP on the open program:
copy the script to `%USERPROFILE%/ghidra_scripts/`, `run_ghidra_script` with the argument
`<trace dir 1>+<trace dir 2>+...`, then `run_analysis` and `save_program`, and remove the copy.
The script is incremental: it skips PCs already disassembled with the same M/X/E mode and
re-disassembles only those with a different mode.

## Scene scripts

`python tools/re/scenes.py [--rom canonical.sfc] [scene ...]` lists the story scripts of the cached ROM (the
82 "scenes" of `docs/re/data/scenes.md`), one command per line, with jump labels and the decoded flag, battle
group, map and character of the commands that have one. The output is derived from the game: stdout only.

## Reads of a level, BP or EXP

`python tools/re/story_level_reads.py [--rom canonical.sfc] [--all]` lists the instructions of the cached ROM that read or write the
level, the BP or the EXP of a character record (party, enemy slot, side record, roster image, BP snapshot; absolute, indexed, long and
record-relative forms), grouped by routine, with the counts of `docs/re/data/free-progression.md`. A match is the bytes of an
instruction, so a table that holds the same pair matches too: read each routine with `xtask disasm`. The output is derived from the
game: stdout only.
