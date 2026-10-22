# M10 research item 2: the redirection catalogue

Scope: every data family a pack must reach (scripts, messages, strings, names, unit stats, commands, level rows,
encounters, map objects, portraits, palettes, tile sets). For each: the reader routine(s) and entry addresses, pointer
width and bank source, the ids the code tolerates, the callers, a class and the hook point a redirector would use.

Tags: **SEEN** = read in the code listing (`xtask disasm`) or counted by a ROM scan (`.work/m10-research/scan.py`,
`refs.py`, `rom.py`; long-address and absolute-operand scans of the ROM), or taken from a doc in the repo that is
cited; **INFERRED** = my conclusion from what was seen. Nothing here was run in a machine: no hook was installed and no
id was patched (I could not verify any hook point by execution). Addresses are `bank:addr` (physical). Widths are the
M/X state at the address, read from the REP/SEP in the listing (SEEN) unless marked.

Classes: **A** = one reader routine (or one pointer load) with a clean place for a wrapper; **B** = a few readers
(2 to 9 sites), each clean; **C** = data read inline from many places or from immediates in code.

## 0. The hook technique this catalogue assumes (SEEN in `crates/ssd-game/src/cheats.rs`)

- A wrapper may be installed at any instruction address that the PC reaches, not only at a routine entry: the cheats
  hook `03:F320` (a branch target inside `03:F300`) and `01:D7F9` (a jump target) (SEEN, `cheats.rs` consts and
  `hook_entries`).
- A wrapper that has nothing to do writes nothing and calls `ctx.resume_at(own entry)`; a wrapper that wants to change
  something writes the registers or work RAM and then also calls `ctx.resume_at(own entry)`, so the original instruction
  at that address runs with the new values (SEEN: `damage_wrapper` writes `$14CF..` then `resume_at(DAMAGE_ENTRY)`,
  `cheats.rs` around line 400-414). `Ctx::resume_at(other address)` is also allowed ("the original code goes on at the
  24-bit address with the registers this routine left", `ssd-oracle/src/ctx.rs:279`), and `set_a/set_x/set_y/set_dp`,
  `write_dp8/16`, `read_db*`, `read8/16` exist (SEEN, `ctx.rs`).
- So the **post-load patch** is the pattern for every pointer family below: hook the instruction right after the original
  loaded the pointer from the ROM table (the pointer is in A or in DP), replace it by the pack's pointer (and the bank
  byte, which the original keeps in a DP byte or hard-codes), `resume_at` the same address. No Rust re-implementation of
  the reader and no `call_original` are needed; the cost is the wrapper's own (zero declared, like the cheats).
  INFERRED: the declared cost / timing-window behaviour of a mid-routine entry that is not a routine entry was not tested.
- `HookTable::insert` replaces any entry at the same key (SEEN, `cheats.rs` doc of `hook_entries`: "They replace any hook
  at the same address"). The cheat wrappers own `01:F692`, `01:F719`, `02:D34A`, `05:BDDC`, `1D:8C33`, `01:C639`,
  `02:D289`, `01:C257` etc. (SEEN, `cheats.rs:134-157`). **A redirector must not use those entry keys**: all hook points
  below that touch the encounter group, enemy stats and zone-pick routines are therefore inner addresses
  (`01:F6B5`, `02:D379`...), not the entries the cheats took. If a redirector and a cheat ever need the same address, the
  kit has to chain them in one function.
- The recompiler: generated code calls callees directly, so every hook key must also be a stop of the generator, like
  `cheats::entry_points()` (SEEN, `cheats.rs` doc of `entry_points`). Mid-routine keys have to be listed there too
  (INFERRED: `LEVEL_DEADLINE_ENTRY` is not in the list I read, `entry_points` returns every `hook_entries()` key, so it is).

## 1. The catalogue

Pointer width "16 + bank 2" means: the table holds 16-bit words and the reader hard-codes the bank in code (`LDA #$02 / STA $02`),
so a pack object in another bank needs the bank byte overwritten as well as the word.

| # | Family (data) | Reader(s), entry | Pointer / bank source | Id range the code tolerates | Callers | Class | Proposed hook (address, M/X, what) |
|---|---|---|---|---|---|---|---|
| 1 | **Scripts** `06:D109` (82 words) | start-scene command handler `03:ACB2` (RTL), command-table slots 0 and 50 both point at it; the bytes are then read through `[$A5],Y` by the command handlers (`03:AC21`, ...) | 16-bit word, bank 6 hard-coded: `STA $00A7` of 6 at `03:ACB6`; the word goes to `$A5` at `03:ACCA` | `scene_id` (`$01D8`) is a byte, `ASL` in 16 bits: 0 to 255; no bound check, ids 82+ read bank 6 past the table | `$01D8` has 23 writers (`01:853C`..`04:D8D5`); the handler runs from the script dispatcher `03:AC41` (`JMP ($AC4C,X)`) | **A** | **`03:ACCA`** (`STA $A5`), M16 X16: A = script word. If scene id (`peek $01D8`) has an overlay: `set_a(ext offset)`, `write8($A7, ext bank)`, `resume_at(03:ACCA)`. Jumps/calls in scripts are absolute 16-bit addresses; `call` stores `addr - $A5` (`03:B2AB`), so a script in any bank only needs addresses of its own bank (SEEN `03:B29E`, `03:B044`). `$A7` is written only at `03:ACB6` (also `09:95FA`, `1D:E647`, `09:AA0D` are other banks' code, not scene code) |
| 2 | **Message lists** `07:8000` (368 words) | `01:D3C0` (RTL, M8 X16) computes `$00..$02` and `Y`; callers `01:D37A` (next word) and `01:D3DD` (window of word 0) | the table address is `message_lists` `$0738` (24-bit, `$073A` bank), set by `00:8078`, `05:97B2` (to `07:8000`) and `05:9391` (battle, `07:D79E`); each table word is a 16-bit offset in the same bank | `message_number` `$072C` is a word, `ASL` 16-bit: 0 to 32767, no bound check; started by `00:89AC` (`STY $072C`) | `00:89AC` has about 100 JSL callers; `01:D4ED`, `01:B205`, `01:99CD` set the bytes themselves (`text-engine.md`) | **A** | **`01:D3D5`** (the `LDA $072A` after `LDA [$00] / STA $00`), M16 X16: if `peek $072C` has an overlay, `write_dp16($00, ext list offset)`, `write_dp8($02, ext bank)`, `resume_at(01:D3D5)`. The whole table can instead be swapped by writing `$0738/$073A` (class A for a full replacement) |
| 3 | **String tables** (8 tables, `07:8F16`, 3 bytes each: word + bank) | `01:CF3A` (RTL) draws one tile and recomputes the string address every call; callers `01:CCC4`, `01:CDAE`, `01:CE6D` | 24-bit table pointer from `07:8F16 + 3*message_table`; string offset = 16-bit word at `[table + 2*message_string]`, in the table's bank (`01:CF48`, `CF52`, `CF5F`) | `message_table` 0 to 7 (index x3, 8-bit); `message_string` is a word, no bound check | the three callers above; `01:CF3A` runs once per written tile | **A** (table pointer is 24-bit, so a pack table anywhere works); **A/B** for ids past a table's end | **`01:CF61`** (`STA $00` after `LDA [$00],Y`), M16 X16: A = string offset, `$02` = table bank, key (`$0733`, `$0723`). Overlay: `set_a(ext offset)`, `write_dp8($02, ext bank)`, `resume_at(01:CF61)`. It runs per tile (`01:CF3A`), so the lookup must be a hash probe. Strings must be read from `$02:$00..` for the rest of the string: `01:CFB4` uses `[$00],Y` with the same `$00..$02` (SEEN `01:CFB4`) |
| 4 | **Name slots in messages** (`FB/FA/F9` codes; `$1640..$1645`) | `01:D0EE`, `01:D0FC`, `01:D10A` (one per slot) merge into `01:D115`..`01:D15B`; type bits pick the table (bit 7 units `02:ACE8`, 6 objects `02:B0D9`, 5 attacks `02:AEA6`, 4 the table `02:B204`) | 16-bit word, bank 2 hard-coded (`LDA #2 / STA $02` at `01:D12A`, `D13A`, `D145`, `D152`) | index byte `ASL` in 8 bits then `TAY`: ids 0 to 127 (the carry out of the `ASL` is lost); the tables hold 105 / 40 / 69 | `01:D0A1` (the `FB/FA/F9` handler of the string engine) | **B** (4 tables, one common tail) | **`01:D15F`** (`STA $00` after `LDA [$00],Y` at `01:D15D`), M16 X16: Y = 2*id; the table is known from `$02`/the branch, so key the overlay by the slot's type (`$1641/1643/1645` bits) and id (`Y/2`): `set_a`, `write_dp8($02, bank)`, `resume_at(01:D15F)`. Ids are limited to 127 by the `ASL` of an 8-bit A (INFERRED consequence of `01:D0F4`: `ASL A` with M8 loses bit 7) |
| 5 | **String words** `02:AB66` (44: 12 character names, menu words) | `01:BC09` (X from `$00`) and `01:BC0B` (X from caller), RTL | 16-bit word, bank 2 hard-coded (`01:BC0D`); copy loop is inline in the routine | `$03` is a byte, `STZ $04`, `ASL` 16-bit: 0 to 255 | 9 JSL callers of `BC09` (`01:95AB`...`01:AB2B`), 34 of `BC0B` (`01:A643`..`01:D5E5`) | **B** | **`01:BC1E`** (`STA $00` after `LDA [$00],Y` at `BC1C`), M16 X16, Y = 2*id: overlay `set_a(ext)`, `write_dp8($02, bank)`, `resume_at(01:BC1E)` |
| 6 | **Unit names** `02:ACE8` (105) | `01:BC46` (id from the enemy slot `$0E1C,Y`), `01:BC49` (id already in `$03`; caller `03:A44A`), both RTL, and the slot path of row 4 | 16-bit word, bank 2 (`LDA #2 / STA $02` at `01:BC4D`) | slot byte 0 to 255 | `BC46`: `01:AE76`, `01:AF52`; `BC49`: `03:A44A` | **B** | **`01:BC5E`** (`STA $00` after `LDA [$00],Y` at `BC5C`), M16 X16 |
| 7 | **Attack names** `02:AEA6` (69) | `03:86F3` (RTL, A = attack id; 0 is replaced by `$44`) | 16-bit word, bank 2 (`03:86FD`) | A is 8-bit (`STA $03`, `STZ $04`, `ASL`): 0 to 255 | `03:873A` (menu loop `03:871A`), `01:B165` | **B** | **`03:870E`** (`STA $00` after `LDA [$00],Y` at `03:870C`), M16 X16 |
| 8 | **Object names** `02:B0D9` (40) | `01:C515` (X from `$00`) and `01:C517`, RTL | 16-bit word, bank 2 (`01:C51B`) | 0 to 255 | `01:AB88`, `01:B2E9`, `01:C4F1` | **B** | **`01:C52C`** (`STA $00` after `LDA [$00],Y` at `C52A`), M16 X16 |
| 9 | **Unit stats** `02:B214` (105 x 14 bytes) | no single reader. Row index is computed with the hardware multiplier (`id * 14`, `$4202/$4203`) and the fields read as `LDA $02B21x,X`: **`01:F719`** (enemy battle records), **`00:8A69`** (`00:8A90`..`8AC9`: characters 8+ except 11 get HP/KI/BP/class from row `id + 26`), **`01:C0FD`** (class refresh), **`03:BCF2`** (record from a row; callers `03:BCB7`, `03:BCC9`), **`03:8660`** and **`03:B248`** (defence factor), **`05:94A1`** (class by id), **`06:E8EE`** (defence factor + class at battle start), **`1D:8C33`** (EXP award, reads `$02B21F`) | long address with bank 2 in every `BF`; the id is a byte | id 0 to 255, no bound check (rows past 105 run into the command-pointer table: `B214 + 105*14 = B7D2`, SEEN by arithmetic) | `01:F719`: `00:961E`, `01:F4A6`, `03:C7FB`, `05:9856`; `06:E8EE`: `03:C57F`, `03:CFEF`, `03:D1BA`, `03:D240`, `06:E85C`; `05:94A1`: `01:F822`, `05:A059`, `05:A0C9`, `1D:8B6D`; `01:C0FD`: `02:F400`, `03:B233`; ROM scan of `BF xx xx 02` with operand in `B214..B7D1` found these 9 routines (plus noise in banks 0C, 10, 1F that is data) | **B/C** (9 readers, all inline `LDA long,X`; the bank is fixed so rows cannot live in an extended bank) | No pointer to patch (long address, bank 2, X from the multiplier), so no post-load patch. Wrap the record builders and fix the records from the pack row; entry keys `01:F719` (cheats' `POWER_ENTRY`) and `01:F692` are taken, so use inner addresses. See note N5 |
| 10 | **Command lists** `02:B7D2` (105 words) -> `02:B8A4..` | `03:8752` (RTL): A = unit id, leaves `$1A..$1C` = pointer to the unit's list | 16-bit word, bank 2 (`LDA #$02 / STA $1C`, `03:8756`); `LDA #$B7D2`, `LDA [$1A],Y`, `STA $1A` at `03:8765..8767` | unit id byte: `STA $18`, `STZ $19`, `ASL`: 0 to 255 | 6 JSL callers: `03:871A`, `03:8780` (in `03:876C`), `03:87D7`, `03:90CC`, `03:911F`, `05:BBB2` (in `05:BBAF`) | **A** | **`03:8769`** (`SEP #$20` after `STA $1A`; `$1A/$1B` = list offset, `$1C` = bank), M16 X16: unit id = `peek_dp8($18) / 2`; overlay: `write_dp16($1A, ext offset)`, `write_dp8($1C, ext bank)`, `resume_at(03:8769)`. Every consumer then reads the pack's list through `[$1A]`, `[$1A],Y` |
| 11 | **Command list consumers** (how a list entry becomes a move) | `03:876C` (menu cursor `$0D62`+1 -> command id, writes record `+12` and `+1F`; callers `01:938E`, `01:9ECB`, `01:B102`), `03:87D6/87EE` (affordable command search), `03:8808` (byte at `list[list[0] + $0D83 + 1]`), `03:881D` (byte at `list[2*list[0] + 2*$0D83 + 1]`), `05:BB52`, `05:BB69` (attack factor, via `05:BBAF`) | the pointer of row 10 | command id byte 0 to 255 | `03:8808`: `03:89A4`, `03:89E3`; `03:881D`: `03:89AB`, `03:89EA`; `05:BB52`: `05:B63D`; `05:BB69`: `05:B82A` | follows row 10 | nothing extra: all start from `03:8752` (SEEN: every JSL to `03:8752` listed in row 10) |
| 12 | **Command cost table** `03:884B` (69 bytes, indexed by command/attack id) | one read site: `03:87CC` (`LDA $03884B,X`) in `03:87C5` (RTL, inlined at the tail of `03:876C`), callers `03:87EE`, `03:8E6D`, `03:8E9A`, `03:8EE7` | long address, bank 3 | id 0 to 255, no bound check (entries 66 to 68 are zero) | as listed | **A** | **`03:87D0`** (`STA $00` after the load), M8 X16: A = cost, key `peek_dp8($10)`: overlay `set_al(cost)`, `resume_at(03:87D0)`. (The KI check that compares it with the fighter's KI is `01:9ECB..9EE8` for the party: it adds 10 to the cost for character 0 when `$0C36` bit 7 is set. SEEN `01:9ED2-9EE1`; item 3 owns that.) |
| 13 | **Level rows** `02:C13B` (13 words) -> `02:C155..` (8 tables of 48 x 11 + Dende's) | `00:8A28` (RTL, Y = record offset in, `$00..$02` and Y = `(level-1)*11` out) | 16-bit word, bank 2 (`00:8A28/8A2A`); pointer from `C13B + 2*record(+1C)` (`00:8A36`) | character id byte (`+1C`) 0 to 255; level byte: `(level-1)` x 11 through the multiplier, so levels up to 256 read rows (SEEN); character 7 is forced to level >= `$11` (`00:8A46-8A53`, also `03:84B2-84BF`) | 6 JSL callers: `00:89B8` (build), `00:8A07` (level-up gain), `01:A8C0` (reads +14 of the row), `01:E42D`, `03:8597`, `06:E91F` | **A** | **`00:8A3F`** (`TYA`, after `STA $00` at `8A3D`), M16 X16: `$00` = table pointer, `Y` = record offset; key `peek $0B1C + Y` (character id): overlay `write_dp16($00, ext table offset)`, `write_dp8($02, ext bank)`, `resume_at(00:8A3F)`. The ext table has the same 11-byte rows, any length up to level 256 |
| 14 | **Characters 8+ override** (stats from the unit row `id + 26`) | `00:8A69` (RTL, called at the end of `00:89B8`; `JSL 00:8A01`) | unit row (see row 9) | char id 8 to 255 except 11 (`CMP #$0B / BEQ`, `CMP #$08 / BCC`) | `00:8A01` | **A** | entry **`00:8A69`**, M8 X16 (starts with `REP #$20`): for a pack character id (>= 12 or an overlay) return at once (RTL), because the override would read unit row `id+26`; or write the pack's stats there (INFERRED) |
| 15 | **Character -> unit id** `00:8AE5` (12 bytes) | `00:8AD6` (RTL) | long address bank 0 (`LDA $008AE5,X`) | character id byte | `02:F3FC`, `03:B22F` | **A** | **`00:8AE1`** (`STA $0B01,Y` after the load), M8 X16: `set_al(unit id)`; key `peek $0B1C + Y` |
| 16 | **Level caps** `03:850E` (12 bytes: 47/48 per character, Dende 3) | `03:84EE` (RTL, returns carry set = may level up), and `03:8476` inside `03:841E` | long address bank 3, id byte | char id 0 to 255 | `03:84EE`: `03:841E`, `03:851C`; `03:841E`: `03:CD8A`, `05:8E51`, `05:8FDD` | **B** (2 reads) | **`03:84FB`** (after `LDA $03850E,X` at `84F7`), M8: A = cap (set Z via `set_nz8`), and **`03:847A`** (`PLX`), M8: A = cap. Key = character id (`X` at `84FB`/`847A` is the id: SEEN `03:84F6`, `03:8475`). No level-48 constant elsewhere in those two routines (SEEN); `03:841E` also excludes characters 8, 9, 10, 12 by compare (`03:8429-8437`) |
| 17 | **Enemy groups** `02:BF0B` (112 x 5 bytes) | `01:F692` (RTL, M8 X16): `group = $0EA3`, `x5` by multiplier, loop reads `LDA $02BF0B,X` at `01:F6B1` | long address bank 2, id byte | group 0 to 255 (255 x 5 = 1275, inside bank 2) | `00:961A`, `01:F4A2`, `02:F094`, `05:9852` | **A** (single reader) | **`01:F6B5`** (`CMP #$FF` after the load; do **not** use `01:F692`: it is the cheats' `GROUP_ENTRY`), M8 X16: A = unit id, `X` = row offset, so group = `peek $0EA3`, slot = `X - 5*group`; overlay `set_al(unit)`, `resume_at(01:F6B5)`. 8-bit group id: at most 255 groups (143 new ones) (INFERRED from the 8-bit multiply input) |
| 18 | **Map -> group table** `02:D6EA` (word per map, then 8 groups per zone, `02:D74A..`) | `02:D34A` (RTL, called from `02:D250`) | 16-bit word, bank 2 (`02:D34C`) | `map_id` byte (SEEN: words at index 48 and up are junk, e.g. `0x100`, `0x200`), zone `(z-1)*8` in 8 bits | `02:D250` only | **A** | pointer: **`02:D35F`** (`STA $00` after `LDA [$00]` at `D35D`), M16 X16: overlay by `peek $01A0`: `write_dp16($00, ext)`, `write_dp8($02, bank)`, `resume_at(02:D35F)`; or the picked group: **`02:D379`** (`STA $0EA3`), M8 (note `02:D34A` is the cheats' `PICK_ENTRY`: inner address, no clash) |
| 19 | **Zone rectangles** `02:D4A4` (word per map) and **zone level** `02:D62A` (4 bytes a map) | `02:D38B` (RTL) inside `02:D37D` (callers `00:8C12`, `03:F169`); level read at `02:D408` | 16-bit word, bank 2 (`02:D38D`) | `map_id < $30` only: `CMP #$30 / BCS` at `02:D395`; so maps 48+ never get zones | `02:D382` | **A/B** | a constant bound (`CMP #$30`, class C for the cap, item 4 owns it). Pointer post-load at **`02:D3A4`** (`STA $00`), M16 X16 |
| 20 | **Map object lists** `02:EAAC` (word per map, ids ended by `$FF`) | `03:93FA` (RTL, called from `00:8C1A`, `03:CE56`, `03:D116`, `03:F36C`) | 16-bit word, bank 2 (`03:940E`) | map id byte; object id byte (0 to 254) | as listed | **A** for the list | **`03:9421`** (`STA $00` after `LDA [$00]` at `941F`), M16 X16: overlay by `peek $01A0` |
| 21 | **Map object records** `02:EB67` (5 bytes/id), hide flags `02:ECCF` / `02:EF23` (1 byte/id), second table `02:EEA2` (3 bytes/id) | records: `03:9468`, `03:948B`, `03:9494` inside `03:93FA`; flags: `03:93D0` (RTL, carry out; called from `03:9434`) | long address bank 2, id byte | id 0 to 254 | `03:93D0`: `03:9434`; `03:9386`: no JSL caller found (entered by a branch) | **B** (3 loads + 2 flag loads + 3 `EEA2` loads) | **`03:946C`** (`STA $10` after the byte load; A = flags/type), **`03:948F`** (`STA $1182,Y`, M16: A = x), **`03:9498`** (`STA $1184,Y`, M16: A = y); key = id at `$1181,Y` (SEEN `03:9457`). Hide flag: **`03:93E5`** (`CMP #$FF` after both loads, M8): A = flag byte |
| 22 | **Portraits** `02:A4EF` (object, 3 bytes each) and `02:A3B4` (face, 3 bytes each), palettes `02:A356` | one routine around `01:C9B0..CA40`: object row `01:C9DB/C9E4`, face row `01:C9FB/CA04` | 24-bit pointer (word + bank byte) read straight into `$0D77/$0D79` | `X = 3*index` | caller graph not traced | **A** (already 24-bit) | **`01:C9DF`** and **`01:C9E8`** (the two stores), or write the pack pointer into `$0D77/$0D79` after `01:C9E8`; same for the face row at `01:C9FF/CA08` |
| 23 | **Palettes** (bank 9, 16 words each; pointer words in bank 2 tables `02:A284`, `02:A356`, `02:A398`) | `01:C885` `load_palette_from_pointer` (in `re/symbols.toml`, so a Rust port exists), reached through four loaders `01:C8AB`, `01:C8C8`, `01:C8E5`, `01:C902` (each: `$04` = index, `$03` = slot) | 16-bit word, **bank 9 forced inside `01:C885`** (`LDA #9 / STA $02`, `01:C885`) | palette index byte `$04`, `ASL` 16-bit | `01:C8C8` has JSL caller `01:F57B`; the other three are not JSL targets in the ROM (reached by other means, not traced) | **B** | For ids past a table or to point at a pack bank: hook the call **`01:C8C3` / `C8E0` / `C8FD` / `C91A`** (the `JSL 01:C885`), M16->M8 (after `SEP` at `C8C1`): copy the 32 bytes yourself to `$0300 + (slot & ~1) * 16` and `resume_at` the instruction after the JSL (`01:C8C7`...), because `01:C885` would reset the bank to 9. To replace an existing palette by content, key `01:C885` entry on the pointer in `$00` (INFERRED) |
| 24 | **Tile sets and every other LZ stream** | decoder `00:C559` (RTL): source = 24-bit pointer `$83` (word) + `$85` (bank); 26 JSL callers; `00:C5ED` variant (4 callers). The pointer is loaded as an **immediate** at 29 `STY $83` and 16 `STA $83` sites, e.g. outdoor tiles `08:8D4C` (`00:8C30-8C37`), town tiles `08:BF20` (`00:8C64-8C6B`), the `02:8470` table of cutscene sets read by `00:8844` | 24-bit | n/a (addresses, not ids) | callers of `00:C559`: `00:8844`, `00:8C72`, `00:8F34`, `00:90CF`, `00:9FE2`, `00:A1B5`, `00:AE2D`, `00:B2D9`, `00:B6A0`, `00:BBE1`, `01:CAA2`, `01:F689`, `03:B357`, `04:875D`... | **C** at the call sites (immediates), **A** at the decoder | **`00:C559` entry** (and `00:C5ED`), key = the 24-bit source in `$83/$85`: overlay `write_dp16($83, ext offset)`, `write_dp8($85, ext bank)`, `resume_at(00:C559)`. Replaces any asset by its original ROM address. INFERRED: needs a check that the decoder does not read `$83/$85` before the entry |
| 25 | **Map header / map data pointer** `06:F2DE` (word per map; `$0704..$0706` = pointer + 2) | map load in `00:8E30..8E6A` (a block inside the map-set-up routine, entry not determined) | 16-bit word, bank 6 (`00:8E47`); `$0704/$0705/$0706` 24-bit; fixed copies `$0707/$0709` set by immediates at `00:8C28/8C2D` (outdoor `09:EFB0`) and `00:8C5C/8C61` (town `08:FA00`) | map id from the stack (`PLA` at `00:8E39`) | not traced | **A/C** (item 4 owns it) | **`00:8E43`..`8E4B`** post-load; see item 4 |

## 2. Notes per family

**N1. Scripts (row 1).** The scene starter is one handler (`03:ACB2`), reachable as command 0 and command 50 of the table
(SEEN: `u16(03:AC4C + 2*i) == ACB2` for i = 0 and 50). The table holds 82 words for ids 0..81; the code does not bound
the id (SEEN, no compare after `LDA $01D8`). The script bytes are fetched through `[$A5],Y` by the handlers
(`03:AC21`, `AD4D`, `AD54`, `AE47`, `AE93`, `AF2B`, `AF39`, `B04F`, `B19C`, `B2A3`, `B3A2`, `B529`, `B551`, `05:AF83`; SEEN scan of
`B7 A5`), so a script in the extended image works as long as `$A7` is its bank and the script's own absolute addresses
are consistent with where it was linked (jump = `LDA [$A5],Y` -> `scene_offset`, which is `addr - $A5` after the
subtraction at `03:B2AB`; for `jump` (`03:B044`/`03:AE8D`) INFERRED the same). Messages started by `E0nn` handlers
(`01:D43B`) start scenes by id and so reach row 1 as well. Tested ids: none (no run).

**N2. Messages (rows 2 to 4).** Three layers, each already 24-bit except where noted:
(a) message number -> list: `01:D3C0` reads `message_lists` (24-bit, `$0738`), so the whole table can be replaced by writing
`$0738/$073A` (this is what `00:8078`, `05:97B2` do); the per-id overlay needs the hook at `01:D3D5`.
(b) list word -> string (table, index): `D0nn` selects the table with `message_table` (8-bit), the string index is a word.
(c) string address: `07:8F16` holds 8 x (word, bank) (SEEN dump `2E 8F 07 | 82 8F 07 | 00 80 06 | 26 92 07 | 74 93 07 | 86 93 07 | 8A 93 07 | 8E 93 07`),
so each table can be moved to any bank by editing that 24-bit entry, and the strings must be in the table's bank
(`[$00],Y` with `$02` the table bank). New strings past a table's end need the per-string hook `01:CF61`.
The battle messages are built in WRAM (`$14E2`, `05:9391`) and use table 1 (`battle_message_pointers`): same path.
Name codes inside a string (`FB FA F9`) go through row 4; number codes (`F8 F7 F6`) copy WRAM bytes (no table).

**N3. Names (rows 4 to 8).** There are five name readers (one per table) plus the slot path; all load a 16-bit word with
the bank hard-coded to 2 and then copy `length byte + tile bytes` with the same helper idea (`01:BC22` for units via slot,
objects `01:C535`, attacks `03:8712`; `string_pointers` and the slot path copy inline). One `LDA [$00],Y` per reader is the
patch point (addresses in the table). The 8-bit ASL in the slot path limits slot ids to 127 (row 4). There is no place
to extend a table in bank 2 itself (the tables are followed by other data: `02:AEA6` follows `02:ACE8`'s strings, etc.
INFERRED from the addresses `ACE8` (105 words, to `ADBA`), `AEA6`, `B0D9`).

**N4. Level rows and characters (rows 13 to 16).** `00:8A28` is the only reader of `02:C13B`/`02:C155` (SEEN: no other
absolute or long reference in the ROM to those addresses). It runs for: building a record at the start of a chapter
(`00:89B8`), each level-up (`00:8A07`, from `03:841E`), the EXP threshold test (`03:8597`, `03:841E` reads the
threshold at `03:8445-8459`), `01:A8C0` (the Status screen, row bytes 14.. ), `01:E42D` (BP gain preview into `$0744`)
and the battle start (`06:E91F`). The level cap is not a constant in code: it is `03:850E[character id]` (SEEN
`BF 0E 85 03` at `03:8476` and `03:84F7`), 47 or 48 for the eight main characters (values `2F 30 2F 30 30 30 30 2F`),
`03` for Dende. So "level cap 100" is a table edit/hook, not class C (this corrects the plan's Section 6 remark "a class C
constant (48)"; SEEN). The multiplier input is `level-1` as a byte, so rows up to level 256 are addressable.
Characters 8 to 10 and 12 are excluded from leveling in `03:841E` by compares (SEEN `03:8429-8437`); a pack character
(id >= 12) would pass those compares and be leveled with `03:850E[id]` = bytes after the table (`00 00 00 03 00 5A DA C2`: 
id 12 -> `00` = no level-up, id 13 -> `5A` is code), so the cap hook is needed for pack characters.

**N5. Unit stats (row 9).** The unit row is read by 9 routines through `LDA $02B21x,X` with X from `id*14`. All use a
long address with bank 2 (SEEN), so a row cannot be redirected by changing X (the address stays in banks 2 and 3).
The ids reach 255 in the code but only 105 rows exist, the 106th "row" is `02:B7D2` (the command pointer table) read as
stats (SEEN by arithmetic), so a pack unit needs either (a) a wrapper after each of the three routines that matter
(`01:F719` enemy records, `00:8A69` characters 8+, `03:BCF2` temporary party record) writing the pack row into the
record, and (b) ignoring the minor readers (class `01:C0FD`, `05:94A1`, `06:E8EE`, `03:8660`, `03:B248`) or hooking
each at its `LDX $4216` (`01:C10B`, `06:E906`, `03:8670`, `03:B255`, `05:94C1`): there X can be set to a bank-2 row that is
a **copy of the pack row**; that works only if a row copy exists in bank 2, which it does not (no free space proven).
Recommendation (INFERRED): treat the unit row as a Rust-side `UnitRow` lookup `(id) -> [u8;14]` and wrap
the record-building routines at their **exit** (or at the `LDX $4216` and replace the following loads by hooking each
`BF ..B2 02` instruction: 8 hook points in `01:F719`, 8 in `00:8A69`, 8 in `03:BCF2`, 1 each in the others). The plan's Task 7
"stats builder `01:F719`" wrapper is enough for enemies; the characters need `00:8A69` and `03:BCF2`.
The cheats' power wrapper (`POWER_ENTRY = 01:F719`) post-multiplies HP and BP after the original; a redirector that fixes the
records must run **before** it. Chaining order matters (INFERRED).

**N6. Commands (rows 10 to 12).** `03:8752` is the single entry for the per-unit list pointer (6 callers, all listed).
Everything else reads the list through `[$1A]` / `[$1A],Y` (SEEN in `03:876C`, `03:87D7`, `03:8808`, `03:881D`, `05:BB52`,
`05:BB69`). The structure of a list (what `list[0]` counts, the groups) is item 3's subject; what matters here: the list
is addressed by a 24-bit pointer (`$1A..$1C`) after the call, so a pack list may live in any bank. The cost table
`03:884B` is read at one place (`03:87CC`). The factor used by the special attack is read from the list itself
(`05:BB69`: byte `2*list[0] + 1`, via `05:BBAF`), so per-unit lists also carry the special-attack factor (SEEN
`05:BB77-BB82`); item 3's note has the detail.

**N7. Encounters (rows 17 to 19).** Group index is a byte; `02:BF0B` is read at one site (`01:F6B1`). The zone logic is
inside `02:D289..D428` (the cheats hook `02:D289` as `ROLL_ENTRY` and `02:D34A` as `PICK_ENTRY`). Maps >= 48 are cut
off in two places: the zone rectangle code (`CMP #$30` at `02:D395`) and the pointer tables `02:D4A4` and `02:D6EA` end
after 48 words (the words after index 47 are `0x800`/`0x300`/`0x100`/`0x200`, SEEN dump; they are not pointers).
A pack map needs: the bound at `02:D395` changed (a flag in the mod state block; the constant lives in code: class C
for that one byte), and the two pointer hooks above.

**N8. Map objects (rows 20, 21).** `03:93FA` is the only loader of the map's object list (4 callers). Per object id: a
5-byte record (type/flags, x word, y word), a hide flag byte and, in a second routine (`03:9386`, no JSL caller found), 3 bytes
of `02:EEA2`. Object ids are bytes, stored as `id + $30` after the load (`03:9483`) for the talk tables
(`03:9DF1`, one word per object id; not catalogued here). The hide flag read is split in two (`02:ECCF` normal,
`02:EF23` when `$01A1` bit 7 is set, `03:93D6`).

**N9. Graphics (rows 22 to 24).** Portraits and faces are already 24-bit pointers in tables, so they are class A by
nature. Palettes are 32 bytes copied from a bank-9 pointer; the bank is set inside `01:C885`, so a pack palette cannot use the
loader unchanged (hook the call, copy yourself). Tile sets and all other pictures are 24-bit immediates in code (45
sites), but they all pass through the LZ decoder `00:C559`/`00:C5ED`, which is the one clean place to redirect a
whole asset by its original address (row 24). The cartridge-level alternative (patching the ROM image's bytes at those
immediates) is code change, which the plan's principle 2 forbids.

## 3. Things I could not verify

- No hook point was exercised in a machine (no headless run with a wrapper). All "M16 X16" widths come from reading
  REP/SEP in the listings; `xtask disasm` marked the traces' widths only for routine entries.
- The callers of the palette loaders `01:C8AB`, `01:C8E5`, `01:C902` and of `01:C9xx` (portrait routine) were not traced
  (no JSL in the ROM; they are presumably reached through jump tables).
- The composition of the unit-row hooks (N5) is a recommendation, not a measured design.
- `03:9386` (map object `02:EEA2` loop) has no JSL/JMP caller found by scan; it may be reached by a `BRA` from a
  neighbour routine or be dead code.
- Counts of "writers of `$01D8`" and of `STY $83` sites are byte-pattern counts; some may be data in code banks.
- Out of scope here and not done: the item tables (attribute bytes, prices `01:CB8B`, effect dispatch), the talk table
  `03:9DF1`, the shop tables, the map header and tile-block tables (research item 4), battle sprite budgets (item 3).

## 4. Scratch used

`.work/m10-research/rom.py` (ROM reader), `scan.py` (JSL/JSR/JMP caller scan), `refs.py` (absolute-operand scan),
`strings.tsv`, `messages.tsv` (xtask text-dump output, derived from the game: scratch only, delete when done).

## 5. Addendum from item 3 (command lists and the other tables indexed by command id)

All SEEN, details in `r3-battle-kaioken.md`, sections 2 and 3.

- **Command list layout** (decoded): `[N]`, then N slot bytes (command id, `$FF` = blank line, slot 0 = plain attack), then N
  attribute bytes, then N words (low byte = attack factor K). A pack list therefore needs the three parallel arrays to
  agree in length; the menu draws slots 1..N-1 and the cursor maps to the N-th non-`$FF` slot byte (`03:876C`). Class A
  with the hook of row 10 (`03:8769`).
- **Row 26, effect / animation id by command** `1D:8E3A`: 65 bytes (`$00..$40`), one read site `05:AB10` (`LDA` long,X into
  `$14E0`); the byte after the table (`1D:8E7B`) is the Kaioken multiplier table, so ids `$41..$44` read it. Class A (one
  site); a pack move with its own effect needs this hook or a Rust-side effect lookup (Task 10).
- **Row 27, Kaioken multiplier** `1D:8E7B`: 38 rows of 8 bytes, read only inside `05:AB60..AB94` (`05:AB94`); class A.
- The command id space is shared by four tables of 69 entries or fewer (names `02:AEA6`, cost `03:884B`, effect `1D:8E3A`
  (65), message/animation selection by `$B7`); a pack move id >= 69 needs all four hooked (rows 7, 12, 26).
