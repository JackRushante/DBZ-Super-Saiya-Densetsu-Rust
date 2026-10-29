# The mod state: wide BP, wide levels and the extended flags

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
| `+4A` to `+4F` | free, zero |
| `+50` to `+55` | the kind of the battle (`enemy_scaling`): the marker `SSDe`, the pending mark and the kind |
| `+58` to `+5D` | the member pick of `party_boosts` (the HD driver): the marker `SSDq`, the object and the row; not saved |
| `+60` to `+87` | the wide levels (`free_progression`'s `level_cap`): the marker `SSDl`, then 12 levels of 16 bits (character `id` at `+64 + 2id`), then the EXP's bits 24 to 31 of the 12 characters (`+7C + id`). Its own marker, like the kind's, valid whether or not the block is live |
| `+88` to `+FF` | free, zero |

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

- The Enhanced Status screen shows the full value (24 bits of the record and the high part of the block, `MenuProbe`), and the level
  from the wide level.
- The battle HUD digits (`1D:8479` to `1D:851B`) are the game's tiles and convert the 24-bit BP of each side, so a wide
  BP shows its low 24 bits there. This was not checked on a run.

No pack row can give a wide BP yet: the pack's `bp` field is 24 bits (0 to 16777215). A wide BP comes from the state.

## Wide levels

A level above 99 does not fit the record's byte (`+02`), which every routine of the game reads. With the pack parameter `level_cap`
(`free-progression.md`, "The level cap") a character's level is a 16-bit **wide level** kept in the block, the record's byte saturates
at 99, and the EXP, which is 24 bits in the record, has its bits 24 to 31 in the block. The wide level counts only while
`min(wide, 99)` equals the record's byte. The BP of such a character is the wide BP above. The hooks of the level-up check and of the
raise (`ssd_game::mods::levels`) keep the three consistent; the readers that show a level read the wide one where they have it.

## The sidecar `ssd.mods`

- One file beside the saves, per save slot. Each entry is keyed by the slot and by the slot's checksum word, as the cheats'
  sidecar is (`ssd.cheats`): a save whose checksum changed has no entry.
- The file's first field is its version (2: the wide levels; a version 1 file is read, with no levels). It is encoded with bincode, little endian, as the cheats file is.
- An entry keeps the 256 bytes of the extended flags when their marker was in work RAM at the save, the 56 bytes of
  the block's high parts when the block's marker was there, and the 36 bytes of the wide levels (`+64` to `+87`) when theirs was. A save
  with none keeps no entry. A replay's state event with only the first two parts is read as before.
- A missing file is an empty sidecar: a save made before the file existed loads with no state. A file of another version,
  or damaged bytes, is refused.

## Replays

The state of a run is carried by the application event with tag `4` (the next free one after the bag's `1`, the cheats'
`2` and the spawner's `3`). Its payload is the state's own bytes. Playing the event writes the state into work RAM as the
run did, so a replay of a loaded save plays with the same flags and high parts.

## The save editor

The save editor (title screen, `docs/re/data/save-format.md`, "What the save editor writes") sets a BP of at most 9 999 999. When it edits
the level or the BP of a member it drops that member's high part (the party slot's, and the snapshot's of its character) from the slot's
entry of `ssd.mods`, and re-keys the entry to the slot's new checksum word with the rest of the state. With a pack whose `level_cap` is
set it sets a BP of up to 9 999 999 999, and writes the member's wide BP high part (party slot), wide level and EXP high byte
(`levels`) in place of dropping them.

## Limits

- The damage shift keeps the larger side within 24 bits with its class bonus. A side much smaller than the other shifts
  to zero, and its damage then rounds to the limit of the damage word (`0xFFFFFF`).
- The wide BP is set only through the state in this part of the work. A pack row field for a wide BP is not defined.
- The save editor reads and writes the wide levels only with a pack whose `level_cap` is set (`free-progression.md`, "The displays").
- Work RAM outside the marked ranges is not touched: the reference replays never write `$7F:FAFC` to `$7F:FBFF` or
  `$7F:FCFC` to `$7F:FDFF`, which the replay-write scan checks.
