# Triggers and talk in data packs

A data pack can start one of its own scenes when the party reaches a place on a map, and it can give the objects of a
pack map their own messages. The scenes are written in the scene language, the messages in the messages format
(`text.md`), the maps in the Tiled format (`maps.md`). This page covers the triggers and the talk.

## The manifest

```toml
[[triggers]]
file = "triggers.toml"
```

A pack may name several triggers files. The triggers of all the packs of a set are used together.

## The triggers file

A triggers file holds one `[[trigger]]` table per trigger. The numbers are decimal.

```toml
[[trigger]]
map = 64                      # 0 to 127: an original map (0 to 63) or a pack map (64 to 127)
rect = [960, 704, 128, 128]   # x, y, width, height in pixels
when = ["flag 3", "not flag 29", "flagx 12", "member 2", "flying"]
scene = 82                    # 0 to 255: the scene to start
```

| Field | Required | Meaning |
|---|---|---|
| `map` | yes | the map the trigger is on, `0` to `127` |
| `rect` | yes | four numbers: `x`, `y`, `width`, `height`, in pixels. `x` and `y` are `0` to `65535`, `width` and `height` `1` to `65535` |
| `when` | no | the conditions, all of which must hold; without it the trigger has none |
| `scene` | yes | the scene to start, `0` to `255`: an original scene or a scene of the pack |

Any other key is an error. An error names the trigger by its number in the file, counted from 1, and the value or the text
that is wrong.

A rectangle holds the leader's position when the position is from its `x` up to, but not including, `x + width`, and from
its `y` up to, but not including, `y + height`. The position is the leader's, in pixels.

### Conditions

| Form | Holds when |
|---|---|
| `flag N` | story flag `N` is set, `N` is `0` to `127` |
| `flagx N` | extended flag `N` is set, `N` is `0` to `2047` (a scene sets it with `flagx.set`) |
| `member C` | character `C` is in the party, `C` is `0` to `11` |
| `flying` | the party is flying (the world map, in flight) |

Each form may be preceded by `not`, as in `not flag 29` or `not flying`. The words of a condition may be separated by any
spaces.

## When a trigger fires

- Each frame, the game checks the triggers of the current map while the party is free: not in a fade, not in a menu, not
  on a town screen, and with no scene running.
- The first trigger of the map, in the order of the set (by pack id, then by the order of the files and the lines), whose
  rectangle holds the leader and whose conditions hold, starts its scene. One scene starts at a time. The frame of the start
  does not run the original's trigger routine of the map, so a second scene does not replace the pack's in the same frame.
- A trigger does not start again while the party stays in its rectangle. It can start again when the party has left the
  rectangle and comes back.
- Every 64th frame the game runs its own encounter check. A trigger that holds on such a frame starts on the next frame.

Two rectangles of one map may overlap. While the party is in both, the first trigger in the order of the set whose
conditions hold starts. While the party stands in the trigger that started, no trigger starts.

## Points of interest on the world map

While the party flies, the world map is map `0`. A trigger with `map = 0` and the condition `flying` is a point of interest:
flying over its rectangle starts its scene. A scene can show a message first, then move the party to a pack map with the
scene command `gomap`:

```
scene fly_in:
    text 403
    zero $01a1
    gomap 0, 64, 896, 648
    end
```

The `gomap` command takes the byte `$164B` (`0` here), the map, and the position the original's `go to map` takes: the
party's leader is placed 128 pixels to the right and 120 pixels down from that position, as the original does.
A point of interest has no marker on the world map in this version.

## Talk on pack maps

On a pack map, an object speaks the message its record gives it: the `message` property of the object in the Tiled map
(see `maps.md`). The original's talk table is not read for it. On an original map the objects speak as before.

## Limits

| What | Limit |
|---|---|
| triggers in a set (all the packs together) | 255 |
| conditions of one trigger | 255 |
| map | `0` to `127` |
| scene | `0` to `255` |
| numbers in `when` | `flag` `0` to `127`, `flagx` `0` to `2047`, `member` `0` to `11` |

A triggers file that is refused stops the pack from loading, and the checks say why.

## What this version does not do

- The extended flags are not written to the save: a new start of the game clears them.
- The state that keeps a trigger from starting again is not saved either. After a load, the trigger the party stood in last
  stays quiet until the party leaves its rectangle.
- A point of interest has no marker on the world map.
- A condition can test only the forms above.
