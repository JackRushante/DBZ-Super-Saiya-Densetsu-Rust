# Battle poses: how a unit's sprites are found in the ROM

Read by `xtask hd-catalogue extract` (`xtask/src/hd_catalogue/decode.rs`), which decodes every pose of every unit
without running the game. The decoding was checked against the keys of the HD art made from poses of real frames
(`ssd_hd::resolve::pose_key`): every pose of every poseset gives the key of the art that was made for it.

## The chain

1. **Unit table**, `1E:CB06`, four bytes per unit id (0 to 104): the 24-bit address of the unit's 32-byte object
   palette (sixteen colours of 15 bits, uploaded unchanged), then the unit's **poseset** (0 to 32). Units that share
   a poseset are the same sprite in other colours: the three Saibamen, Frieza's soldiers, Vegeta and Super Saiyan Vegeta.
   Units 84 to 95 have no palette (they are not drawn).
2. **Poseset table**, `15:8060`, six bytes per poseset: a 24-bit pointer to the poseset's table of pose graphics and a
   24-bit pointer to its table of pose layouts. The layout table is a list of 16-bit pointers (in the layout bank), one
   per pose; the poses are counted until the table reaches the first layout it points at.
3. **Pose graphics**: three bytes per pose (word, then bank). The word leads either to one compressed stream (its first
   word is a size under `$8000`) or to a list of stream words ended by zero. The streams decode back to back with the
   graphics decoder (`ssd_catalogue::graphics::lz_decode`) and are regrouped from bitplane pairs to the SNES tile layout
   (`regroup`); tile `n` is bytes `32 n` to `32 n + 31`, four bits a pixel.
4. **Pose layout**, a small bytecode expanded by the routine at `05:BF77` into the object tables in work RAM and drawn
   by `05:9626`. `$80` ends the layout; `$81` sets the attribute `$70` (horizontal flip) for what follows, `$84` sets
   `$30`, `$83` makes every entry carry its own attribute byte; `$82 n` is a run of entries for tiles `n` down to 0 that
   have only a position (`y`, `x`, and an attribute byte in the per-entry mode); any other first byte is `y`, followed by the
   tile, the attribute byte (per-entry mode only) and `x`. Every sprite is 8 x 8. Positions are signed bytes.
5. **Drawing**: the game flips whole poses with the hardware's flip bits (`y' = $F8 - y`, `x' = $F8 - x` and the
   attribute's flip bits toggled), so a pose seen on screen has up to four keys: as decoded, mirrored left to right,
   upside down, and both. The renderer looks art up by the key and by the mirror key; art made from a pose seen upside down
   has the upside-down key.

The command screen draws five enemy slots with a fixed pose (23); its cell grid (`02:A597`, per unit) is the layout of
that pose.

## Which poses are used

A pose number reaches the side record's byte `+$21` (bit 7 asks for a load) from a few places in bank 5, and the
scripts that feed it are not tied to a unit, so a pose number is used for every poseset that has it.

- **Battle script** (`05:CA8D`, the opcode table at `05:CAFA`, 133 handlers): each instruction is an opcode and its
  arguments, and the handler advances the script position by the argument count it loads before its jump back (`05:DD51`
  adds the count and one). The scripts are in bank `$1E`: two tables of 125 pointers, indexed by the animation id, one
  for each side, and three blocks the follow modes use. Opcodes 3 (pose from the first argument), 49 (pose from the
  second argument, when the record's first byte matches) and 51 (pose from the second argument; `$25` becomes `$27` in
  one state) set a pose. The branch instructions (jump by an offset, jump by the hit class, conditional skips) are
  followed to every forward offset in their arguments, so the set is an upper bound.
- **Effect script** (`05:DEC3`, bank `$1D`, two tables of pointers at `1D:908C` and `1D:9183`): opcode 3 sets a pose from
  its argument, 1 and `$22` take one argument, 0 ends the script.
- **Fixed routines**: the idle pair (3 and 4, picked at random), the resets to 0 and 2, 5 for the opening of a fight,
  and 23 for the command screen.

The union is every pose number from 0 to 39 but 22. A log of the pose loads of the reference and secret replays (328 pairs
of unit and pose) is inside it: pose 18 is set only by an effect script; poses 36 and 39 only by scripts' opcodes 3 and 51 on branches.

## Exceptions

- Pose 5 of some posesets is a dissolve whose frame is not a static pose.
- 63 poses of the posesets name a tile that their graphics do not hold (the tile draws blank); their key is not the
  game's, and the catalogue leaves them out of the check.
- Poseset 19 pose 2: the stream does not decode. Poseset 2 pose 27 has no opaque pixel.
- The renderer keys a fighter over its whole palette group and, when the pieces are more than 12 pixels apart, over each
  cluster of at least 40 opaque pixels as well; the catalogue lists the cluster keys of the poses that fall apart.

## Portraits

Face portraits (`02:A3B4`, three bytes per unit id, 105 rows) and object portraits (`02:A4EF`, 40 rows) lead to
compressed sets of 32 tiles that hold two 32 x 32 pictures (halves); see `menus.md`. A set is shared by several units
and objects, each showing one half, so the HD portrait art is found by the set and the half.
