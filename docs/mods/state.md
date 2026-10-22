# The mod state: wide BP and the extended flags

A data pack can keep state that the save image does not hold: a set of extended flags for its scripts, and a battle power
(BP) wider than the game's 24 bits. Both live in work RAM outside the save image. Each save slot keeps its mod state in
a sidecar file beside the saves, `ssd.mods`, and a replay carries it as an application event. The game's own data does
not change: with no state, every route below is the original's.

## The extended flags

- 2048 flags in 256 bytes at `$7F:FD00`. They are valid only with the marker `SSDf` at `$7F:FCFC`. Without the marker
  every flag reads clear, and the first set writes the marker and clears the bytes.
- The scene language sets, clears and tests them with `flagx.set <n>`, `flagx.clear <n>` and `flagx.if <n>, <label>`
  (opcodes `$33` to `$35`). `flagx.if` jumps when the flag is set.

## The mod state block

The block is 256 bytes at `$7F:FB00`, valid only with the marker `SSDm` at `$7F:FAFC`. Without the marker the whole
block reads as zero, whatever its bytes are, because work RAM starts with random fill.

| Offset | Holds |
|---|---|
| `+00` to `+11` | the high 16 bits of the wide BP of the 9 party records (2 bytes each; party slot `i` at `+2i`) |
| `+12` to `+1B` | the same for the 5 enemy records (enemy slot `j` at `+12 + 2j`) |
| `+1C` to `+1F` | the same for the side records `$7E:1400` (side A) and `$7E:1440` (side B) |
| `+20` to `+37` | the high 16 bits of the BP snapshot `$7E:4C00 + 3 * id`, for characters 0 to 11 (2 bytes each) |
| `+40` to `+49` | scratch of the battle hooks, zeroed when a state is applied; no save keeps it |
| `+4A` to `+FF` | free, zero |

The extended flags are not copied into the block: they already sit at `$7F:FD00`, and the sidecar keeps them.

## Wide BP

A battle power is 40 bits. The record keeps the low 24 bits at `+07..+09`, which every routine reads as before; the high
16 bits are in the block. A record without a high part is an ordinary 24-bit BP.

The routines that keep the high part consistent are:

- **Damage** (`05:B804` to `05:B8DC`): when a side has a high part, both sides are shifted right by one common amount until
  the larger, with its class bonus, fits 24 bits. The body computes the damage from the shifted pair, which keeps the ratio
  up to rounding, and the original low parts go back into the records after the last read of the BP.
- **Snapshot** (`01:FE4B` and `01:FE7E`): the high parts are saved with the 24-bit snapshot and restored with it.
- **Side copies**: the high part follows a record's copy to and from a battle side (`05:AE81`, `05:AE16`, `05:9D5B`
  and `05:9D79`, through their ends).
- **Kaioken boost** (`05:AB60`): the boost is computed in 40 bits, `BP + BP * m / 2`, and the pre-boost high part is kept
  for the two restores: the one at the end of the boost by KI (`05:9D3D`) and the stop (`01:9FE0`).

Not yet kept consistent (their BP writes still wrap at 24 bits): the form changes (`05:A54A`, `05:A622`), the power-up
(`05:A727`), the level-up add (`03:855E`), the form builder (`03:862F`), and the BP objects and stat objects of the item
menus (`01:DF3E`, `01:E4AE`, `01:E568`).

The readers that show a BP:

- The Enhanced Status screen does not yet show the full value: it still shows the low 24 bits.
- The battle HUD digits (`1D:8479` to `1D:851B`) are the game's tiles and convert the 24-bit BP of each side, so a wide
  BP shows its low 24 bits there. This was not checked on a run.

No pack row can give a wide BP yet: the pack's `bp` field is 24 bits (0 to 16777215). A wide BP comes from the state.

## The sidecar `ssd.mods`

- One file beside the saves, per save slot. Each entry is keyed by the slot and by the slot's checksum word, as the cheats'
  sidecar is (`ssd.cheats`): a save whose checksum changed has no entry.
- The file's first field is its version (1). It is encoded with bincode, little endian, as the cheats file is.
- An entry keeps the 256 bytes of the extended flags when their marker was in work RAM at the save, and the 56 bytes of
  the block's high parts when the block's marker was there. A save with neither keeps no entry.
- A missing file is an empty sidecar: a save made before the file existed loads with no state. A file of another version,
  or damaged bytes, is refused.

## Replays

The state of a run is carried by the application event with tag `4` (the next free one after the bag's `1`, the cheats'
`2` and the spawner's `3`). Its payload is the state's own bytes. Playing the event writes the state into work RAM as the
run did, so a replay of a loaded save plays with the same flags and high parts.

## Limits

- The damage shift keeps the larger side within 24 bits with its class bonus. A side much smaller than the other shifts
  to zero, and its damage then rounds to the limit of the damage word (`0xFFFFFF`).
- The wide BP is set only through the state in this part of the work. A pack row field for a wide BP is not defined.
- Work RAM outside the marked ranges is not touched: the reference replays never write `$7F:FAFC` to `$7F:FBFF` or
  `$7F:FCFC` to `$7F:FDFF`, which the replay-write scan checks.
