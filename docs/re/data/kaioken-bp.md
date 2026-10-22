# M10 research item 3: Kaioken, Super Saiyan, the special-attack path, BP readers and writers, story gates

Scope (from the plan): the Kaioken (attack id, where it lives, where its effect is applied, cost, duration), the Super
Saiyan state (unit `$3B` and the "Kaioken and Genki Dama are disabled" rule), the special-attack path, how a command
list entry becomes a card or a move, the KI cost check, every reader and writer of record bytes `+07..+09` (BP), the
unused bytes of the 32-byte record, and (Part B) every story check of level, EXP or BP. It feeds the mods "Super Saiyan
Kaioken" and "Free progression".

Tags: **SEEN** = read in a listing (`xtask disasm`, address given) or counted by a ROM scan, or run headless
(`xtask shots`, command given); **INFERRED** = my conclusion from what was seen. **Nothing here ran a Kaioken in a
machine**: no replay reaches command `$33` in a fight, so every statement about Kaioken is SEEN in code, not executed
(the one run is the special-attack bit, section 8). Addresses are `bank:addr`; `$xxxx` are bus addresses in bank 0
unless a bank is given. Widths are those of the listing (M8 X16 unless said). At most 5 instructions are quoted.
Scratch: `.work/m10-research/` (`rom.py`, `scan.py`, `ctx.py`, `bpscan.py`).

## 1. Answers in short

1. **Kaioken is command / attack id `$33` (51), name "KaioKen"** (SEEN: `attack_name_pointers` entry 51 of `02:AEA6`, text
   dump `attack:51`; cost table `03:884B[$33]` = 10; effect table `1D:8E3A[$33]` = `$0E`). It is **neither a card nor a
   story item**: it is one **entry of a unit's command list** (SEEN: `$33` appears in exactly one list, unit `$39`, and
   units `$34`/`$35` in one other, unit `$3A`, see 3). It is used together with a card (the card's first value picks the
   multiplier).
2. **Its effect is a temporary BP boost** applied in the attack set-up routine `05:AB60` (RTS, called from `05:AB1E` when
   the move's effect id is `$0E`): `BP := BP + (BP * m) / 2`, with `m` a byte from the table `1D:8E7B` indexed by Goku's
   level and the card's first value (SEEN code, 4). The number shown on screen is `m` (SEEN: `1C:8044` prints `$01EF`), so
   the real factor is `1 + m/2` (INFERRED from the arithmetic; the maximum `m` is `$14` = 20, a factor of 11).
3. **Cost and duration**: 10 KI to start (table), then **10 KI more for every action while it is on** (SEEN `01:9ED7`
   check, `05:9CDF` payment); **it ends when the KI cannot pay** (`05:9D14..9D3D`), when the player answers "no" at the
   prompt (`01:9FC6..9FEB`), or when a battle starts (`06:E83A`) or ends in a flee (`05:87AB`). The boosted BP is
   thrown away at the start of the victory step (`05:8DDA` restores the BP snapshot, 9).
4. **Goku has four unit ids** (all in id category 0, SEEN `0D:8200`): `$00` plain, `$39` "knows the Kaioken" (set by the
   King Kai conversation: message `$122` ends with handler `E0:24` = `01:D982`), `$3A` "Kaioken is on" (its list has
   `$34` and `$35`, no `$33`), `$3B` Super Saiyan, `$3C` a fourth form (list like Piccolo's).
5. **Super Saiyan** is built by `03:85F0` (unit `$3B`, level 48, BP from the level-48 row = 4 999 999), triggered by
   `05:8772..87B3` and `03:8683`. "Kaioken and Genki Dama disabled" is **not a check**: the SSJ unit's command list has
   no `$33`, `$34`, `$35`, and the builder clears the two state bytes `$0C36`/`$0C37` (SEEN `03:86E4`, `03:86E7`).
6. **The special attack** (bit 3 of `active_side`) is a random roll (`05:BA8B`, bit set at `05:BB46`) that makes the
   attack use the damage routine's entry `05:B7FC` (shift 4, double damage) and the factor of slot 0 of the command
   list (`05:BB69`). SEEN run: `secret-raditz` frame 5144.
7. **A record has no byte that is provably unused** (10); the free place for a wide BP is the mod block, as planned.

## 2. Tables indexed by command / attack id (all 69 ids, `$00..$44`)

| Table | Address | Entries | Use |
|---|---|---|---|
| attack names | `02:AEA6` | 69 words, bank 2 | name text ("KaioKen" `$33`, "KameHameHa" `$34`, "Spirit Bomb" `$35`; many are kana) (SEEN) |
| KI cost | `03:884B` | 69 bytes (read at one place, `03:87CC`) | `$33` = 10, `$34` = 10, `$35` = 20, `$05` KameHameHa = 8, `$01` = 2 (SEEN) |
| effect / animation id | `1D:8E3A` | **65 bytes** (`$00..$40`); entries `$41..$44` read the first bytes of the next table | read at `05:AB10` into `$14E0`: `$33`->`$0E`, `$34`->`$0F`, `$35`->`$10` (SEEN) |
| Kaioken multiplier `m` | `1D:8E7B` | 38 rows of 8 bytes (Goku level 11 to 48 by card value 1 to 8) | `m` from 1 to 20, `$14` at the top (SEEN, 4) |

Command ids `$38..$3E` are *command variants* resolved by the jump table at `03:8A11` (reached from `03:89F2`, used by
the Auto choice and the symbol-5 card), not extra attacks (SEEN, structure only).

## 3. The command list: layout, and how an entry becomes a move

`unit_command_pointers` (`02:B7D2`, 105 words) leads to one list per unit (`02:B8A4..`). The old note says the groups were
"not decoded"; decoded here from the three consumers (`03:871A`, `03:8808`, `03:881D`, all SEEN) and checked on units
`$00`, `$39`, `$3A`, `$3B`:

    [N]                      one byte: the number of slots
    [slot 0 .. slot N-1]     N bytes: a command id per slot ($00 = the plain attack in slot 0; $FF = a blank line)
    [attr 0 .. attr N-1]     N bytes: one byte per slot ($0B1D of the record; $10, $0F, $0E... a move/animation number)
    [factor 0 .. N-1]        N words: little endian, the low byte is the attack factor K ($0B1E; $FF = no damage factor)

- The byte `N` is at offset 0; slot `j` is at offset `1+j`, attr at `1+N+j`, factor at `1+2N+2j` (SEEN: `03:8808` reads
  offset `N + k + 1` and `03:881D` offset `2N + 2k + 1`, with `k = slot offset - 1`).
- The menu draws slots 1..N-1 (`03:871A`: N-1 iterations from offset 2; slot 0 is the plain attack and has no line).
- Examples (SEEN, ROM bytes decoded): unit `$00` N=6, slots `00 01 38 FF 04 05`, K = 4, 6, 10, (12), 255, 24; unit `$39`
  N=7, the same plus slot 6 = `$33` (K 255); unit `$3A` N=8, slots `00 01 38 FF 04 34 35 FF`, K = 4, 6, 10, 12, 255, 28, 30, 50;
  unit `$3B` (SSJ) N=6, the same slots as `$00` with K = 8, 10, 12, 14, 255, **85** (Kamehameha).
- **Cursor to move** (`03:876C`, RTL, callers `01:9ECB`, `01:938E`, `01:B102`): the menu cursor `$0D62` + 1 picks the
  N-th non-`$FF` byte from offset 2; the command id goes to record `+12`, the slot offset minus 1 to `+1F`, and the KI
  cost (`03:884B[id]`) to `$00..$01` (SEEN). Every time the choice is refreshed `03:89B3` loads `+1D` (attr) and `+1E`
  (K) from the list by the command id (`03:8838` finds the slot) (SEEN `03:89B3..89F1`).
- **Slot to card/move**: the command does not replace the card. The player picks a card afterwards (menu step 2,
  `01:9CFC`; the card's two values and symbol go to record `+14..+16`, SEEN `01:9D1D..9D31`), so a command is
  "this move, with that card's numbers": the damage routine reads `va` from the card (`05:B7FC/B801`) and `K` from `+1E`.
  For the Kaioken the card's first value indexes the multiplier table.
- **Enemies** use the same lists: `05:BB52`/`05:BB69` read `list[N+1]` and `list[2N+1]` (slot 0) through `05:BBAF`
  (`03:8752` with the unit id) for counter and special strikes (8). The cost is paid for them too (`05:9CF4`).

## 4. The Kaioken, step by step (code SEEN, flow INFERRED where said)

State bytes: `$0C36` (Kaioken: bit 7 on, bit 6 "just chosen"), `$0C37` (Spirit Bomb charge: bit 7, bit 6, low bits a
count), `$01EE` (a delayed copy of `$0C36`, used for the extra cost), `$01EF` (the multiplier `m` in use, 0 = none),
`$01EB..$01ED` (Goku's BP before the boost, 24 bits), `$7E:4C00 + 3*id` (the permanent BP snapshot, 9).
`$0C36` and `$0C37` are not in the save image; `03:E2C6` / `03:E3B4` copy them to and from `$7E:2AEC` (map state swap).

1. **Learning** (SEEN): message `$122` ("Congratulations, Goku. You're all set", then the talk about going back to Earth;
   INFERRED to be King Kai's, from its portrait `$34`) ends with `{E0:24}`;
   handler 36 of the table at `01:D43B` is `01:D982`, which writes unit `$39` to record 0 and clears `+1B`.
2. **Choosing it** (SEEN `01:9ECB..9F4C`, menu step 5): the A press calls `03:876C`; for character id 0 with `$0C36` bit 7
   set the cost is raised by 10 (`01:9ED7`); if the KI word `$0B05` is below the cost the "Not enough Ki. Cancel?" box
   follows (`01:9F9B`, block `$4D`). With enough KI: command `$33` sets `$0C36 = $C0` (when bit 6 was clear, `01:9F3B..9F44`)
   and goes to the card pick (`01:9F47`, step 2). The commands `$12 $1C $23 $27` (charge attacks) and `$35` take their own
   branches (`01:9EF8..9F18`).
3. **Applying it** (SEEN `05:AB04..AB1E`, `05:AB60..ABE3`): the attack set-up reads the effect id from `1D:8E3A[cmd]`; `$0E`
   calls `05:AB60`. It (a) if `$01EF != 0` first restores the saved BP (so a new choice does not compound), (b) reads
   the card's first value (`+14`) minus 1 and Goku's level minus 11 (floor 0) and looks `m` up in `1D:8E7B[(level-11)*8 + value-1]`,
   (c) stores `m` in `$01EF`, prints it (`05:ABE4` -> `1C:8044`), (d) saves the BP to `$01EB..$01ED` and `$17EA`, multiplies it
   by `m` (`1C:806C`, 24 x 16 bits to 40), halves the product and **adds it to the BP in place** (24 bits, the carry out
   is lost). So at level 48 `m` runs 14..20 by card (SEEN rows 48), at level 47 13..20. **Overflow**: a BP above
   16 777 215 / (1 + m/2) wraps; Goku's own BP at level 47 is 260 090 (sum of the rows, SEEN), so it never wraps, but the
   Super Saiyan BP (4 999 999) overflows at any factor above 3.35.
4. **The extra cost while on** (SEEN `05:9CD1..9CE8`, in the routine `05:9C57` that ends every move): the previous value
   of `$01EE` is tested before it is refreshed from `$0C36`; if its bit 7 was set `$08 = 10`. Then the cost is the table
   value plus `$08` (`03:87B7` computes the table part from the command id) and is taken from the KI of the side record
   (`05:9D0A..9D12`). The Kaioken-on surcharge therefore starts on the action after the one that started it.
5. **Ending by KI** (SEEN `05:9D14..9D3D`): when KI is not above the cost, if the fighter is category 0 and `$08 != 0` the
   Kaioken stops: `$0C36`, `$01EE`, `$0C37` are cleared, unit `$39` is written, the BP is restored from `$01EB..$01ED`;
   then in every case the KI becomes 0 (`05:9D40`).
6. **The prompt** (SEEN `01:9984..99A6`, `01:9FBC..A013`): when Goku is picked while `$0C36` bit 7 is set and bit 6 clear, a
   yes/no box opens (menu step 7). The first answer sets unit `$3A` when KI >= 10 (`01:9FF5`), the second restores the BP
   from `$01EB..` and writes unit `$39` (`01:9FD2..9FE8`). INFERRED: this is the "Use Kaioken?" (string 36 of `02:AB66`) /
   "Will you stop?" (string 41) pair; which text shows when was not run.
7. **Auto battle** (SEEN `03:8CFC..8D1E`): the Auto choice for Goku, if his BP is below the BP of any enemy, sets `$0C36 = $80`,
   unit `$3A` and command `$33`; the Spirit Bomb has a twin at `03:8D70` (`$0C37 = $80`, command `$35`). A story script can
   set the same state (`05:A87B`, animation event `$52`: unit `$3A`, `$0C36 = $80`).
8. **Clearing** (SEEN writers of `$0C36`): `00:9722` (map set-up for the ending fight), `01:9D69` / `01:9E28` (cancel while
   choosing), `01:9FE0`, `01:9FFC`, `03:86E4` (SSJ), `05:83F2` (a stale `$33` command at the start of the fight),
   `05:87AB` (flee), `05:9D24` (KI), `05:D2FB` (a fight-end path), `06:E83A` (battle set-up). After the map is reached
   `03:F06F` (called from `00:8F7D`) turns a unit `$3A` with `$0C36` bit 7 clear back into `$39`. INFERRED: a win while the
   Kaioken is still on leaves unit `$3A` until the next map return (the list then has no `$33`); not run.

Spirit Bomb (`$35`): KI must be at least 30 (`01:A046`, `01:A057`), a charge count in the low bits of `$0C37`
(`05:A8A4..A8B7`, `03:88DC`: `$85` = ready), unit `$3A` only.

## 5. The KI cost check

- **Menu check** (SEEN `01:9ECB..9EEC`): cost = `03:884B[cmd]` (via `03:876C`); `+10` for character id 0 when `$0C36` bit 7 is
  set; refused when the word `$0B05` (KI) is smaller (unsigned compare). Slot 0 (the plain attack) is not on this menu (the cursor starts at slot 1); on the payment side a plain
  attack (table 0) still pays the 10 KI surcharge while the Kaioken is on.
- **Payment** (SEEN `05:9CC0..9D45`, routine `05:9C57`, RTS, M8 X16, 10 JSR callers `05:8AE9`...): for the party
  (`active_side` bit 0 clear) the command is `$0B12,X` at `$14C3`; for the enemy `$0E12,X` at `$14C5`; command `$3C` pays
  nothing. KI after = KI - (table + `$08`), floored at 0 (the same store handles equality and shortfall). The side record
  is copied back to the party or enemy record afterwards (`05:9D5B..9D8A`), so any change to the side record must come
  before `05:9D4A`.
- A pack move needs a cost row: the table has 69 entries and the id is not bounds-checked (`03:87CC`): ids >= 69 read
  code bytes (SEEN: entry 69 on is `B9 14 0B...`). Hook candidate in `r2-tables.md`, row 12 (`03:87D0`).

## 6. Super Saiyan (SEEN unless said)

- **Trigger** (`05:872A` state 2 tail, `05:8772..87B3`): in group `$56` (or `$6E`), when the acting side's record is
  category 0 and its class `+1B` is not 6, its level is at least `$2F` (47) and `$14D2` is non-zero (the KO routine `05:A9AA` ORs a bit into `$14D2` at `05:A9F0` when
  a party fighter of category 9, 1, 15, 10, 11, 12 or 13 falls, table `05:AA0B`: Gohan, Piccolo, Vegeta, Krillin, Tien,
  Yamcha, Chiaotzu; not Goku himself), message `$68`, `$0C36`, `$0C37`, `$01EE` are cleared and
  `battle_phase` becomes `$1A`. That phase (`05:92C9..92DD`) calls `05:C45F` with A = 0, which calls **`03:8683`**:
  Goku found (`03:F021`), level >= 47, group `$56`, and either `$01BB` is below `$0D9A` or fewer party records (flag bits
  `$C0`) remain than `$0D9A`; `$0D9A` is the party size counted by the last-fight set-up (`03:C820..C848`, from the id list
  at `$7E:2AA0` plus Goku) (SEEN). Then **`03:85F0`** with A = 0.
- **Build** (`03:85F0..867A`): unit id = `03:867B[A]` (`3B 3F 3E`: Goku, Gohan, Piccolo); level = `03:850E[char] + 1` (48 for
  Goku, `$30` forced for Piccolo); BP, max HP, max KI, KI, HP are **assigned** (not added) from the character's row
  `level-1` (`03:85C7`): Goku's row 48 is BP 4 999 999, HP 800, KI 189; the defence factor comes from the unit row.
  Back in `03:86C1..86E7`: class `+1B = 6`, the new BP is saved to `$7E:4C00` (so the victory restore keeps it), the flag
  bits `$26` of `+00` are cleared and **`$0C36` and `$0C37` are cleared**.
- **"Kaioken and Genki Dama disabled"**: there is no explicit check. (a) The SSJ list (unit `$3B`) has no `$33/$34/$35`.
  (b) the builder clears the state. (c) `lookup_id_category[$3B] = 0` still, so the Kaioken code paths (`05:AB40`,
  `05:8AF1`, `01:9984`) would accept `$3B` if the list had `$33`.
- **Where a mod must look** (all write unit ids): `01:9FE6`, `01:9FF5`, `01:A002`, `05:83F5`, `05:9D2D`, `03:8D17`, `03:F087`,
  `05:A883` write `$39` or `$3A` into record `+01` and would turn an SSJ Goku back into a plain one.

## 7. What "Super Saiyan Kaioken" needs (hook proposals, in the style of `cheats.rs`: `Ctx::resume_at` at an address)

| Need | Place (SEEN addresses) | Proposal |
|---|---|---|
| `$33` in unit `$3B`'s list | `03:8769` (post-load of the list pointer, r2 row 10) | pack list for `$3B`: N=7, slots + `$33`, attr + `$FF`, factor + `$00FF` |
| keep unit `$3B` | the 8 writers in 6 (`LDA #$39/#$3A` + `STA +01`, 4 bytes each) | wrapper at each `LDA`: when the record's `+01` is `$3B`, `resume_at` after the store |
| multiplier up to x10, no 24-bit wrap | `05:AB60` (RTS, M8 X16, `Y` = side record) | wrapper: decline unless the unit is `$3B`; else compute `BP*(2+m)/2` with a 40-bit shift and write the wide BP (Task 9) |
| 10 % of maximum HP per move with both on | `05:9D4A` (inside `05:9C57`, before the record copy-back; side record `Y`) | when `+01 == $3B` and `$0C36` bit 7: `HP := max(1, HP - maxHP/10)` on `$1403/$1404` (side A) then `resume_at(05:9D4A)` |
| the "stop" paths | `05:9D14..9D3D`, `01:9FD2..9FE8` | restore the 24-bit BP from the snapshot; keep the unit |
| SSJ builder clears the state | `03:86E4`, `03:86E7` | harmless: Kaioken is chosen after the transformation |
| BP restore at victory | `05:8DDA` -> `01:FE7E` | restores from `$7E:4C00`; SSJ's BP was saved there by `03:86CD..86D6`, so it stays; a Kaioken boost is lost (wanted) |

Caution: every site above is inside bigger routines; the cheats already hook inner addresses, but timing of mid-routine
keys was not tested here (r2 note, section 0).

## 8. The special-attack path

- **Roll** (SEEN `05:BA8B..BB51`, JSL from `05:8B40` in battle state 6, `05:8AF1`): `$10 = 0`; unless the move is one of a
  list of animation ids (`$B7`), the chance is `min(10 + attacker level + attacker `+0E` - card value - defender level, 24)`
  against a random byte masked with `$0F` (animation ids `$2F`, `$43`) or `$1F`; if it passes `$10 = 1` and
  **`active_side |= 8` at `05:BB46`** (the store; `05:BB49..BB4E` then clears bit 6 of `$14C7`). Animation `$51` always passes.
- **Use**: with `$10 != 0` the hit flag `$14CC` is set, the animation is picked (`05:8B50..8B7D`) and the damage is computed
  with **`05:B7FC`** (Y = 3: `D` shifted by 4, i.e. twice the damage of `05:B801`); the message is `battle:93` "uses a
  special attack on" (SEEN text dump). With `$10 = 0` the state goes to phase 9.
- **Factor**: `05:BB69` (called at `05:B82A`) returns `K`: with `active_side` bit 3 (special) or bit 1 (counter) it reads **slot 0 of
  the command list** through `05:BBAF` -> `03:8752` (unit id) -> `list[2N+1]`; otherwise `+1E` of the side record (the factor
  of the chosen slot). `05:BB52` is the twin for the card value (`list[N+1]` for a counter). Both double `K` for Gohan
  (category 9) with `$14EA` bit 6 (`05:BBA3..BBAB`).
- **SEEN run**: `xtask shots .work/replays/secret-raditz.ssdreplay --upto 5160 --watch 0x7E14C2,... --watch-from 5140`:
  `$14C2` becomes 8 at frame 5144 (pc `05:BB49`), the hit flag at `05:8B50`, `$14CF` goes from 53 to 121 (the first call of
  the pair uses shift 4). The same bit appears in `secret-frieza-forced-battle` (frame 3777) and
  `secret-zarbon-vegeta-joins` (frame 4629).
- The older note says the power "comes from the attack table through 03:8752": that is slot 0 of the unit's command list
  (above), not a global attack table.

## 9. Readers and writers of record bytes `+07..+09` (BP, 24 bits)

Method (SEEN): ROM scan (`bpscan.py`) of absolute and absolute-indexed operands `$0B07..09` (party), `$0E07..09` (enemy),
`$1407..09` / `$1447..49` (side records) and `$0007..09,X/Y` (record-relative, `Y` = a record base), banks 00 to 05, 0D, 1C,
1D (hits in other banks are data); every code hit was read in context. Not covered: reads through a direct-page pointer
(only `05:B4A2` was found by reading), through long addresses (none found for `$7E:0B07`), and block copies of whole records
(listed at the end). `$7E:4C00 + 3*char id` is the BP snapshot, written by the saver and read by the restorer below.

| Site (bank:addr) | R/W | What |
|---|---|---|
| `00:89C1..89D0` | RW | build / level-up: adds the row's BP to the record (`00:89B8`; also the second entry `00:8A07..8A1F`) |
| `00:8AA8, 8AB1` | W | characters 8+ (except 11): BP set from unit row `id + 26` (`00:8A69`) |
| `01:9FD5, 9FDD` | W | Kaioken "stop": BP restored from `$01EB..ED` |
| `01:AC24, AC2F`; `01:C033, C03C`; `01:EEE6, EEF2` | R | menu displays (Status, command screen, Scouter-style readout) |
| `01:C056, C05F`; `01:E69F..E6FF` | R | enemy BP displays / the Scouter readout and its running maximum (INFERRED from `$0740`) |
| `01:DF3E..DF71` | R | BP objects: `+50 %` (`BP/2`) or `+25 %` (object id 5, King Kai), marks the record bit `$20` (temporary, undone by the snapshot) |
| `01:E438, E446` | R | stat object: preview of the row's BP gain |
| `01:E4AE..E4BE` | RW | stat object: adds the gain to the record **and** to `$7E:4C00 + 3*id` (permanent) |
| `01:E568..E59B` | RW | BP object: adds a table value, clamps at `$0744..46` (the row's BP) |
| `01:EAD5, EAEA` | W | builds the temporary record of unit `$4A` (BP 24 000) |
| `01:FE64..FE70` / `01:FE9B..FEA4` | R / W | **snapshot save** (`01:FE4B`, callers `00:9004`, `00:95C9`, `03:C84B`, `03:D1BF`, `03:D23C`, `03:E6A9`, `03:F2A2`) / **restore** (`01:FE7E`, callers `01:98DA`, `01:A316`, `03:B8A1`, `03:D2AB`, `03:D2CB`, `05:8DDA` = victory) |
| `01:F7B5..F7DC` | W | enemy records: BP from the unit row (`01:F719`, with the small random figure) |
| `03:8061..806C`; `03:8096..80A1` | R | sum of the BP of the party / of the enemies (battle AI helper) |
| `03:82CB..82D7` | R | picks the party member with the largest BP (battle order helper) |
| `03:855E..856D` | RW | level-up: adds the row's BP (`03:841E` chain) |
| `03:862F, 8638` | W | form builder (`03:85F0`): **assigns** BP from the level row (SSJ and the Gohan/Piccolo forms) |
| `03:86CA..86D6` | R | after the build: copies the new BP to `$7E:4C00` |
| `03:8D01..8D0D`, `03:8D75..8D81` | R | Auto choice: party BP - enemy BP < 0 triggers the Kaioken / Spirit Bomb (4) |
| `03:B205..B20B` | W | new record: BP = 0 (`03:B203`, effect add-member) |
| `03:BB76, BB80` | W | record of character id `$0C`: BP 0 |
| `03:BD1D, BD2F` | W | record built from a unit row (`02:B214`) |
| `03:C5A0, C5AB` | W | effect `$35` (Piccolo returns): BP from the table `03:C5BA` by level - `$21` |
| `03:C6A1..C6AA`; `03:C8C5..C8D1`; `03:E11C..E125` | R | save a character's BP to `$7E:4C00 + 3*id` (effects `$3A`, `$3E`, helper) |
| `05:83E4, 83EC` | R | start of a fight: stale Kaioken command `$33` -> BP saved, `$0C36` cleared |
| `05:9D35, 9D3D` | W | Kaioken end by KI: BP restored |
| `05:A54A..A57A` | RW | form change: BP divided by 10 (`1C:8136`) |
| `05:A622..A649` | RW | form change: BP multiplied by 10 (`1C:806C`) |
| `05:A727..A765` | RW | "power up": BP + BP/2 (BP/4 for unit `$2F`), saturating at `$FF..` |
| `05:AB6A..ABE0` | RW | **Kaioken boost** (4) |
| `05:AC4F, AC57`; `05:AC7D, AC86` | R | Gohan "rage" test: party BP * 10 compared with each enemy's BP (INFERRED, `$14EA`) |
| `05:B324, B329`; `05:DDD3..DDEE` | R | side B BP into `05:B4A2` (searches a record with equal BP through a DP pointer); tests of BP >= 10 000 for both sides |
| `05:B6F4..B736` | R | **damage**: `BPa`, `BPd` read (`05:B6F2`, `05:B6E0`) with the +25 % class bonus |
| `05:BCF2..BD05` | R | counter-attack chance: category-`$0F` party BP minus the side's BP |
| `05:D389..D3B3` | RW | copy of a party record into an enemy slot with BP reduced by a quarter (INFERRED: the "Body Change" move) |
| `1D:8479..851B` | R | battle HUD: BP of side A and B to the digit routine `1D:84B0` (24-bit binary to BCD) |

Block copies of the whole 32-byte record (they copy BP too): `05:AE81` / `05:AE16` (record to side record), `05:9D5B` /
`05:9D79` (side record back), `05:D370` and `00:C6A6..C74E` (11 bytes `+01..+0B` to the direct page), the save/load of the
roster image (`03:E419`, `03:E461`, `03:E4F4`, `docs/re/data/save-format.md`).

## 10. The 32-byte record: bytes and what is free

Counts of absolute-indexed uses per offset (SEEN scan, banks 00 to 05, 1C, 1D, party + enemy + side + relative) show
every offset in use. What the rarely used ones are (SEEN contexts):

| Byte | Use |
|---|---|
| `+10` | the middle byte of the 24-bit EXP |
| `+12` | chosen command id; `+13` is its high byte (word accesses in the command resolution `03:8E6A..8F0B`) |
| `+14..+16` | the card in play (values and symbol) |
| `+17` | defence factor |
| `+18` | flags (bit 7 "command pending", bit 3 "charge attack", `$70` after the SSJ phase) |
| `+19` | a battle status byte (copied at `05:976B..978B`, tested at `05:9F0A`, `05:A920..A934`, `05:BC10`) |
| `+1A` | the hand slot the card came from |
| `+1B` | class (and 6 for SSJ); `+1C` character id |
| `+1D`, `+1E`, `+1F` | attr, factor K and slot offset of the chosen command (`03:89E7`, `03:89EE`, `03:87AD`) |
| `+20` (next record's `+00`) | on the side records `$1420`/`$1460` is a "next form" unit id (`05:9C57..9C65`, `05:A539`) |

**Conclusion**: no byte is provably free. `+13` and `+19` are the least used and are still written by battle code; do not
store wide BP there. The plan's separate mod block (`$7F:FE00`) is the right place. INFERRED: `+10`, `+0B`, `+0D`, `+04`,
`+06`, `+08` are high bytes of words and cannot be spared.

## 11. Not verified

- No Kaioken fight was run (no replay uses `$33`); the BP factor `1 + m/2` and the unit swaps are code readings.
- Which text the Kaioken prompt shows (strings 36 and 41) and the exact menu step order after `$33` is chosen were not traced.
- `$01BB` (a counter that many routines increment) is compared with `$0D9A` at `03:869C`; its meaning in that test is not settled.
- Sites that read BP through direct-page pointers or computed addresses are not listed beyond `05:B4A2`.
- Timing of mid-routine hook keys is untested (r2 note).

---

# Part B. Every story check of the party's level, EXP or BP (for "Free progression")

The complete list, with the corrections of this part, is `free-progression.md`: it adds the second level test of the Super Saiyan
trigger (`05:8791`), measures what the floors do, and shows that the BP table of effect `$35` holds only the levels 33 to 40.

Method: a ROM-wide scan for absolute, indexed and long operands in `$0B00..$0C1F` at the level (`+02`), BP (`+07..09`) and
EXP (`+0F..11`) offsets, with every code hit in banks 00 to 05 read in context; all story code asks "is character N in
the party" through `03:F021` (A = character id, Y = record offset, carry set when absent) and its 28 call sites were
listed. Scene scripts (bank 6) have **no** level, EXP or BP compare: the 51 commands test flags only, and levels enter only
through the effect routines (command `1B nn`). Nothing in story code compares EXP.

## B.1 Real gates

| # | Site | Reads | Compare | Gates | Replacement for "Free progression" |
|---|---|---|---|---|---|
| G1 | `03:F21F..F23E` (map 0 trigger, `03:F212`; verified) | Goku level `+02`, else Piccolo's | Goku >= 5, or Piccolo >= 4 (flag 3 set, flag `0x0B` clear) | starts scene 2, the forced fight with Raditz (`JMP 03:F7DE`) | an event condition; wrapper returns at `03:F240` when the pack's step says so |
| G2 | `02:D477..D498` (map 0, called by the encounter code) | same levels | same | zeroes `$0D8B`/`$0D8A`: random battles stop | wrapper on `02:D477` that returns at once |
| G3 | `03:F300`, body `03:F320` | Piccolo level, else Gohan's | Piccolo >= 9 or (no Piccolo) Gohan >= 10 | `$0173` bit 5, `$0174 = 8`, kills the Z fighters not yet recruited; stops map-5 random battles. Callers `03:F259`, `02:D472` | already neutralised in Remastered (`story_level_deadline` at `03:F320`); a pack keeps it neutral and gives the deadline an event |
| G4 | `03:EC38..EC55` (map 7 set-up, `03:EC25`) | Goku level | < 4 -> 7, else 8 | raises Goku to that level (catch-up) | only raises; keep |
| G5 | `03:868B..8697` (`03:8683`, the SSJ trigger, 6) | Goku level | >= `$2F` (47) and group `$56` and the party-count test | the Super Saiyan transformation | the level is a hard constant: with scaled levels Goku may never reach 47; wrapper on `03:8683` replaces the level test by "group `$56` and Goku present", or the scene `$3F` floor (below) raises him |
| G6 | `03:EDF2..EE08` (map 16 set-up, `03:ED9C`; verified) | roster image Goku level `$7E:2802` | < `$22` -> 1; `$22..$23` -> 2; >= `$24` -> 4 | ORs bit 0, 1 or 2 into `$0172` (flags `0x10..0x12`); flag `0x12` decides whether the Ginyu fight starts with Goku's landing | set the flag from the story, not from the level; wrap `03:ED9C` and write `$7E:2802` first |
| G7 | `05:9183` | Dende level | < 3 skips | Dende heals in group `$55` fights | harmless (Dende's cap is 3) |
| G8 | `01:A0DC`, `01:99BB` | record at `$0C20`; Dende level | >= 3 and `$0C38` bit 7 clear | the map menu's Dende healing; messages `$09`/`$0A` | harmless |
| G9 | `05:AC3E` | Gohan (category 9) level and BP | level >= 11 sets `$14EA` bit 4; BP * 10 compared with each enemy | Gohan's "rage" (K doubled when `$14EA` bit 6) | battle event; a tuning point if levels are scaled |
| G10 | `03:8D01..8D14`, `03:8D75..8D88` | party BP - enemy BP | party < enemy | the Auto choice: Kaioken (unit `$3A`, `$0C36 = $80`, command `$33`) or Spirit Bomb (`$0C37 = $80`, command `$35`). (A first reading, "Spirit Bomb unit", is corrected by 4, item 7.) | battle event; with enemy BP scaled to the party the compare flips |
| G11 | `05:BCF2..BD05` | party BP vs the side's BP | carry | counter-attack chance (battle) | battle only |
| G12 | `05:8E3F..8E55` | none (a fixed `DEC` of Goku's level twice, then `03:841E`) | - | end of a fight with `$01D6` bit 7 set, bit 6 and `$01E7` bit 1 clear (INFERRED: the practice fight; story transition `$14` follows) | keep: a fixed -2 levels, not a compare |

## B.2 The level-up check and the caps (relevant to "level cap 100")

- `03:841E` is the level-up check (called from `05:8FDD` after each fight, `05:8E51`, `03:CD8A`). It skips character ids 8,
  9, 10 and 12 and records without flag bit 6, compares the EXP of the next level (the character's row) with `+0F..11`
  and raises `+02`.
- **The cap is a table**, not a constant: `03:850E` = `2F 30 2F 30 30 30 30 2F 00 00 00 03` indexed by character id (Goku
  47, Gohan 48, Piccolo 47, Krillin/Tien/Yamcha/Chiaotzu 48, Vegeta 47, ids 8..10 none, Dende 3). Readers: `03:84EE`
  (7 callers), `03:8476`, and `03:8610..861F` (a promotion sets cap + 1; Piccolo is forced to 48). An id of 12 or more
  reads past the table. Other constants: 47 at `03:868E` and `05:C8C5` (48 for category 9), 99 at `01:E420`, Vegeta's
  floor 17 at `00:8A46..8A53` and `03:84B2..84BF`.
- There are 48 rows per character, so any level above 48 needs generated rows (the level row reader `00:8A28`).

## B.3 Level floors and gifts (they only raise levels: not gates)

All use `03:C64A` (`$164C` = character, `$164D` = delta, then `03:851B`; stops at the cap; the exact arithmetic is
INFERRED, off by one possible).

| Site | Context | Floor / gift |
|---|---|---|
| `03:EAED` (map 1 set-up `03:EAC4`) | Piccolo alone | to 13 |
| `03:EBB1` (map 5 set-up `03:EB54`) | Piccolo from the roster image | to 7 |
| `03:EC38..EC55` (map 7) | Goku | to 7 if < 4, else 8 |
| `03:EE5F` (maps 18 to 20, `03:EE2A`) | Goku alone; if `$0180` (King Kai trials) is non-zero it sets `$0170` bit 1 instead | to 13, then scene `$50` |
| `03:FA79` (post-scene hook of scene `$3F`, `03:FA3F`, `$1661` = 3) | Goku | to `$31` = his cap 47: what makes G5 reachable |
| `03:D203..D233` (effect `$67`, `03:D1BF`) | every party member | to Piccolo's roster level; sets each unit id from `03:D249` |
| `03:D1B4` (effect `$66`) | Piccolo | promoted (unit `$3E`, level 48) by `03:85F0` |
| `03:C522` (effect `$35`, Piccolo returns) | BP from `03:C5BA` indexed by level - `$21` | **hazard**: needs level >= 33 or the index wraps (wrong BP); keep Piccolo >= 33 here or replace the lookup |
| `06:D26B..D27E` (scene 4), `06:DB9B..DBA3` (55), `06:DC68..DCBC` (60), `06:DF52..DF5A` (76) | effect `$38` gifts | Gohan +2/+3; Piccolo +5, +4/+2/+1, +20 |

Other: `03:A587` (price band by the party's maximum level, training arena) is a reward scale; King Kai's trials (effects
`$51..$53`, `03:CD10..CDB1`) count in `$0180` and pay EXP by counters, the only EXP source outside battle; nothing to change.

## B.4 What the pack has to change, and hook candidates

1. G1, G2: stop reading levels on map 0. 2. G3: keep `story_level_deadline` neutral and give the deadline an event.
3. G5: replace the level >= 47 test (or force Goku to 47 at scene `$3F`, as the game already does). 4. G6: set flags
`0x10..0x12` from the story. 5. Floors are harmless, but effect `$35` needs Piccolo >= 33. 6. The cap table `03:850E` and
the 47/48/99 constants if the cap changes. 7. Battle events G9..G11 compare BP between party and enemies: re-check
the Auto Kaioken (G10) and Gohan's rage (G9) when enemy BP is scaled.
Candidate keys (cheats style): `03:F320` (exists), `03:F300` (RTL, M8 X16), `02:D477` (RTL), the map-0 trigger `03:F212`
(inline: wrap the entry, RTL), `03:8683` (RTL), and the map-16 set-up `03:ED9C` (write `$7E:2802` first).

## B.5 Not verified

Reads through direct-page pointers and computed addresses; of the 43 `#$0B00..` immediates in banks 00 to 05 only six
were read (block copies and record loops with no compare); the final level after a floor.
