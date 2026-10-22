#!/usr/bin/env python3
"""Lists the instructions of the cached ROM that read or write the level, the BP or the EXP of a character record.

    python tools/re/story_level_reads.py [--rom canonical.sfc] [--all]

The scan behind `docs/re/data/free-progression.md`: every instruction of the ROM whose operand is one of the bytes of those three
fields of a party record (`$0B00`, nine of 32 bytes), of an enemy slot (`$0E00`), of a side record of a battle (`$1400`, `$1440`),
of the roster image (`$7E:2800`) or of the BP snapshot (`$7E:4C00`), absolute, indexed or long, and the record-relative forms
(`$0002,Y`, `$0007,X`... for a record base held in a register). A match is only the bytes of an instruction: a table or a graphic
that holds the same pair matches too, so each routine has to be read (`xtask disasm <address>`). The output groups the matches
by routine (the nearest entry before them: a target of a call or a jump, a word of the effect and trigger tables, a routine of
`re/symbols.toml`) and ends with the counts. It prints to stdout only: it is derived from the game and is never written to a
tracked file (CONTRIBUTING.md). The ROM is read from the cache of the game; the file holds no data of it.
"""
import bisect
import collections
import os
import re
import sys

# The fields of a record, by offset: the level, the BP (3 bytes), the EXP (3 bytes).
FIELDS = {2: "level", 7: "bp0", 8: "bp1", 9: "bp2", 0x0F: "exp0", 0x10: "exp1", 0x11: "exp2"}
# Opcodes with a 16-bit operand (absolute, absolute indexed) and with a 24-bit one (long), by mnemonic.
ABS = {
    0xAD: "LDA", 0x8D: "STA", 0xEE: "INC", 0xCE: "DEC", 0x2C: "BIT", 0x0C: "TSB", 0x1C: "TRB", 0xCD: "CMP", 0x9C: "STZ",
    0x6D: "ADC", 0xED: "SBC", 0x2D: "AND", 0x0D: "ORA", 0x4D: "EOR", 0x0E: "ASL", 0x4E: "LSR", 0x2E: "ROL", 0x6E: "ROR",
    0xAE: "LDX", 0xAC: "LDY", 0x8E: "STX", 0x8C: "STY", 0xEC: "CPX", 0xCC: "CPY", 0xBD: "LDA,X", 0x9D: "STA,X",
    0xB9: "LDA,Y", 0x99: "STA,Y", 0x7D: "ADC,X", 0x79: "ADC,Y", 0xFD: "SBC,X", 0xF9: "SBC,Y", 0xDD: "CMP,X", 0xD9: "CMP,Y",
    0x3D: "AND,X", 0x39: "AND,Y", 0x1D: "ORA,X", 0x19: "ORA,Y", 0x5D: "EOR,X", 0x59: "EOR,Y", 0xDE: "DEC,X", 0xFE: "INC,X",
    0x9E: "STZ,X", 0xBC: "LDY,X", 0xBE: "LDX,Y", 0x1E: "ASL,X", 0x5E: "LSR,X", 0x3E: "ROL,X", 0x7E: "ROR,X", 0x3C: "BIT,X",
}
LONG = {
    0xAF: "LDA", 0xBF: "LDA,X", 0xCF: "CMP", 0xDF: "CMP,X", 0x8F: "STA", 0x9F: "STA,X", 0x6F: "ADC", 0x7F: "ADC,X",
    0xEF: "SBC", 0xFF: "SBC,X", 0x2F: "AND", 0x3F: "AND,X", 0x0F: "ORA", 0x1F: "ORA,X", 0x4F: "EOR", 0x5F: "EOR,X",
}


def default_rom():
    base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~/.local/share")
    return os.path.join(base, "dbz-ssd-recompiled", "cache", "rom", "canonical.sfc")


def phys(offset):
    return offset // 0x8000, 0x8000 + offset % 0x8000


def fmt(offset):
    bank, addr = phys(offset)
    return "%02X:%04X" % (bank, addr)


def in_code_banks(offset):
    """The banks that hold code (the others hold graphics, text, scripts and tables): 0 to 5, the code of 6, 10, 13, 1C, 1D."""
    bank, addr = phys(offset)
    if bank in (0, 1, 2, 3, 4, 5, 0x1C, 0x1D):
        return True
    return (
        (bank == 6 and addr >= 0xE800)
        or (bank == 0x10 and (addr < 0x81F0 or 0xA000 <= addr < 0xA030))
        or (bank == 0x13 and 0xEB40 <= addr < 0xEBA0)
    )


def in_code_ranges(offset):
    """The ranges of those banks that hold code (the recompiler's map of the code of 1C and 1D, and the code of 10, 6 and 13)."""
    bank, addr = phys(offset)
    if bank in (0, 1, 2, 3, 4, 5):
        return True
    return (
        (bank == 6 and addr >= 0xE800)
        or (bank == 0x10 and addr < 0x81F0)
        or bank == 0x13
        or (bank == 0x1C and addr < 0x8200)
        or (bank == 0x1D and addr < 0x8D00)
    )


def scan(rom):
    """Every match: (offset, mnemonic, operand, kind, field)."""
    hits = []
    for o in range(len(rom) - 3):
        op = rom[o]
        if op in ABS:
            v = rom[o + 1] | rom[o + 2] << 8
            indexed = "," in ABS[op]
            if 0x0B00 <= v < 0x0C20 and (v - 0x0B00) % 32 in FIELDS:
                hits.append((o, ABS[op], v, "party", FIELDS[(v - 0x0B00) % 32]))
            if indexed and v in FIELDS:
                hits.append((o, ABS[op], v, "rel", FIELDS[v]))
            if 0x0E00 <= v < 0x0EA0 and (v - 0x0E00) % 32 in FIELDS:
                hits.append((o, ABS[op], v, "enemy", FIELDS[(v - 0x0E00) % 32]))
            if 0x1400 <= v < 0x1480 and (v - 0x1400) % 32 in FIELDS:
                hits.append((o, ABS[op], v, "side", FIELDS[(v - 0x1400) % 32]))
        if op in LONG:
            v = rom[o + 1] | rom[o + 2] << 8 | rom[o + 3] << 16
            bank, low = v >> 16, v & 0xFFFF
            if bank in (0x00, 0x7E, 0x80):
                if 0x0B00 <= low < 0x0C20 and (low - 0x0B00) % 32 in FIELDS:
                    hits.append((o, LONG[op], v, "party-long", FIELDS[(low - 0x0B00) % 32]))
                if 0x1400 <= low < 0x1480 and bank in (0x00, 0x7E) and (low - 0x1400) % 32 in FIELDS:
                    hits.append((o, LONG[op], v, "side-long", FIELDS[(low - 0x1400) % 32]))
            if bank == 0x7E:
                if 0x2800 <= low < 0x2980 and (low - 0x2800) % 32 in FIELDS:
                    hits.append((o, LONG[op], v, "roster", FIELDS[(low - 0x2800) % 32]))
                if 0x4C00 <= low < 0x4C30:
                    hits.append((o, LONG[op], v, "bpsave", "-"))
    return hits


def entries(rom, repo):
    """Entry points of routines: targets of calls and jumps, the words of the effect and trigger tables, the symbols."""
    targets = set()
    for o in range(len(rom) - 3):
        op = rom[o]
        bank = o // 0x8000
        if op in (0x22, 0x5C):  # JSL, JML: a long target
            tb, ta = rom[o + 3] & 0x7F, rom[o + 1] | rom[o + 2] << 8
            if tb < 0x20 and ta >= 0x8000:
                targets.add(tb * 0x8000 + ta - 0x8000)
        elif op in (0x20, 0x4C):  # JSR, JMP: a target in the same bank
            ta = rom[o + 1] | rom[o + 2] << 8
            if ta >= 0x8000:
                targets.add(bank * 0x8000 + ta - 0x8000)
    for base, count in ((0xB6A8, 110), (0xF1AE, 50)):  # effect routines, per-map triggers (bank 3)
        for i in range(count):
            at = 3 * 0x8000 + base - 0x8000 + 2 * i
            targets.add(3 * 0x8000 + (rom[at] | rom[at + 1] << 8) - 0x8000)
    symbols = os.path.join(repo, "re", "symbols.toml")
    if os.path.exists(symbols):
        for m in re.finditer(r"address\s*=\s*0x([0-9A-Fa-f]{6})", open(symbols).read()):
            value = int(m.group(1), 16)
            bank, addr = value >> 16, value & 0xFFFF
            if bank < 0x20 and addr >= 0x8000:
                targets.add(bank * 0x8000 + addr - 0x8000)
    every = sorted(targets)
    after_return = sorted(t for t in targets if rom[t - 1] in (0x60, 0x6B, 0x40) or t % 0x8000 == 0)
    return every, after_return


def main(argv):
    rom_path = default_rom()
    show_all = False
    it = iter(argv)
    for arg in it:
        if arg == "--rom":
            rom_path = next(it)
        elif arg == "--all":
            show_all = True
    rom = open(rom_path, "rb").read()
    repo = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
    hits = scan(rom)
    banks = [h for h in hits if in_code_banks(h[0])]
    code = [h for h in banks if in_code_ranges(h[0])]
    every, after_return = entries(rom, repo)

    def routine_start(o):
        i = bisect.bisect_right(after_return, o) - 1
        if i >= 0 and o - after_return[i] <= 0x300 and after_return[i] // 0x8000 == o // 0x8000:
            return after_return[i]
        i = bisect.bisect_right(every, o) - 1
        if i >= 0 and o - every[i] <= 0x100 and every[i] // 0x8000 == o // 0x8000:
            return every[i]
        return o - 0x30

    groups = collections.OrderedDict()
    for h in sorted(code):
        groups.setdefault(routine_start(h[0]), []).append(h)
    for start, hs in groups.items():
        kinds = collections.Counter(h[3] for h in hs)
        print(fmt(start), "n=%d" % len(hs), dict(kinds), " ".join("%s %s@%s" % (h[1], h[4], fmt(h[0])[3:]) for h in hs[:8]))
        if show_all:
            for h in hs:
                print("    %s  %-7s %-10s %06X %s" % (fmt(h[0]), h[1], h[3], h[2], h[4]))
    print(
        "matches in the 32 banks: %d; in the banks that hold code: %d; in the code ranges: %d; routines: %d"
        % (len(hits), len(banks), len(code), len(groups))
    )


if __name__ == "__main__":
    main(sys.argv[1:])
