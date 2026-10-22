# Hidden and conditional events: what the crafted saves reach

The events of `secrets.md` (a research list made from guides, not from the game) were tried in the
emulator, headless, with saves crafted by `xtask save-edit` and short scripted replays
(`tools/saves/secrets.sh`, timelines in `tools/saves/secrets/`). Every replay is checked three ways: the
screenshots of `xtask shots` (read, not trusted), the WRAM at the end (`--wram-out`: map, scene, mode,
group, flags, party records) and a green `verify-replays --hooks original`. The saves and replays are
never tracked; `secrets.sh` rebuilds them from any save with a used slot 1.

How a save is placed just before an event is in `data/save-format.md` ("What identifies a story point")
and `data/scenes.md` (the scenes, the flags, what starts them). In short: the map and the position, the
roster and party, the story flags (`flags=`) and, for the last chapter, three spots of the save image.
The frame counts include the 2 304 frames of title screen and Continue that `replay-from-save` adds.
"Mash" is A pressed 2 frames of every 6: it reads every message and takes the first command in a battle.

## Reached

| Replay | Frames | Event (`secrets.md`) | Seen |
|---|---|---|---|
| `secret-raditz` | 5 304 | S-03: the forced fight with Raditz | the scene starts by itself on Continue (Raditz taunts the party about Gohan), then the battle of group 5 and its rounds; the replay ends in the fight |
| `secret-recruit-krillin` | 3 044 | S-08: Krillin joins | walking up into him; his record is added at level 8, flag `0x10` set |
| `secret-recruit-yamcha` | 3 044 | S-08: Yamcha joins | flag `0x11`, Yamcha added at level 8 (his greeting scene) |
| `secret-recruit-tien-chiaotzu` | 3 064 | S-08: Tien and Chiaotzu join | walking left into them; both added at level 8, flags `0x12` and `0x13` (Chiaotzu's greeting) |
| `secret-saiyan-nappa` | 7 854 | S-13: Nappa, Vegeta and the Saimen | walking up to the Saiyans starts the Saiyan scene (`$11`): Nappa's fight (group `$14`), Vegeta orders him to deal with the party, then the fights of five Saimen (group `$13`); the scene sets the four recruit flags |
| `secret-namek-zfighters` | 2 870 | S-11: Yamcha, Tien and Chiaotzu on Namek | map 11 with Gohan, Krillin, Tien, Yamcha and Chiaotzu in the party; the Status screen lists all five |
| `secret-zarbon-vegeta-joins` | 5 904 | N-03: Zarbon, Vegeta joins | the same five on map 11: scene `$2E` runs, Vegeta joins at level 25 (a sixth record), then the fight with Zarbon (group `$38`) with the five Z-fighters |
| `secret-dende-rescue` | 8 304 | TR-02: Dende's rescue | map 9: the scene in Frieza's camp (the Namekians beg for the children), Dende joins at level 1, then the flying chase minigame (mode `$0A`), then the map |
| `secret-maima-tsumuri` | 5 190 | TR-04: the fight with Maima and Tsumuri | map 9, talking to the two Namekians with Piccolo and Dende in the party: the battle of group `$26` (Dende rests in it) |
| `secret-cui` | 5 304 | TR-05: Cui | the fight of group `$24` (Cui's energy beam) with Goku, Gohan and Krillin |
| `secret-dodoria` | 5 304 | TR-06: Dodoria | his scene and the fight of group `$25` with Goku, Gohan, Krillin and Vegeta |
| `secret-fake-nail` | 5 304 | N-02: Nail and the other Nail | map 34: Nail joins at level 25 and fights the second Nail (group `$6D`) with Gohan and Vegeta |
| `secret-ginyu-jeice-burter` | 5 904 | N-07: Jeice and Burter | map 16: the scene starts by itself, the fight of group `$4E`; Krillin is knocked out in it |
| `secret-ginyu-goku-duel` | 7 104 | N-08, N-09: Goku's arrival, Goku against Ginyu | Goku's spaceship lands (he announces himself), the party becomes Goku alone and the battle of group `$51` starts |
| `secret-nail-porunga` | 10 704 | N-13, N-14: Nail against Frieza, the dragon balls, Porunga | the Z-fighters decide to revive Piccolo, the Guru's house scene, Nail's battle against Frieza (group `$57`: Nail alone, 42 000 BP) lost to Frieza's attacks, the dragon balls, Porunga rising, Dende's first wish (to revive the dead) and the dragon asking for the second, then the next chapter's map (Dende has joined) |
| `secret-frieza-forced-battle` | 5 784 | F-02: Frieza forced battle | map 22 with event bit 0 and five battles counted: the first random battle is Frieza's first form (group `$53`, scene `$42`) |
| `secret-frieza-random-encounter` | 5 784 | F-05: Frieza on the last map | map 24: with Goku in the party and the counter at 3 a random battle is Frieza's third form (group `$59`, scene `$3D`) |
| `secret-ssj-frieza` | 9 300 | F-06: Goku becomes a Super Saiyan | the last fight (group `$56`, Frieza's final form) with Goku, Gohan, Piccolo, Krillin and Vegeta; Piccolo and Krillin, at 1 hit point, fall: Goku's record becomes unit `$3B` with 4 999 999 BP, golden hair, level 48 ("the final form's power-up message) |
| `secret-ending-super-vegeta` | 19 302 | F-08, E-01, E-02: the ending and the hidden boss | Frieza dies at once (1 hit point, Goku already a Super Saiyan); the epilogue captions (Frieza defeated, Namek restored by the dragon), the staff credits to the END card, then Goku and Vegeta in the air: Vegeta boasts of the true Super Saiyan power and the battle of group `$6F` (Super Vegeta) starts |
| `secret-ending-normal` | 19 302 | E-01, E-03: the ending without the hidden boss | the same without Vegeta in the party (E-03: he must be there): the captions, the credits and the END screen, no hidden fight |
| `secret-piccolo-dende-fusion` | 4 294 | F-03: the Piccolo fusion | map 24, talking to the two Namekians with Piccolo, Gohan, Vegeta and Dende in the party: the flash, "Fuse with me, too!", Piccolo's power-up (level 40 to 48) and Dende gone from the party (the Status screen shows three records) |
| `secret-piccolo-returns` | 8 560 | N-14: Piccolo back in the party | map 22: Frieza's first form (group `$53`) is fought until it ends, then scene `$38` ("I've arrived!") makes the party Piccolo, Tien, Yamcha and Chiaotzu (the Status screen at the end) |
| `secret-piccolo-snake-way` | 6 084 | S-06: Gohan's practice fight against Piccolo, then Goku's way to King Kai | map 3 (Piccolo's cave): talking to Piccolo starts scene `$04` ("Six months have passed", "Now, you will fight me!"), the practice fight (group `$1D`, a cave background) won by Gohan at level 30, Piccolo's last words, the Snake Way picture (Goku with his halo on the road), the flight to King Kai's planet and the arrival there (map 7) |
| `secret-king-kai-planet` | 6 954 | S-05: Goku at King Kai's | map 7, Goku dead with his halo: he walks left, right, up and down on the little planet, King Kai (from above) greets him ("Bravo, small one"), the fight against King Kai in the sky (Goku at level 40 wins), the Shenron picture, Goku's flight and the Earth chapter's first battle |

## Save recipes

Every recipe is one `--set` list on the base save (any first-chapter save). `tools/saves/secrets.sh`
is the executable form; the fields are repeated here to read the doc alone. `flags=` lists flags set (`+n`) after
clearing all (`none`); `lvl` is `<character>.level`; `scene=0` is always set.

| Replay | Script | Fields |
|---|---|---|
| raditz | mash-3000 | `goku.level=12 piccolo.level=12 flags=+3` (map 0, the base's position) |
| recruit-krillin | recruit-krillin | `party=gohan,piccolo` lvl 8, `map=5 flags=none,+1,+2 leader=680,690 follower=552,570` |
| recruit-yamcha | recruit-yamcha | the same, `leader=920,940 follower=800,820` |
| recruit-tien-chiaotzu | recruit-tien-chiaotzu | the same, `leader=1320,1784 follower=1192,1664` |
| saiyan-nappa | saiyan-nappa | the same with Gohan and Piccolo at level 30, `leader=1316,1200 follower=1188,1080` |
| namek-zfighters | status | `party=gohan,krillin,tien,yamcha,chiaotzu` lvl 24, 22, 22, 20, 18, `map=11 flags=none,+1,+3,+4,+5,+6,+0x10,+0x11,+0x12,+0x13 leader=768,424 follower=640,304 items=d.radar,scouter zeni=300` |
| zarbon-vegeta-joins | mash-3600 | the same party at lvl 30, 28, 28, 26, 24, `map=11 flags=none,+1,+2,+3,+4,+5,+6,+0x10,+0x11,+0x12,+0x13,+0x1B leader=768,1000 follower=640,880` |
| dende-rescue | mash-6000 | `party=gohan,krillin` lvl 20, `map=9 flags=none,+1,+3 leader=880,320 follower=752,200` |
| maima-tsumuri | talk-maima | `party=piccolo,gohan,dende` lvl 47, 40, 3, `map=9 flags=none,+1,+2,+0xB leader=2436,624 follower=2308,504` (the script walks up 28 frames and talks) |
| cui | mash-3000 | `party=goku,gohan,krillin` lvl 40, 35, 32, `map=9 flags=none,+0,+1,+3,+0x1E leader=1200,1000 follower=1072,880` |
| dodoria | mash-3000 | `party=goku,gohan,krillin,vegeta` lvl 40, 35, 32, 36, `map=9 flags=none,+0,+1,+3,+7,+0x1B leader=336,896 follower=208,776` |
| fake-nail | mash-3000 | `party=gohan,vegeta` lvl 35, 36, `map=34 flags=none,+1,+7 leader=120,150 follower=0,30` (flag 3 must be clear) |
| ginyu-jeice-burter | mash-3600 | `party=gohan,krillin,vegeta` lvl 30, 27, 28, `map=16 flags=none,+1,+3,+7 leader=532,1148 follower=404,1028 items=d.radar,scouter` |
| ginyu-goku-duel | mash-4800 | `party=goku,gohan,krillin,vegeta` lvl 40, 30, 27, 28, `map=16 flags=none,+0,+1,+3,+7,+0x1F,+0x12 leader=1860,800 follower=1732,680` |
| nail-porunga | mash-8400 | `party=gohan,krillin,vegeta,dende` lvl 30, 27, 28, 3, then `party=gohan,krillin,vegeta` (Dende keeps a record), `map=16 flags=none,+1,+3,+7,+0x1F,+0x22,+0x1A leader=1792,768 follower=1664,648` |
| frieza-forced-battle | walk-fight | `party=gohan,krillin,vegeta` lvl 35, 32, 36, `map=22 flags=none,+1,+3,+7 page:83=01 page:B0=0500 leader=1500,1500` |
| frieza-random-encounter | walk-fight | `party=goku,gohan,piccolo,vegeta` lvl 47, 40, 40, 40, `map=24 flags=none,+0,+1,+2,+7 page:B0=0300 leader=2000,2000` |
| ssj-frieza | mash-7000 | `party=gohan,piccolo,krillin,vegeta,goku` lvl 40, 40, 30, 40, 47, `piccolo.hp=1 krillin.hp=1 map=24 flags=none,+0,+1,+2,+3,+7,+8 page:B0=00 leader=2100,2592 image:2A0=01020307FF image:303=3C00 image:30A=3C00` (flag 8, a square of the map, no battle counted, the party list and Frieza's hit points of the image) |
| ending-super-vegeta | mash-17000 | `party=gohan,piccolo,vegeta,goku` lvl 40, 40, 40, 47, `image:01=3B goku.bp=4999999` (Goku a Super Saiyan), `map=24 flags=none,+0,+1,+2,+7,+8 page:B0=00 leader=2100,2592 image:2A0=010207FF image:303=0100 image:30A=D007` |
| ending-normal | mash-17000 | the same without Vegeta: `party=gohan,piccolo,goku`, `flags=none,+0,+1,+2,+8`, `image:2A0=0102FF` |
| piccolo-dende-fusion | talk-fusion | `party=piccolo,gohan,vegeta,dende` lvl 40, 40, 40, 3, `map=24 flags=none,+1,+7,+0xB leader=1664,1172 follower=1536,1052` (flag 2 must be clear, flag 3 clear so that Dende leaves; the script turns up one frame and talks) |
| piccolo-returns | frieza-flees | `party=gohan,krillin,tien,yamcha,chiaotzu,vegeta` lvl 35, 32, 30, 28, 26, 36 then `party=gohan,krillin,vegeta` (the three others keep a record), `piccolo.level=40 map=22 flags=none,+1,+3,+7 page:83=01 page:B0=0500 leader=1500,1500 follower=1372,1380 image:1A0=040506FF` (the list the set-up would have kept; without it the party after the scene is garbage). Dende must not be in the party: the mash gets stuck on his "He's resting" command |
| piccolo-snake-way | talk-piccolo | `party=gohan` lvl 30, `map=3 flags=none,+1 leader=128,52 follower=128,90` (the script turns up and talks; Piccolo stands at x 128, y 32) |
| king-kai-planet | talk-king-kai | `party=goku` lvl 40, `map=7 flags=none,+0 leader=128,60 follower=128,60` (King Kai stands at x 128, y 80 and blocks the road: talk to him from above or from the left; from below the game says "Nothing there!") |


## Not reached

- **N-10, Captain Ginyu** (the body swap), **N-15** (the Porunga card), **F-01** (Nail's fusion), **F-07** (the
  Guru card on Goku): need long battle states; not tried.
- **S-07, TR-08, TR-09, N-05** (Goku's arrival timing, the gravity room's trials, Piccolo's training on map 1):
  scenes `$43`-`$4C` and the gravity-room maps were not entered. King Kai's planet itself is reached
  (`secret-king-kai-planet`), but Goku's fight against King Kai is the only trial it plays.
- **S-09, S-10, S-12** (Piccolo's level 9, the deadlines, Piccolo's death): the level rule lives in the map 5
  routine; it was run on a crafted save (`secret-recruit-krillin` with `piccolo.level=9`: flag `0x1D` set and the recruits
  gone at the first check, `free-progression.md` section 7); the death is inside the Saiyan scene `$11`, which
  `secret-saiyan-nappa` only starts.
- The friendly outcome of the Maima and Tsumuri conversation (TR-04: they join and give the 2-star ball; the
  replay shows the fight) and the Dragon Radar cases (TR-01).

## How the saves were found (for the next one)

1. Pick the event in `scenes.md` (scene, map, flags, position). 2. `save-edit` a base save with them. 3.
`xtask replay-from-save` with a mash timeline, then `xtask shots <replay> --every N --sheet 8` and read the
sheet; `--watch 7E01D8` says when the scene id changes. 4. If a scene crashes (black screen, noise, a hang),
the state was incomplete: the last chapter needed the party list and the enemy's hit points in the image.
5. Cut the timeline where the event is shown, `verify-replays --hooks original` on it.
