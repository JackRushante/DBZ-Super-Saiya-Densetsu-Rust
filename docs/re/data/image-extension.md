# Research 1 and 7: extended image, replay and save-state plumbing

Tags: SEEN = read in code or run, with file:line or command; INFERRED = reasoned, not run.
Scratch: `.work/m10-research/ext/` (own `[workspace]`, own target dir `.work/m10-research/target`); no tracked file was touched.

## Verdict

**GO** for the extended image (2 MiB, 4 MiB) with **no core patch**. Two small oracle-side changes are needed (below), both outside `third_party/`.

## R1 - Address space and image extension

### Findings

1. SEEN `Machine::with_wram_seed` (`crates/ssd-oracle/src/machine.rs:463-472`) hands the `Vec<u8>` straight to `SnesEmulator::create` and builds its own `Arc<[u8]>` copy with `rom_image` (`machine.rs:2811`: strips a 512-byte header, mirrors to the next power of two). That copy is used only by `peek`/`peek_code` (`machine.rs:2140-2155`, masks with `len-1`), so a 2/4 MiB image works there unchanged.
2. SEEN The core (`third_party/jgenesis/backend/snes-core/src/memory/cartridge.rs`): `Cartridge::create` (l.248) rejects only `< 32 KiB`; mirrors to a power of two (l.277-281, no-op for 2/4 MiB); `guess_cartridge_type` (l.873) reads the LoROM header at the fixed address and the reset vector. The ExHiROM branch needs `len >= 0x410000` AND map byte `$25/$35` (l.889-893), so 4 MiB with our map byte `$20` stays LoROM. Region, SRAM size and chipset come from header bytes at the same fixed addresses (l.296-321), which do not move.
3. SEEN The header is not checked against the image: the core never reads the ROM-size byte or the internal checksum (only `CRC.checksum(rom)` against fixed coprocessor/PAL lists, l.365, 955, 1020). Header of the cached ROM: map `$20`, ROM-size byte `$0A` (1 MiB), left unchanged by extension, harmless.
4. SEEN LoROM mapping `lorom_map_rom_address` (cartridge.rs:1071): `((addr & 0x7F0000) >> 1) | (addr & 0x7FFF)`, masked by `rom_len-1`. So the new banks are `$20-$3F` (image 1..2 MiB, 1 MiB of space), mirrored at `$A0-$BF`; with a 4 MiB image `$40-$7D` also reach 2..4 MiB (banks `$7E/$7F` are WRAM, so LoROM tops out at `$7D`, about 3.94 MiB). Today (1 MiB image) banks `$20-$3F` are an alias of `$00-$1F`, so a read there returns mirror data.
5. SEEN Access speed is by bank number, not image size (`bus.rs:164-198`), so reads in `$20-$3F` cost the same SlowROM 8 master cycles as banks `$00-$1F`.
6. SEEN The recompiled runtime reads data only through `Ctx::read8` -> `Machine::bus_read` -> the core bus (`crates/ssd-oracle/src/ctx.rs:608-611`; `ssd-recomp/src/rt.rs:518`). SEEN the generated file `.work/recomp/generated.rs` holds, outside function bodies, only `COV`, `VARIANTS`, `TABLE`, `MIDS`, `MID_TABLE` (grep `^pub static`), no ROM data tables; function bodies are the code of banks `$00-$06,$10,$13,$1C,$1D` (modules `b00..b1D`) with opcode immediates. So code is static and tied to the original 1 MiB; data is read live from the image. Consistent with Principle 2.
7. SEEN The 1 MiB cached ROM is full: every half-bank has data up to or near `$7FFF` (script over `canonical.sfc`: last non-fill offset per bank, e.g. `$7FFE`, `$7EB1`, `$759E`; the emptiest are banks 07 and 13 with 5-7 KB of scattered FF/00, not a free block). New data belongs in the extension.
8. SEEN Experiment (scratch program `ext`, built with `cargo build --release --offline` in `.work/m10-research/ext`): baseline = `Replay::power_on(rom)`; test = `Machine::with_wram_seed(image, ...)` with the replay's seed and initial SRAM, both played with `run_frame_with`, per-frame digest (FNV of WRAM, VRAM, CGRAM, OAM, SRAM, frame pixels, total master cycles). Command: `.work/m10-research/target/release/ext.exe <canonical.sfc> battle menus landing`. Output (no hooks, Classic, 7200 + 6878 + 6022 frames):
   - 2 MiB original + zero padding: first difference **None** on all three
   - 2 MiB original + pseudo-random padding: **None** on all three
   - 4 MiB original + pseudo-random padding: **None** on all three
   Every frame of the three replays has the same digest for each image. Random padding makes the test sensitive to any read in `$20-$3F`/`$A0-$BF` (mirror data today, random bytes with the image). Wall time 4 min (release, no hooks).
9. INFERRED So the game does not read `$20-$3F`/`$A0-$BF` (nor, for 4 MiB, `$40-$7D`) in those replays. Limits: three replays only (rules forbade the full set); an unplayed path could still read a bank. There is no bus-read log of original-code reads (`Machine::log_reads` records only `Ctx::read8` reads of generated/Rust code, `machine.rs:2303`), so no bank histogram. The real proof is Task 1's acceptance plus the full set once per milestone on a random-padded 2 MiB image.
10. NOT RUN (stated): the recompiled (`--features recomp`) path on the extended image; a save state taken and loaded on an extended machine; a deliberate negative control (a modified first MiB) for the digest.

### Needed changes (oracle only, no `PATCHES.md` entry, no `CORE_VERSION` change)

- A. `Machine::rom_crc32` must stay the CRC of the **original** ROM for every machine (it feeds `Replay::of`, `Replay::power_on`, the state header at `machine.rs:2722/2732`, and callers like `ssd-app/src/app.rs:1171`). Today it is computed from the bytes passed in (`machine.rs:471`), so passing the extended image would change it. Add a constructor, e.g. `Machine::with_image(rom: &[u8] /*original*/, image: Vec<u8>, saves, seed)`: CRC from the original, the image goes to the core and to `self.rom`. `Machine::new/with_wram_seed` stay as they are (Classic path: no code path from a pack).
- B. The builder must hand a power-of-two length (2 MiB / 4 MiB): both `rom_image` and `Cartridge::create` mirror a non-power-of-two image by repetition (SEEN cartridge.rs:277-281, machine.rs:2813; the mirror function itself was not read), which would silently duplicate pack data.
- CORE_VERSION: SEEN `replay.rs:43-63`: CRC of the vendored `.rs`/manifests plus `ORACLE_REVISION`, bumped only when what a run without hooks does from power-on changes. A new constructor not used for Classic changes neither, so no bump and no `rebase-replays` of the 30 replays. If A is done by editing `with_wram_seed` itself, re-check.

## R7 - Replay and save-state plumbing

### What carries what today

- SEEN Replay header (`crates/ssd-oracle/src/replay.rs:1-30, 235-262`): `SSDRPLAY | format u32 | crc32 | rom_crc32 | wram_seed u64 | core_version | sram | frames`, then inputs. Format 1 = Classic with no events. Format 2 is written only if `mode != Classic` or any frame has a host event (`to_bytes`, l.246): a mode byte after the frame count, then per frame the joypad word and its events. Event kinds: `0` WRAM write, `1` App {tag u8, payload}.
- SEEN The cheats ride as **App events**, not header fields: `CHEATS_TAG` (`ssd-game/src/menu/driver.rs:756`), applied on playback by `Driver::replay_event` (`driver.rs:427-445`) from `hd_frame` (`ssd-app/src/app.rs:350`). Other tags: `SPAWN_TAG`, `BIND_TAG` (keeper.rs).
- SEEN `Replay::from_bytes` rejects bad magic/format, CRC, `rom_crc32` != expected, core version != this build, trailing bytes. `Replay::rebase` (l.289) rewrites only the core version and refuses a file that does not re-serialize to itself; it passes the file's own `rom_crc32`, so it works with any CRC.
- SEEN Save state (`crates/ssd-oracle/src/state.rs:1-60`): `SSDSTATE | format 3 | crc32 | rom_crc32 | frames | payload_len | core_version | bincode payload`. The ROM is not in the payload. A state loads only if `rom_crc32` and core version match (`machine.rs:2732`). It records nothing about packs, mode or cheats.
- SEEN `Ruleset` (`ssd-game/src/mode.rs:19`) is `Copy` with `const fn` constructors (`mode`, `bag`, `cheats: bool`); it cannot hold a pack list as is.
- SEEN Every consumer builds the machine through `Replay::power_on(rom)` (app `replay_io.rs:144`, `lockstep.rs:339-340`, about 25 xtask call sites: coverage, framecheck, fuzz, hd_art_run, bench_hd, standalone_diff:69-70). `power_on` checks `crc32(rom) == replay.rom_crc32` (replay.rs:157). It is the single choke point for the image.

### Proposal: how a pack set reaches a replay and a state

1. **Replay: a new optional section, not an App event.** The image must exist before the machine is created, and `power_on` runs before frame 0's events are applied; an App event at frame 0 could be pre-scanned, but every tool calling `power_on` would have to know. Add **format 3** = format 2 + a section (set id, image CRC32) after the mode byte, written only when the set is non-empty. Without packs `to_bytes` keeps writing format 1/2 exactly as now, so a replay without packs is **byte-identical** (provable by the "rewrites to itself" rule in `rebase` on all 30 replays). `from_bytes` accepts 1, 2, 3; format 3 requires mode Enhanced. `Replay` gains `packs: Option<PackBinding>`; `Replay::of` stays `None`, a recorder sets it.
2. **Header `rom_crc32` stays the original ROM's.** Then `Replay::from_bytes(bytes, crc32(original_rom), ...)` (`app.rs:1171`, `replay_io.rs:79`, `replays.rs:154`) and `rebase` keep working. The image CRC is checked in a new `Replay::power_on_image(&self, rom: &[u8], image: Vec<u8>)`: original CRC against `rom_crc32`, `crc32(image)` against the section's CRC, then `Machine::with_image` (change A). A wrong set gives an error naming set and CRC. `power_on(rom)` on a format-3 replay returns "needs packs".
3. **Save state without a format bump.** `Machine` gets `image_crc32: Option<u32>`; `save_state/load_state` pass `image_crc32.unwrap_or(rom_crc32)` as the header key (`machine.rs:2722/2732`). No pack: bytes identical to today, existing `.ssdstate` still load. With packs: a state loads only into a machine with the same image. Cost: the state does not name its set (only a CRC); the app gets the set from the sidecar. If states must be self-describing, bump `FORMAT_VERSION` to 4 and add the set id (invalidates existing states).
4. **Sidecar/slots** (plan section 8) are a separate file; a replay does not write sidecars (`driver.rs:427`).
5. **Not in the replay:** the packs' bytes (a replay names the set; the loader finds packs by id and checks the CRC), redirector state (derived), the HD overlay.
6. **HD layer** gets the original ROM, not the image: `Hd::new(rom: &[u8], ...)` (`hd.rs:122`, `app.rs:308`) and the catalogue extract art from it. Pack art goes through its own overlay (Task 12).

### Identity runs and `--mods-identity`

- SEEN `verify-replays`: `Args { hooks, dir, cheats_identity: bool }` and `parse_args` (`xtask/src/verify_replays.rs:35-70`); `config(symbols, hooks, cheats_identity)` (l.82) installs `ssd_game::cheats::install(&mut table)` into B's table (l.94-98); `main.rs:256` passes it. `standalone-diff`: `args.contains("--cheats-identity")` with a flag allow-list (`main.rs:945-952`, usage l.1217) and a `cheats: bool` parameter of `standalone_diff::run` (l.56-67) that does the same install. Both build A and B with `replay.power_on(rom.to_vec())` (`lockstep.rs:339`, `standalone_diff.rs:69-70`).
- `--mods-identity` mirrors it: a `mods_identity: bool` next to `cheats_identity` in `Args`/`config`/`parse_args` (an `Identity { cheats, mods }` struct avoids a 4th positional bool), the flag in the allow-list and usage text, and `ssd_game::mods::install(&mut table, <empty overlays>)` after the cheat install. To prove the cartridge too, B should run on the **extended image**: `Replay::power_on_image(rom, ssd_mod::image::build(rom, &[]))` (zero-padded 2 MiB). That needs `LockstepConfig` (lockstep.rs) to carry an optional `image` used for B only (A stays the 1 MiB original); `standalone-diff` builds B itself, so it takes the image directly. I did not read `compare` to confirm it ignores the ROM (INFERRED it compares machine state). Both flags must combine (`--cheats-identity --mods-identity` = the shipped Enhanced). A neutral set writes no section, so the replays themselves stay as they are.
- `rebase-replays` (`xtask/src/rebase_replays.rs`) takes only paths and `--check`; no flag needed, works for format 3 once `from_bytes/to_bytes` handle it. With "no core patch" M10 never needs it.
- Reminder (the repository's verification rules): the neutral identity is Classic-reaching: quick set `battle menus landing` per task, full set once per milestone.

## Proposed changes to the plan

Task 1 (`ssd-mod` core):
- Replace "`Machine` takes the built image instead of `rom`" by: `Machine::with_image(original_rom, image, saves, seed)`; `rom_crc32` always the original's; `Machine::image_crc32`; `Replay::power_on_image`; state header key = image CRC when set (no format bump).
- `image::build` returns a power-of-two vector (2 MiB first, 4 MiB only if the budget needs it; LoROM ceiling is bank `$7D`); allocator range "banks `$20-$3F` = 1 MiB; `$40-$7D` only with a 4 MiB image".
- Replay format 3 with a packs section, written only for a non-empty set; tests: "no packs -> same bytes as before" (the 30 replays re-serialize unchanged) and "wrong set -> named error".
- `Ruleset` is `Copy` + `const`: keep the pack set outside it (an `Arc<PackSet>` beside it) or put only a `Copy` handle in it.
- Drop "a core patch only if research 1 says so": research 1 says no patch, so `PATCHES.md` and a `rebase-replays` run leave Task 1. Keep the acceptance: a random-padded 2 MiB image plays `battle menus landing` identically (already true, command above).

Task 2 (redirectors):
- The identity gate gets its flag as above, and B runs on the extended image so it also covers the cartridge.
- Redirectors read pack data through `Ctx::read8` at `$20+` addresses (SEEN it goes through the core bus), so they work in recompiled and original execution without a separate path.

## Open questions

1. Is "3 replays x 3 images" enough for the go, or does the owner want the 30 replays once on a random-padded 2 MiB image (about 4 min per 20k frames per image run here, so roughly 40-60 min for the set, a separate run)? The scratch tool only needs a replay list.
2. States: key = image CRC without the set id (no format bump), or format 4 with the set id?
3. Replay format 3 (proposed) needs the oracle's replay tests extended; it is Classic-reaching under rule 8 (quick set applies).
4. Is 4 MiB needed at all? 1 MiB of extension (`$20-$3F`) may suffice for text/scripts/maps but not for much new art; sizing belongs to Task 1's budget.
5. Not verified: the recompiled build and the save-state round trip on an extended machine (cheap to add to Task 1's tests).
