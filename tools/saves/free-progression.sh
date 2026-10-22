#!/usr/bin/env bash
# Headless proofs of the feature free_progression (docs/re/data/free-progression.md, section 7): each gate of the story that read a
# level is played on a crafted save, once as the original and once on the image of the example pack, and what the story did is printed.
#
#   tools/saves/free-progression.sh [name ...]       names: raditz-low raditz-high encounters deadline ssj ginyu piccolo
#
# It needs the crafted saves and replays of tools/saves/secrets.sh (.work/demo/secret-*.sav, .work/replays/secret-raditz.ssdreplay),
# the cached ROM and a built xtask. The saves, replays and dumps go to .work/fpg/ (never tracked) and are deleted at the end.
# Run from the workspace root. Without a name every proof runs; each prints the lines that show the story moving (or not).
set -euo pipefail
x="cargo run --release -q -p xtask --"
[ -x target/release/xtask.exe ] && x=target/release/xtask.exe
[ -x target/release/xtask ] && x=target/release/xtask
demo=.work/demo
timelines=tools/saves/secrets
pack=examples/free-progression
out=.work/fpg
mkdir -p "$out"
only=("$@")
want() { [ ${#only[@]} -eq 0 ] || printf '%s\n' "${only[@]}" | grep -qx "$1"; }

# craft BASE NAME TIMELINE SET...: the crafted save of NAME from the base save, and its replay with the timeline.
craft() {
    local base=$1 name=$2 timeline=$3
    shift 3
    local sets=()
    for s in "$@"; do sets+=(--set "$s"); done
    $x save-edit "$demo/$base.sav" "$out/$name.sav" "${sets[@]}" > /dev/null
    $x replay-from-save "$out/$name.sav" "$timelines/$timeline.txt" "$out/$name.ssdreplay" > /dev/null
}

# both LABEL REPLAY UPTO WATCH FILTER [shots args...]: the replay without and with the pack, the watch lines that match FILTER.
both() {
    local label=$1 replay=$2 upto=$3 watch=$4 filter=$5
    shift 5
    for world in original pack; do
        local mods=()
        [ "$world" = pack ] && mods=(--mods "$pack")
        echo "--- $label, $world"
        $x shots "$replay" ${mods[@]+"${mods[@]}"} --upto "$upto" --watch "$watch" --no-png "$@" 2>&1 | grep -E "$filter" || true
    done
}

# party OUT.wram: the party records of a dump of the work RAM.
python=""
for candidate in python3 python; do
    if command -v "$candidate" > /dev/null && "$candidate" -c '' 2> /dev/null; then
        python=$candidate
        break
    fi
done
party() {
    "$python" - "$1" <<'PY'
import sys
w = open(sys.argv[1], 'rb').read()
print('  mode %02X map %d group %02X' % (w[0x24], w[0x1A0], w[0xEA3]))
for r in range(9):
    b = 0xB00 + 32 * r
    if w[b] & 0xC0:
        bp = w[b + 7] | w[b + 8] << 8 | w[b + 9] << 16
        print('  record %d: unit %02X level %d character %d BP %d' % (r, w[b + 1], w[b + 2], w[b + 0x1C], bp))
PY
}

if want raditz-low; then
    # The imposter in the cave is beaten (flag 0x0A), Goku and Piccolo at level 1: the forced fight comes on the event.
    craft secret-raditz raditz-low mash-3000 goku.level=1 piccolo.level=1 flags=+0xA
    both "Raditz, levels 1, imposter beaten" "$out/raditz-low.ssdreplay" 2700 scene_id,7E0171 'scene_id=02|00171=0C'
fi
if want raditz-high; then
    # Levels 12 and the imposter not beaten: the original starts the fight, the feature waits for the event.
    both "Raditz, levels 12, imposter not beaten" .work/replays/secret-raditz.ssdreplay 3300 scene_id,7E0171 'scene_id=02|00171=08'
fi
if want encounters; then
    both "random battles on map 0, levels 12" .work/replays/secret-raditz.ssdreplay 2200 encounter_level,encounter_zone \
        'frame +2132'
fi
if want deadline; then
    for level in 9 30; do
        craft secret-recruit-krillin "deadline-$level" recruit-krillin "piccolo.level=$level"
        both "recruits' deadline, Piccolo $level" "$out/deadline-$level.ssdreplay" 3000 7E0172,7E0173,7E0174 'frame +(2132|2529) '
    done
fi
if want ssj; then
    # The last fight with Goku at level 20 and the numbers of his level 47.
    craft secret-ssj-frieza ssj mash-7000 goku.level=20 goku.maxhp=745 goku.hp=745 goku.maxki=138 goku.ki=138 goku.bp=260090 \
        goku.exp=170000
    for world in original pack; do
        mods=()
        [ "$world" = pack ] && mods=(--mods "$pack")
        echo "--- Super Saiyan, Goku level 20, $world"
        $x shots "$out/ssj.ssdreplay" ${mods[@]+"${mods[@]}"} --upto 9300 --watch battle_phase --wram-out "$out/ssj-$world.wram" \
            --no-png 2>&1 | grep -E 'battle_phase=1A' || true
        party "$out/ssj-$world.wram"
    done
fi
if want ginyu; then
    # Goku's roster level 10 on a quiet map 16; mode 6 is entered by a host write at frame 2500 so that the chapter set-up runs.
    craft secret-ginyu-jeice-burter ginyu mash-3600 image:02=0A flags=none,+1,+3,+7,+0x1F,+0x22
    both "map 16 set-up, Goku's roster level 10" "$out/ginyu.ssdreplay" 3300 7E0172 'frame +2502 ' --wram 2500:7E0024=86
fi
if want piccolo; then
    for level in 40 25 45; do
        craft secret-piccolo-returns "piccolo-$level" frieza-flees "piccolo.level=$level"
        for world in original pack; do
            mods=()
            [ "$world" = pack ] && mods=(--mods "$pack")
            echo "--- Piccolo returns at level $level, $world"
            $x shots "$out/piccolo-$level.ssdreplay" ${mods[@]+"${mods[@]}"} --upto 8560 --wram-out "$out/piccolo-$level-$world.wram" \
                --no-png > /dev/null 2>&1
            party "$out/piccolo-$level-$world.wram" | sed -n '1,2p'
        done
    done
fi
rm -rf "$out"
