# Dragon Ball Z: Super Saiya Densetsu Recompiled - developer guide

This is the reference for building, running and checking the project: the command line of the game and
the `xtask` developer commands. The overview, the install steps and the roadmap are in the main README.

A native Windows reimplementation of *Dragon Ball Z: Super Saiya Densetsu* (SNES, 1992) in Rust:
faithful first (verified against the original running in an emulator), enhanced on top.

You need your own ROM. Nothing from the game is included in this repository.

## Build and run

    cargo run --release -- --rom "path/to/rom.sfc"     # first launch: identifies and caches the ROM
    cargo run --release                                  # later launches use the cache

Accepted inputs: Klepto v1.02 English translation (with or without checksum fix), or the Japanese Rev 1 ROM
plus `--ips path/to/dbz102.ips`; a 512-byte copier header is stripped automatically.

The HD art pack of a release (`hd-pack.zip`, next to the executable) is installed on the first `--mode enhanced` or
`--hd-scale` launch, or with `--install-hd-pack <zip or folder>` (see `hd-pack-public` below). The build scripts
`tools/build-classic` and `tools/build-recompiled` (PowerShell and bash) build the game and lay out the folder of a
release (`tools/windows/`: the launchers, the quickstart, the licences).

Enhanced graphics (M6): `--hd-scale N` (2 to 8) draws the picture again at N times the PPU's resolution, from the state
the PPU drew it from, in the window and with `--ui-snapshot`. `--enhanced-look crisp|smooth` picks the look: `crisp`
(the default) is pixel art made bigger with smoothed diagonals, `smooth` a bicubic filter. Without `--hd-scale` the
Classic picture (the PPU's own frame) is shown; a verification run always shows the Classic pictures. Mode 7 planes
are sampled again at the picture's resolution. Use `--ui-size 1170x956` for a snapshot at 4x: the frame's 8:7 pixel
aspect makes the picture 1170 pixels wide. With `--mode enhanced` the picture is the 16:9 view (372 pixels wide, black
beside a battle, a title or a room) drawn at that scale: 1488 x 956 pixels at 4x.

HD art: when the pack of the cached ROM holds art (`xtask hd-import`, below), the Enhanced picture draws it: the
portraits and faces on the field and in battle, the fighters' poses and the effects of the battle, and the HD Item
screen's portraits. `--hd-pack DIR` names another pack folder, `--no-hd-art` draws the look alone. The field's sprites
(the party, NPCs, ships and objects) are found as groups of the visible sprites (runs of one OAM palette, split into sets
within 2 pixels of each other) and drawn the same way, recoloured to the palette in use; a group cut by the edge of the
view is drawn from the group of the index that holds it, and a group that runs under line 224 (where the art's capture ends, the 239-line view shows the band below) takes the sprites of its palette that touch it there, when the index holds art for the whole figure. In the 16:9 view the groups are found over the whole view, so an
object in the margins (one the original never draws, which the wide view adds beside its window) takes the same art,
recolouring, mirroring and soft shadow as one inside the original window, and one across the window's edge is one group
(its parts keep the order of the original's loop whatever side of the edge they lie on), so nothing changes when it
crosses. A ground shadow (an effect of one dark flat colour in the shape of an
ellipse, below the rest of its group: never a dark part of a character such as hair; or the command screen's BG1 layer of
one dark flat ellipse) is drawn again as a smooth ellipse of the original's box and centre, with a feathered anti-aliased
edge, under the fighters and whatever the art index holds for it. It darkens the HD picture by multiplying it with the light
the original's shadow left against the picture around it (never below 14 percent, so a black shadow still shows the water's
ripples); a silhouette that is not an ellipse keeps its own shape and colour through a soft alpha. The Enhanced picture is shown
with linear filtering, so the art's anti-aliased edges survive a window that is not a whole multiple of it. The sprites that start under line 224 (the 15 lines that a frame of 239 lines shows below the original's 224: the ending's speed lines reach into
them) are effect poses of their own, found by the exact pixels the index has art for (the capture of the art kept no sprite there; they are clustered on
their own so that the poses above them keep their keys). The 16:9 picture is drawn in segments of lines on the thread pool (the
same picture as in one piece; `SSD_HD_SERIAL=1` in the environment draws it in one piece, to compare).

HD text: at a scale of 2 or more the game's own UI text (dialogue, menus, names, item and shop lists, the HP/KI/BP
numbers, the ending captions: every tile of the English font the game draws on a 2-bit layer, found by its content) is drawn
again from Share Tech Mono (SIL OFL, `assets/fonts/`), anti-aliased at the picture's scale, one glyph per 8-pixel cell in the
colours the game uses for it. The title set (the stream `00:DF39`: the title screen's lines, the intro's "LONG, LONG AGO"
text, the credits) is drawn the same way: its white letters take one 8-pixel cell, its bold names two cells stacked, and the
bold letter keeps its two colours (the top half in colour 2, the bottom half in colour 3). The 1x picture is untouched.
`--no-hd-font` draws the game's own letters.

HD battle backgrounds: when the pack holds background art (`xtask hd-import`, `bg\` folder) the Enhanced picture draws the
battle's background layers again from it: the command screen's picture (BG2), the animation scene's far layer (BG4: sky,
clouds, sea) and near layer (BG3: the ground band) and the flights' and charges' single-screen backdrops (BG4). A layer is
found by its content (its tile map and the tiles the map uses, `ssd_hd::bgart::layer_key`), whatever its scroll; the art is
one picture of the whole layer, and each pixel of the screen takes the picture's pixel that its line's scroll registers
(so an HDMA parallax or two layers at different speeds work as the tiles do) point at. The picture replaces what the tiles
drew where the winner of the pixel is the layer, the backdrop or an object that an HD pose hides; windows, priority and
colour math are the pixel's own. The game shows some layers in several palettes (the same cave in ochre, blue, green, red,
purple and grey, a speed-line backdrop in four colours): a layer of the index that is a palette variant (`palette_key` of its
entry: `ssd_hd::bgart::palette_key`, a hash of the palette rows its tile map reads and of the backdrop) is the art of the
content made for that palette, and it is bound first when the frame shows the layer in exactly those colours; the art of the
content with no `palette_key` is the fallback. A palette that is not the one the art was made for (a fade, a flash, a cycle, a
variant with no art of its own) is drawn through one smooth colour transfer of the art's colour per layer and frame
(`ssd_hd::transfer`), built from the pairs (the colour the game's pixel had where the art was made, the colour it has now) of
the pixels drawn: the same palette draws the art as it is; a fade (every colour scaled towards black, or lightened towards
white, by one factor) is a plain blend of the art; any other palette is a table of the art's colours (Shepard's inverse
distance blend of the pairs' offsets, each scaled by the art's brightness against its reference's, read with trilinear
interpolation), so the result is a function of the art's colour alone and a smooth painted gradient stays one in any palette
(the colour of the pixel under each art pixel never reaches the picture, so the original's dither and blocks do not).
A layer with mosaic (a fade-in's blocks) is drawn as the
game draws it: each block of the mosaic is flat, one point of the art at its top left pixel's place. A frame that loads VRAM
while it is displayed is drawn from the tiles. HD text and cards are not drawn over a pixel of a background the art covers.

A portrait is only ever drawn from its own art: a picture of the index covers exactly the characters it was made from (found by
their content), in the palette it was made for or recoloured to the one in use; sprites that show a picture (the item picture of
a shop, a face drawn as objects) are never replaced by a pose of the same silhouette, and a picture the pack has no art for stays
the original's pixels. The HD Items menu finds the picture of an object the same way, by the object's characters
(`Pack::menu_picture`), not by its number: objects share the sets of their pictures in pairs (`docs/re/data/menus.md`).

HD pictures: the same mechanism draws the full-screen pictures that the game builds from BG layers: the title's logo and
subtitle, the intro scene's streaked backdrop, the story scenes' scenery, characters and light effects (Frieza's soldiers,
the capsule's starfield, Porunga and his pillar of light) and the END screen. They are `bg\` layers like the battle's
(any BG, mode, tile map, character base and size; `rows` bound the picture; a layer of 4-bit tiles with transparent
parts has `fill` false and art with an alpha). The art is found by the content of the layer, so a picture is drawn from its
art in every scene that shows it, and the colours of a fade or a flash follow the game's pixel. Objects over a picture
(the fighters, Goku powering up, the capsule, the portraits) are the object art's.

HD Mode 7: when the pack holds Mode 7 art (`xtask hd-import`, `mode7\` folder) the Enhanced picture draws the Mode 7 plane of the flights and of the
last fights again from a redrawn picture of the whole plane (128 x 128 tiles, 1024 x 1024 game pixels, at 4x, RGBA). The plane is found by the content of its
VRAM (`ssd_hd::mode7art::plane_key`, a hash of the tile map and the 8-bit characters) and a picture pixel by the game's own matrix of its line (HDMA changes it
from line to line: the perspective of a flight is the original's lines, not a stretch): the matrix is applied to the finer position, the art is sampled
bilinearly there, and a plane that is far away on a line (the horizon) is read from a smaller copy of the art, two copies blended, so it does not shimmer. The
view is the PPU's picture wherever the plane is not the top layer, wherever the art has no pixel (alpha 0: its edges are soft) and wherever the frame loads
VRAM while it is displayed; a palette that is not the one the art was made in (a fade, a flash) changes the art as it changed the game's pixel, as the
backgrounds' art does, and the colour math and the brightness of the pixel apply. It works in the 16:9 view, where the plane continues past the window where it has texels (where it has none, as beside a picture drawn on the plane, the sides are black like those of any picture). What
the flights show is not a world map: they are the cutscenes' planes, a character flying over a flat backdrop (a small picture on the plane) and three
landscapes of 256 x 256 pixels (a grassland, a coast and a lake, and a view of mesas under a sky). The title's two planes (`$87`) are in the art too: the
logo that zooms in (the art of the title logo's picture layer, which is the same drawing at the same place on the plane) and Goku and Gohan on the Flying
Nimbus, whose close-up is the same plane magnified by the game's matrix.

HD cards: when the pack holds card art (`xtask hd-import`, `cards\` folder) the Enhanced picture draws the battle cards
again at a scale of 2 or more (it needs the HD text pass, so not with `--no-hd-font`): the black body in its cream frame,
the Dragon Balls with their stars or Z and the kanji numerals, the coloured square with the card's kanji. The game builds
a card from tiles of its UI set, so the art is made tile by tile and covers every combination of stars, kanji, numeral and
colour (`docs/re/data/cards.md`, "How a card is drawn"); the kanji, numerals, Z and stars take the colours of the game's
palette; the Z is the original's own stylised Z, vectorised from its pixels (corners and serifs kept, the staircases cut to
straight runs), and the numerals fill the box the original's fill. While the pick cursor (the arrow above the hand) is on a
card a small window in the game's style says what the card's kanji means (for instance "Kami / God"; the table is in
`docs/re/data/cards.md`); it sits above the hand and never covers the original's own message for a favourite card ("Attack
everyone!", which the original shows to the right of the card under the cursor). Classic is untouched.

The Cheats screen's "Next card" preview is the same card: `ssd_hd::CardArt::preview` composes it from the tiles of a card (a 4 x 6 grid of the
codes the battle's hand uses: the attack ball with four dots or the Z, the symbol 必, the defence ball with the numeral seven or the Z) at 8x, and
the app paints it as one picture in the preview's box (`Op::Card` of the draw list); a pack with no card art keeps the pixel-style card
(`ssd_menus::screens::pixel_card`).

The same pass draws the art of other tiles of the 2-bit sets, by their content: an entry of the cards' index may be a tile of the title set
(`"set": "title"`, bound in both forms the game's VRAM holds it in) and may carve its shape out of the tile's colour 1 (`"carve": true`: the art is
drawn over the tile's own background colour, so a shape need not cover every pixel the tile shows). They are the window frame's pieces (UI set
tiles 50 to 54 and 59 to 61: the credits' picture frame, and every window built from them) with their rounded caps, and the credits' green
separators (title set tile 203), both drawn by a program from the tile's pattern in its surroundings; the glyph indexes of the art start at 104,
which makes room for 152 tiles.

HD maps: when the pack holds map art (`xtask hd-import`, `maps\` folder) the Enhanced picture draws the field's map layer
(BG2: the Earth and Namek maps, the caves and the rooms) again from a redrawn picture of the whole map, at 4x and in the
game's colours, with the same layout (every wall, path, shore, tree and house stays on the original's grid). The layer is
found by the pointers of the map's data (`ssd_hd::mapart`: every map that shares them shares the art, whatever palette it
is shown in) and a pixel by its world position, so the art is drawn wherever the map shows: in the original's window and
in the margins of the 16:9 view, whatever the camera. The art is cut into tiles of 256 x 256 game pixels and read as the
frames show them. A fade, a flash, the water's palette cycle and a map shown in another palette (the same cave in blue and
in red) change the art's colours as they change the game's (`bgart::adapt`); the sprites (party, people, houses) are the
field sprites' art. The mechanism is the battle backgrounds' (`ssd_hd::bgart`): a field map is a
layer slot whose position is the world position instead of the scroll registers'. A picture that shows the field's map on BG2
(a menu laid over the map, a scripted scene on it, a town's room: the main loop is not in a field state, so the picture keeps
its 256-pixel window with black beside it) is drawn from the same art when BG2 holds the tiles the map lookup gives at the world
position the scroll registers and the camera say (`ssd_hd::world`, `shows_map`).
A town's rooms are maps 48 to 63 of the same table (1 x 1 blocks); the game does not load their header into work RAM while the party is in
the town, so `FieldState::in_room` reads the size and the grid from the table record and the art is found like any layer. The field is main loop state `$00`
(after a load, a battle or a scene), `$06` (after a flight) or `$05`: the map the party walks back to after a building
(`$01`), where it stays until the next battle, so the art is drawn in all three.

## Keys

Arrows = D-pad, X = A, Z = B, S = X, A = Y, Q = L, W = R, Enter = Start, Right Shift = Select.
F5 = save state, F9 = load state, 1–4 = state slot, Tab = cycle speed ×1/×2/×4/×8, P = pause, Esc = quit.
F1 = show or hide the debug panel (see below). F12 = save a screenshot as a PNG (see "Screenshots"). These are the defaults: in
Enhanced mode the Controls page of the Options remaps the buttons and the five shortcuts (save, load, speed, pause,
screenshot), keyboard and gamepad apart (see "Options (M7)").

The debug panel docks to the right of the game. Closed (the default), the window shows only a small "F1 debug" label
top-right and, when a verified routine diverges, a short message bottom-right. Open, it has a header (frame, frames
per second, share of the instructions of the last 60 frames that ran as Rust, and the hooks mode: Rust, Verify or
Original, changed live), the four tabs described below, and a controls bar under the game: pause or continue, next
frame while paused, the steps while paused, speed ×1/×2/×4/×8, and save/load for slots 1–4. While a text field of the
panel has the keyboard, the game and the hotkeys do not see the keys. The fonts are IBM Plex Sans and JetBrains Mono,
under the SIL Open Font License (`assets/fonts/`).

A standard gamepad works too, with the SNES layout by position: D-pad, East = A, South = B, North = X, West = Y,
shoulders = L/R, Start = Start, Back = Select.

## Enhanced mode

`--mode classic` (the default) is the original's rules and visuals, the reference every replay is verified against.
`--mode enhanced` is the game with its named differences; the first is the HD menus. A replay recorded in Enhanced mode
(format 2: the mode, then each frame's joypad and host events) needs `--mode enhanced` to play, and Classic replays stay
valid and unchanged.

### HD menus (M4)

In Enhanced mode the map menus are drawn at 1280x720 in the original's style (cream windows in an orange frame, the
Share Tech Mono font of the game's own HD text, DotGothic16 for the two kanji of the cards, the original portraits
upscaled) over the field as it was when the menu opened: the command window of
the field, the towns and the flying party with its help window, the corner portraits with their HP and KI bars and the
Zeni window; the Menu list (Cheats opens the cheats screen below; Options opens the options screen, see "Options (M7)"); and the Item screen.
While an HD menu is up the keys belong to it: the arrows move its cursor, X confirms, Z goes back and Q or W changes
the bag's tab (the game gets no buttons until the original draws a message or a screen of its own, such as Cards, Save
or the take-off). The original menu keeps running underneath and performs every consequence (Talk, Look, the take-off,
saving, the effects of the objects): the HD only presses its buttons, so Classic stays byte-exact. The HD window goes down
as the last press of Talk, Look, Fly, Land or a Menu entry goes (the original shows its own window, message or scene from
there, and the press is taken when the original answers, a message that holds the field included). While the HD plays
a choice (a few seconds), its keys wait for it to end. Options is greyed out: A on it does nothing, and its help line
says "Coming soon."

Enhanced mode is shown at 16:9: every frame is drawn 372 pixels wide, the original's 256 in the middle (where the view
stops at a map's edge the picture moves toward it, and the frame still fills the width). The field shows its map beyond
the window, a Mode 7 flight its plane, and the battles, the title, the cutscenes and the interiors have black at the
sides. The HD menus are drawn over that frame. The view is read from the PPU's render log of each frame (`ssd-hd::world`),
and it never changes what the game does.

The Item screen is a bag of 60 slots with up to 99 units in a stack (both options later), two columns with `x NN`
counts, the tabs All, Allies and Tools, the picture, the Pt price and the help of the object under the cursor. A use
takes one unit, as the original takes it, with no quantity: the HD stages the stack on the original's list and presses
the original's own Use, and that is all it presses. From then the curtain lifts and whatever the use asks next is the
player's, on the original's own screens: the fighter it is for, a card to change, a wish, the lines of a message, the
effect; the Dragon Radar's screen stays up until B leaves it. The HD's Item screen comes back when the original is on
its item list again, and goes when the menu closes. An object the original
refuses (Saiba) is greyed out; one refused at the moment of use (Mr. Popo or Puar at full KI) keeps its unit and the
menu stays open for the player. Drop asks Yes or No and drops one unit. A tab with no stack says so when A is pressed
on it. The Dragon Balls and the story's objects are pinned at the top and cannot be dropped, and a full bag refuses
what it is offered, with a notice. The bag lives in `saves/ssd.bag` beside the battery save (one bag per save slot,
keyed by the slot's checksum: a save changed by Classic play rebuilds its bag from the game's list; a file that cannot
be read is kept as `ssd.bag.bad` and the bags are rebuilt). Loading a state rebuilds the bag from the game's list.
In a battle the Item command opens the same Item screen over the battle: the bag with its counts and pages, the
stacks a battle can use in full colour and the others greyed out (the original answers "can't use" for those). A use
stages the chosen stack first on the original's list, which takes it through its own battle item code, and the steps
after the Use (the fighter, the messages, the effect's A) are the player's, as on the map; the battle's command window
coming back ends it. Back goes to the battle's command window. A battle offers no Drop.

The pictures: a picture box shows its object's picture (the HD art when the pack holds it, the original's pixels
otherwise), and stays empty, never with a placeholder text, when nothing is under the cursor. The corner portraits are
the original's faces: Goku, Piccolo, Gohan, Krillin, Tien, Yamcha, Chiaotzu, Vegeta and Dende are drawn from their
picture sets in the corner palette; Nail, Tsumuri and Maima have no face yet, as no party with them has been seen. The
objects with no price (the story's helpers and the Dragon Balls) show no figure, where the original draws a kanji mark
that the menu does not draw yet. Four objects (Yemma, Baba, 27 and Goku) have a run-time slot in their help that the
original fills from a message: the menu shows an ellipsis there. `menu-test items` checks all of this (the `pictures`
check names each gap).

### Cheats (M5)

In Enhanced mode the Menu list's Cheats entry opens the Cheats screen (original style, over the field; the original's
Menu list stays open unseen underneath). Up and Down choose a row, Left and Right (or X) cycle its choices, X on Reset all
turns everything off, Z goes back. The rows: damage dealt x1/2 to x10, defence x1/2 to x10, EXP x1 to x10, Z cards
(Off, Attack, Defence, Both; for the whole party or one character), maximum zeni, zeni gained x1 to x10 (from selling and
from the story's events: the wrapper of the sale `03:A790` adds the rest of the price first, the money event's scales its
amount; the pair `GAIN` lies after the money's in the block, and a zero reads x1), endless items, the item spawner (A opens
its page: the 40 objects by page with the units the bag holds and each one's help; Left and Right choose how many, up to 99,
L and R turn the page, Y goes back to 1, A adds them to the bag through the keeper, `Keeper::spawn`; the objects the story
checks, the bag's key items and the "Moon" card and the Dragon Radar, are red, one at a time, and A first asks "Add anyway"
or "Cancel" with the cursor on Cancel; an add is the application event 3, `SPAWN_TAG`, so a recording replays it, and it
marks the game, `Cheats::spawned`, whose saves keep the CHEATS badge even after Reset all), random battles on or
off, how often they come x1/4 to x4, enemies per battle x1/2 to x3, enemy power x1/2 to x5 and, under Story, Unlock chapters
(Off or On: every chapter of the data packs is offered, finished game or not; a save with it on is marked like the other
cheats). A card previews the Z balls
and an "On: N" box counts the active choices.

Story fix (Remastered, with the cheat wrappers, always on): the level deadline of map 5 (`03:F300`, Piccolo at level 9, or
Gohan at 10 without him) kills the fighters not yet recruited and stops the random battles; the wrapper of its body
(`03:F320`, `story_level_deadline`) returns at once, as the routine does for a lower level, so the story never moves on a
level. The deadline of walking into the Saiyans stays. `menu-test cheats deadline` runs the routine with Piccolo at levels
8, 9 and 30 in Classic and Remastered.

The cheats act in the running game only: Rust wrappers on top of the routines they change (`ssd_game::cheats`) read their
numbers from a 32-byte block of work RAM at `$7F:FF00` (outside the save image), which the app writes as host events
(recorded in a format 2 replay, so a replay with cheats plays to the same state). `Ruleset::classic()` installs nothing,
and at a neutral block every wrapper is the original routine, so Classic stays byte-exact (`verify-replays
--cheats-identity` installs the wrappers at a neutral block and checks the replays are still identical). Fights the story
expects the party to lose (`SCRIPTED_LOSS`, today Nail against Frieza) suspend the battle cheats: the screen shows "Cheats
off for this fight" for about two seconds and "Cheats back on" on the map. Other story fights are not gated: Frieza's first form on map 22 is won and followed by the same scenes with the cheats (a recording made for Classic ends at another frame there). `docs/re/data/battle.md` has the hook points and
the group verdicts.

Each save slot keeps its cheats in `saves/ssd.cheats` beside the bag (keyed by the slot's checksum word: Continue restores
them, a new game or a slot changed by Classic play starts with all off; a file that cannot be read is kept as
`ssd.cheats.bad`). A save made with a cheat on is marked with a CHEATS badge over the original's slot lists (the title's
Continue and the Menu's Save). Every Save writes the entry of the slot it chose, also when the save's data is the same as
before (the Save's completion is the event; the checksum word only keys the entry). A recorded session carries the settings
as application event 2 (`CHEATS_TAG`): a replay sets the run's cheats on playback, and never writes `saves/ssd.cheats`.

    cargo run --release -p xtask -- menu-test cheats [--save <file.sav>] [--replays <dir>] [name...]
    cargo run --release -p xtask -- verify-replays --cheats-identity [dir]
    cargo run --release -p xtask -- cheat-spike <dir>

`menu-test cheats` plays the cheats headless on the reference replays and crafted saves: the battle numbers, the Z cards,
zeni, random battles off and the encounter rate, recorded sessions that replay to the same hash, the Cheats screen driven
by its keys (rows, wraps, a battle, persistence through Save and Continue, a second Save of the same data: `screen-resave`)
and the scripted loss of Nail (with `--save`).
`cheat-spike` plays the `secret-*` replays of a directory in Classic and with extreme cheats (the group verdicts of
`battle.md`). `replay-from-save` takes `--title` (stop at the title's slot list), `--cheats` (write extreme cheats into
the block at frame 300, Enhanced mode) and `--bind FRAME` (the save's item list is recorded as the bag bound at that
frame, in Enhanced mode, so the bag of a replay is the save's; the units keep state word 0), and `menu show cheats` in a UI script checks the screen.

### Options (M7)

In Enhanced mode the Menu list's Options entry opens the Options screen (original style, over the field; the original's Menu
list stays open unseen underneath, as for the Cheats). A category list on the left (Difficulty, Gameplay, Video, Audio,
Controls, Accessibility, each labelled "Saved with the game" or "This PC") (on Android a seventh page, Game, labelled "This device", see "Android (M9)"), the rows of the chosen category on the right, and a
help strip with the key hints. Up and Down choose a row, Left and Right (or X) change it, L and R change the page, Y is Reset
this page, Select is Reset all (a Yes/No question, the cursor on No), Z goes back. The rows so far: Gameplay has the text
speed (the original Menu's Text entry: the same byte, `$01D0`, five speeds) and the random-encounter rate (the Cheats
screen's rows Random battles and How often); Video is applied at once, no restart (see below);
Difficulty has the preset (Easy, Normal, Hard), the three values it sets and a button that opens the Cheats screen. The pages of the later tasks show a note until their rows land.

The Gameplay page (saved with the game, per slot in `saves/ssd.options`; keys `gameplay.auto_run`, `gameplay.auto_wait`,
`gameplay.anim_speed`): Messages has the Text speed (the Menu's Text entry, the byte `$01D0`: 0 writes the most characters a
frame and 4 the fewest, so the row reads Slowest, Slow, Normal, Fast, Fastest; five steps as the original), Auto-run messages
(Off, On) and its wait (Short 0.75 s, Normal 1.5 s, Long 3 s); Battle has the animation speed (Normal, x2, x3, x4); Exploring
has the encounter rate. Auto-run: when a message has written its last page, or a page that waits, and shows the arrow for A
(`$071B` = `$82` or `$83`), the driver puts A into the joypad of the frame once the wait is up, for three frames (a press of the
player's own starts the wait again); it is a joypad bit of the frame like any other, so a recording carries it and a replay plays
it. The animation speed runs 2 to 4 machine frames for one frame on the screen during the battle's attack animation only (the
sound of the first is played), so the machine, the turn order, the numbers and the random draws are the original's; shortening
the waits inside the game would shift the free-running random generators (`docs/re/data/gameplay.md`). Neither installs a hook:
Classic is untouched. `menu-test cheats screen-gameplay` plays the page through its keys (the text speed byte, the three values, Save and Continue, the auto-run on a
waiting message); `cargo test -p ssd-game --release --test gameplay_real_rom -- --ignored` (with `SSD_ROM_PATH`) plays the auto-run on the
story replay and checks that the animation frames ignore the pad on the reference battle.

Difficulty presets: Easy is enemy power x1/2, EXP x2, money x2; Normal is the original (all x1, clean); Hard is enemy power x2, EXP x1, money x1/2. Enemy power and EXP are the Cheats screen's own values; the money multiplier is one more value of the cheat block (`MONEY`, a num/den pair after the marker, outside the settings bytes) that the Cheats screen has no row for. It scales the zeni of the story's events (`01:D7F9`, which adds `$0D53` to the zeni), and the Cheats screen's zeni gained scales them again: a battle itself pays no zeni (`docs/re/data/battle.md`). The preset is read off the three values, never stored apart, so changing one on the Cheats screen reads Custom (the last change wins). Easy and Hard are cheats, so the save made on them carries the CHEATS badge; only Normal is clean. The money step is kept per slot in `saves/ssd.options` as `s<slot>.difficulty.money` (the preset is written there as `difficulty.preset` for reading, Normal leaves both out); the power and the EXP travel in `ssd.cheats` as ever. `menu-test cheats money` plays the event at x1/2, x1 and x2 against Classic.

The rows are a table per category (`crates/ssd-menus/src/options/<category>.rs`); a task adds rows to its own file. Where a
value lives: Difficulty and Gameplay are saved with the game, per save slot, in `saves/ssd.options` (keyed by the slot's
checksum word like `ssd.cheats`; a Save writes the slot it chose); Video, Audio, Controls and Accessibility are per PC in
`saves/ssd.settings`. Both are text files, a first line `ssd-options 1` or `ssd-settings 1` and then `key=value` lines of
whole numbers (`s<slot>.sum=` and `s<slot>.<key>=` in the slot file): a missing key is the default, an unknown key is kept, a
damaged line is skipped, and a file that is not an options file is kept aside as `.bad`. The rule: only a windowed run
reads and writes the two files. A headless run (`--headless-frames`), a UI snapshot, a benchmark (`--bench-app`) and a
replay (`--replay`) read neither and write neither; their settings stay in memory for that run.
`menu show options` in a UI script checks the screen.

The Video page (this PC, keys `video.display`, `video.scale`, `video.vsync`, `video.look`, `video.aspect`, `video.scanlines`,
`video.crt`): Display (window or fullscreen), Scale (2x to 8x), V-Sync, Look (crisp or smooth), Aspect (16:9 shows the extra
map at the sides, 4:3 shows only the original window), the size of the picture it gives, and two filters, Scanlines (light,
strong) and CRT filter (soft, full). Every row works while the game runs. A change of display, scale or aspect asks "Apply
video changes?": Keep writes it to `ssd.settings`, Revert (B) puts the old choice back, and after 10 s without an answer it
reverts (the cursor starts on Revert). A `--hd-scale` or `--enhanced-look` flag wins for its run until that very row is
changed on the page; headless runs, snapshots and replays ignore the saved file. Scanlines, the CRT filter and the 4:3 crop
are a last pass of the app over the picture (`crates/ssd-app/src/filter.rs`), not part of `ssd-hd`: with them off the picture
is the renderer's own, byte for byte.

The Accessibility page (this PC, keys `access.text_size`, `access.contrast`, `access.cursor`) changes how all the HD menus look (Menu, Items, Cheats,
Options and the notices), never the game's own picture or Classic: Text size (Large makes every text a quarter bigger where there
is room beside it and inside its window, and never less than drawn), High contrast (white windows, black text and frame, black
selections) and Cursor (Safe is a bigger blue arrow with a bar under the row). The "Sample" window under the rows shows the choices
at once. With the three at their defaults the menus are the design exactly: `ssd_menus::Style` is applied to each finished draw list
and returns it unchanged. The windows keep their size, so a text grows only as far as its window and its neighbours allow.

The Audio page (this PC, keys `audio.volume`, `audio.output`): Volume (0 to 100 in steps of ten, default 100) and Sound (stereo or
mono). The game's sound reaches the host as one mix, so music and effects cannot be told apart without a change to the vendored
core (`docs/re/audio-voices.md`): there is one Volume, not two. Both rows act on the samples handed to the speakers only
(`crates/ssd-app/src/audio.rs`, `options/audio.rs`): at full volume in stereo the samples are unchanged bit for bit, at 0 they are
silence, mono plays the mean of both sides on both. The machine, Classic and the replays never see them.

The Controls page (this PC; keys `controls.device`, `controls.kb.<action>`, `controls.pad.<action>`): the Device row (Keyboard
or Gamepad: each device has its own map) and a key box per action, two to a line: the SNES buttons (the d-pad and the
shoulders on the left, the buttons on the right) and five shortcuts (speed, pause, save state, load state, screenshot). A on a box
waits for the next key (or, on the Gamepad page, button); Esc cancels and Backspace clears a shortcut (a SNES button keeps
its key). A key or button an action already has is refused, there is no swap: the box says "Key in use" and flashes, and
the help strip says "Key already used by <action>. Choose another key." The window's own keys (Esc, F1, 1 to 4) cannot be
given away and are refused by what they do. L and R still change the page (the device is the first row). A key is stored
as its USB usage number (SDL's scancode), a gamepad button as SDL's button number plus one, and 0 as none; a missing,
unknown or wild value is the action's default (the keys above), and a value equal to the default is not written. The
map drives the game window and the HD menus alike (the menus read the same joypad), in Classic runs too. The joypad a
replay records is the SNES buttons after the map, so a replay is the same under any map and plays without it; the map
is read only by the live window. In a UI script, `menu press <key>` (or `esc`, `backspace`, `pad <button>`) answers a box
that waits.

### Data packs (M10)

A data pack adds or replaces game data in Enhanced mode: maps, scenes, text, tables, art and chapters. The Classic game is
never changed by packs, and a pack holds no code. The pack format and its pages are in `docs/mods/README.md`.

To install a pack, put it in the `mods` folder of the data folder: a folder with a `pack.toml` in it, or a `.ssdpack` file.
In Enhanced mode, Options > Gameplay > Data packs (A opens it) lists every pack of that folder with its version, and a
box that is filled when the pack is on. A pack the checks refuse (made for another ROM, a file that does not read) is in
red with a cross, stays off, and its details say why.

On the Data packs screen, A switches a pack on or off, and on "Install a pack..." (the line after the packs) opens the system's
file picker and copies the pack chosen into the mods folder (`packs::pick_install`; the pack joins the list, off); Y held with Up or Down moves the pack in the order (the packs load in
that order); X asks to apply the set, and Apply writes it to `mods\mods.toml` and restarts the game to the title: save first,
unsaved progress is lost. A set the checks refuse is not applied, and the screen says why. B goes back to the Options.

    enabled = ["super.kaioken", "free.progression"]

`mods\mods.toml` names the enabled packs by id, in their load order; a missing file enables none. An Enhanced run that persists
(a windowed run) loads that set at its start; a set the file names that the checks refuse, or a pack it names that is not in
the folder, is not loaded: the game starts without packs, and the screen says why. The command line overrides the file:
`--mods <folder-or-.ssdpack>` (repeatable) runs the given packs and `--no-mods` none. A run that does not persist (a headless
run, a UI snapshot, a benchmark or a `--replay` run) reads no set file.

A pack that offers chapters lists them under "-- Chapters --" when the pack is on. A chapter that waits for the game to be
finished is grey and says "locked", until the game is completed or the cheat Unlock chapters is on. An open chapter asks
"Start this chapter?" with the cursor on Cancel; Start restarts the game into the chapter's own saves, under
`saves\chapters\<id>\`, and the main saves are not touched. In a chapter session the top of that list is "Main game", which
goes back to the main saves. A chapter starts from the set in use: when the list has changed and is not applied, the screen
says "Apply the packs first". The command line does the same: `ssd-recompiled --mode enhanced --chapter <id>`.

In a UI snapshot, `menu show mods` checks the screen, and `menu key x` is X (see the list above).


## Debug panel

F1 shows or hides the panel. Its tabs:

- Routines: each routine of the symbols file with its mode (O original, R Rust, V verify, switched live), its calls,
  its fallbacks to the original, and the share of the instructions that run as Rust. The share is a dash until a
  Verify run has measured the means of the routines in Rust.
- Memory: a field that takes a symbol or an address and moves the view there (symbol search), a hex view of the bytes
  around it with the changed ones marked, and freezes that hold a value. Freezes are off while a replay records or
  plays.
- Debugger: pause, breakpoints by symbol or address, step into, over and out, and stop at interrupts.
- Replays: record a replay from power-on, play one (Stop replay ends it), verify one in lockstep against the original,
  and the divergences a check saved, which Reproduce runs again.
- Below the transport and state controls: Save screenshot (the same as F12) and Save 1x frame (the PPU's own frame, 256 x 224
  or 239 lines, unscaled). The line next to them says where the file went.

Keys 1–4 select the save-state slot; F3 opens the Cheats screen over a battle (Enhanced; Back closes it and the battle goes on). The panel can be checked without a window with `--ui-snapshot` (see "UI snapshots"
below).

## Android (M9)

State: a feasibility spike with a real first launch. A debug APK builds, installs on an x86_64 emulator and runs the game,
Classic and Enhanced (with the HD pack). The first launch asks for the ROM, the mode and, for Enhanced, the HD pack, on screens in
the game's style (the system file picker is Android's). The touch controls are in (below). It has been played on phones (arm64),
a foldable included (inner and outer screens, split screen, folded and unfolded).

First launch (`crates/ssd-app/src/android_setup.rs`, before the game starts): the screens of the approved boards, drawn with the
HD menus' own drawing. `ssd_menus::setup` (pure, tested) builds each page as a draw list in a 932 x 430 space (a cream window in
an orange frame with a dark-red line, the orange main button with the cursor, quiet dark-red buttons, a progress bar) plus the
buttons' rectangles; `crates/ssd-app/src/setup_ui.rs` paints it through `hd::paint_fit` and turns a tap on a button (at least
48 logical pixels high) into a command; the d-pad or arrow keys move between the buttons and A, Enter or Space chooses, B, Esc
or Back goes back. The pages: Choose your ROM (accepted: English v1.02, CRC-32 A8CF4F2A, or Japanese Rev 1 plus the patch,
which asks for `dbz102.ips` on a page of its own), Choose a mode (Classic or Enhanced), Install the HD pack (Select
hd-pack.zip, or Skip, play Classic only; a bar with the step: copying, unpacking N%, building, putting the art in place,
starting). The system file picker is SDL's file dialog, which on Android is the Storage Access Framework: SDL opens the
`content://` address it returns as an I/O stream, and a worker thread copies the file in 1 MiB blocks to the app's folder
and gives it to the same identify, normalise and cache code as `--rom` (a copier header is stripped); the screen keeps
drawing and shows the steps. A wrong file shows the reason on the same page and the choice stays open. The mode is kept in
`files/android-mode.txt` (`--mode` in the arguments file wins). An install copies the picked `hd-pack.zip` (deleted afterwards)
and runs the desktop installer; Skip writes Classic when there is no pack yet. A launch that needs nothing opens no setup window.

Later: the Options have a Game page on Android only (`options::show_game_page`; the rows are hidden on the desktop). It shows the
current mode and the HD pack (revision and size) and has Change mode and Install the HD pack. A button ends the game and the
launch loop (`SDL_main` in `android.rs`) runs the setup again at that page (`android_setup::request`), then starts the game in
the new mode, so "applied at the next start" costs a second of waiting and no manual restart. Classic has no Options, so there
Select + Start held for two seconds does the same (the Mode page). `hd_art` forgets its pack and font between runs, so a pack
installed in between is the one used.

Layout: `crates/ssd-android` is the shared library `libmain.so` (empty on every other target); its root,
`crates/ssd-app/src/android.rs`, includes the desktop `main.rs` and adds the `SDL_main` entry that SDL's activity calls. The
Gradle project is in `android/` (`GameActivity` is SDL's activity; SDL's Java glue is copied from the SDL sources that
cargo fetched, and is not tracked). The recompiled code (`--features recomp`) is never part of the APK. The desktop build is
unchanged: the Android parts are `cfg(target_os = "android")` or separate crates outside the default members.

Build (Linux, or any host with the Android SDK and NDK, a JDK 17+, cmake, Rust with the Android targets and `cargo-ndk`):

    export ANDROID_HOME=~/Android/Sdk ANDROID_NDK_HOME=$ANDROID_HOME/ndk/25.2.9519653
    android/build-apk.sh                 # x86_64 for the emulator; add arm64-v8a for a phone; --release first for release
    # the APK: android/app/build/outputs/apk/debug/app-debug.apk

A release build (`android/build-apk.sh --release arm64-v8a`) is unsigned unless `SSD_KEYSTORE_PROPERTIES` names a
properties file, kept outside the repository with the keystore it points to:

    storeFile=/path/to/release.jks
    storePassword=...
    keyAlias=...
    keyPassword=...

Then `app-release.apk` is signed with that key and installs with `adb install` (a newer build over an older one needs
the same key). Without the variable nothing changes.

Test on a headless emulator (`emulator -avd NAME -no-window -no-audio -gpu swiftshader_indirect`): install the APK with
`adb install`, then put the files into the app's private folder (the debug build allows `run-as`):

    adb push canonical.sfc /data/local/tmp/r.sfc
    adb shell "run-as io.github.dbz_ssd_recompiled sh -c 'mkdir -p files/cache/rom && cp /data/local/tmp/r.sfc files/cache/rom/canonical.sfc'"

The launcher icon is pixel art (`assets/icon/sfera-4-stelle-54.png`, 54 x 54, a pixel for 2 dp of the adaptive icon's
108 dp): `python tools/android-icon.py` writes it as vector drawables in `android/app/src/main/res/` (the four-star ball as the
foreground, the starry blue as the background, and the ball with its stars cut out as the monochrome layer of themed icons),
so it is sharp at every density and the repository holds no picture outside `assets/`. `build-apk.sh` builds the game's crates
optimised even for a debug APK (the HD renderer at the dev profile's level 1 is too slow on a phone) and deletes the last APK
first (Gradle packages over it and keeps the room of a bigger old library).

`files/android-args.txt` (one line) holds extra launch arguments: `ENV:NAME=value` sets an environment variable, the rest is
the command line (`--mode enhanced --install-hd-pack /data/data/io.github.dbz_ssd_recompiled/files/hd-pack.zip`).
Errors and the log go to `adb logcat -s ssd`.

Graphics backend (`canvas::backends_to_try`, `video::open_gpu`): `WGPU_BACKEND` (for example `ENV:WGPU_BACKEND=gl` in the
arguments file) wins; otherwise an emulator (system properties `ro.hardware`, `ro.kernel.qemu`) gets OpenGL ES alone, because its
Vulkan is SwiftShader, which crashes the emulator; any other device tries Vulkan first and falls back to OpenGL ES when the
adapter or device cannot be created, or the Vulkan adapter is a software one.

Refresh rate and sound: an adaptive (LTPO) screen drops to about 24 Hz while nothing touches it, and every present waits for
it. `GameActivity` asks for the lowest display mode of the current resolution that is a multiple of 60 Hz and sets the
surface's frame rate to 60 (Android 11+), at start and on each focus (a fold or an unfold is another display); and at speed 1
the pacer runs the frames the clock owes, up to four a window frame (`CATCH_UP`), each with its sound, so a slow screen costs
smoothness only. The sound queue is kept near 60 ms by bending the playback speed by at most 0.5%
(`SDL_SetAudioStreamFrequencyRatio`, on every platform: on a desktop it stays near 1.0), and every 5 s the log gets the
sound's health (samples made per second, flushes that found the queue dry, the least and most queue).

Lifecycle: the activity follows the sensor (landscape and portrait), keeps the screen on and hides the system bars (immersive, sticky;
the `WindowInsetsController` on Android 11+). When the window loses
the focus (home button, another app, screen off, the notification shade) the game stops running, the wgpu surface is dropped, the
battery save is flushed and the whole machine state is written to `states/background.ssdstate` (not in a run that does not
persist, a recording or a replay); SDL then blocks the event pump until the app is back, which also pauses the audio. On
`FocusGained` the surface is made again on the window's new native surface (retried every frame until the window has one), the
pacing clock starts over and the state file is deleted, so the game continues exactly where it was. If the system kills the
process while it is in the background, the next launch loads that file once and deletes it, so nothing is lost beyond what the
game itself would lose; the file can never replace a newer battery save, since it only exists between a background and the next
start. Tested on the emulator: five home/return cycles and two screen off/on cycles with the same process alive and the intro
continuing, and a kill in the background followed by a launch that resumed.

Touch controls (`crates/ssd-app/src/touch/`): on Android (and with `--touch-ui` on a desktop, for UI snapshots) an overlay of
buttons is drawn over the game and driven by SDL's finger events; the design is the owner's approved mock-up. They feed the
same input state as the keyboard and the gamepad (`InputState::touch`), so the game, the HD menus and the shortcuts work
unchanged, and a replay (which records the SNES buttons after the map) does not depend on them. It is drawn with egui, the
overlay the app already has: anti-aliased shapes and Share Tech Mono text in dp (egui points), where the HD menus' draw
lists are a fixed 1280 x 720 design mapped onto the picture. SDL's touch-to-mouse events are turned off so the fingers are the
controls' alone.

- Landscape: the 16:9 picture is centred with the controls in the side margins and Start and Select below it. A fixed d-pad at
  the left (or, as an option, a floating stick that appears under the thumb in a zone at the left), A/B/X/Y in the SNES diamond
  (A right red, B bottom yellow, X top blue, Y left green), L and R at the top corners, a speed button (x1/x2/x4/x8 cycle),
  MENU, and MIN. Multi-touch: the d-pad and A at once; a finger slides across the d-pad directions (8-way) or the face
  buttons, and lets go when it leaves. A pressed button turns solid cream; the rest is 75% opaque (20 to 100%).
- Minimal (battles): only the d-pad, a big A and B, MENU and FULL (back to all). Switched by hand with MIN and FULL, or by itself
  in a battle (option "Switch automatically in battle", off by default; a battle is the game mode of a battle with `in_battle`
  set, `ssd_game::gameplay::in_battle`).
- Portrait (the activity is no longer locked to landscape): the picture on top, the controls on an opaque band below. The
  rows are parts of that band, so on a short screen (a split screen, a window, a Fold's halves) no group covers another or
  the picture; a test checks every group at nine screen sizes, both ways.
- Picture (Touch options): Standard (the design), Large (the controls at 85% and packed toward the edges; in portrait the
  picture takes the whole width where the band still fits) or Full screen (the picture as big as the screen holds, the
  controls floating over it at their opacity). With the touch controls an Enhanced picture fills its area at any scale
  (`layout::fit_any`); the Classic frame and the desktop keep whole scales.
- MENU pauses: Save state and Load state (the current slot; State slot picks it, 1 to 4), Options (the touch options), Edit layout, Hide controls (a chip
  at the top right brings them back), Resume, and a GAME page with Restart game and Quit, each on a second tap (the play
  since the last save is lost; on Android the launch loop starts the game again, `android_setup::Forced::Relaunch`). The
  Android back key opens it too.
- Edit layout: tap a group to select it, drag it, drag a corner handle or use +/- for the size (100 to 180%, so never under
  48 dp), opacity, Reset to default, Done. The layout is kept per device in `saves/ssd.settings` under `controls.touch.*`
  (`style`, `opacity` as a step of five percent, `haptics`, `auto_battle`, `hide_pad`, `picture` (0 Standard, 1 Large, 2 Full
  screen), `show_fps`, and `<group>.x|y|s` or
  `p.<group>.x|y|s` for the portrait layout: the centre in per mille of the screen and the size in percent).
- Options > Controls: the Device row has a third page, Touch, with Touch layout (D-pad or Stick), Opacity, Haptics, Switch
  automatically in battle, Hide with gamepad (on by default; only a pad with a stick and face buttons counts), Picture,
  Show FPS (off by default: a line at the top with the frame rate, the sound queue in milliseconds and the frames that found
  it dry, drawn even while the controls are hidden) and Reset touch layout (which keeps Picture and Show FPS).
- Haptics: a light tick on a press, on by default; the native side sends a command to `GameActivity`, which performs the
  system's keyboard-tap feedback (no permission needed).

In a UI snapshot with `--touch-ui`, `touch menu|options|edit|hidden|play|minimal|full|stick|dpad|gamepad|select <group>|press up,a|release`
lines set the state drawn (README, "UI snapshots"). Tested on the x86_64 emulator: the d-pad and the START and A buttons
played the intro past the title, MENU, Edit layout (a drag, saved in `ssd.settings`), MIN/FULL and portrait.

Next: a gamepad-aware first launch, a check of the touch controls on a phone
(with the touch controls done), performance on a phone, saves and states in app storage, and a signed release build.

## Data and files

Saves, save states and the ROM cache live in `%LOCALAPPDATA%\dbz-ssd-recompiled\`:

- `saves\` — battery saves (and `saves\chapters\<id>\`, the saves of a chapter of a data pack)
- `states\slotN.ssdstate` — save states, one file per slot (N = 1–4)
- `cache\rom\canonical.sfc` — the identified, normalised ROM
- `mods\` — the data packs, each a folder with `pack.toml` or a `.ssdpack` zip, and `mods\mods.toml`, which names the enabled
  packs (see "Data packs (M10)")

The environment variable `SSD_DATA_DIR` overrides that directory (the tests use it).

## Screenshots

F12 (or the debug panel's Save screenshot) saves the picture the window shows as a PNG, without the debug panel: the
Enhanced picture with the HD menus and overlays at the window's size in pixels, or the Classic picture, laid out as with the
panel closed. Save 1x frame saves the PPU's own frame unscaled. The files go to `<Pictures>\DBZ SSD Recompiled\`
(the system's Pictures folder), named `dbz-YYYYMMDD-HHMMSS.png` in local time (`-2`, `-3` if several are taken in one
second; a file is never overwritten). The PNG is deflate-compressed. Beside each picture the same run writes the machine's
state, named the same with `.ssdstate` (as `states\slotN.ssdstate` is), so the moment is reproducible: `xtask shots
--state-in`. Without a Pictures folder, and on Android, they go to `screenshots\` of the data folder. A short notice (2 s)
shows the path. The key is remappable on the Controls page; `SSD_SCREENSHOT_DIR` overrides
the folder (the tests use it). A screenshot is not a save: a headless run, a UI snapshot or a `--replay` run takes none.

Only a windowed run writes the saves and states folders. A run that does not persist (a headless run, a UI snapshot, a
benchmark or a `--replay` run) writes none of their files and does not create the folders: its battery save, bag and cheat
changes and its save states stay in memory for the run. It still reads the battery save, the bag and cheat files and the
states that exist, so a snapshot shows the player's saves; a replay plays its own SRAM instead. The option files are neither
read nor written by such a run (see "Options (M7)"), and neither is the pack set file `mods\mods.toml` (see "Data packs (M10)"). The ROM cache and the HD pack under `cache\` are written by any run that
is given `--rom` or installs a pack: they are caches of the ROM, not saves.

Headless test mode: `ssd-recompiled --headless-frames N --screenshot out.bmp` runs N frames without a window and
writes the last frame as a BMP.

`--touch-ui` draws the touch controls (always on in the Android app): with `--ui-snapshot` they are in the picture and the script's `touch` lines set them (see "Android (M9)").
UI snapshots: `ssd-recompiled --headless-frames N --ui-snapshot out.png [--ui-script actions.txt] [--ui-size 1440x900]`
runs N frames without a window, applies the script to the debug panel and writes the whole picture as a PNG. It writes no
file of the saves or states folders and reads no option file (see "Data and files" and "Options (M7)"). The panel
is drawn offscreen by the window's own code: no window or surface is created and no input is read, so this is how
the UI is checked without opening a window (`CONTRIBUTING.md`). `--replay` and `--hooks` work as in a headless run, and a verify run still
exits with code 5 on divergences. The script has one action per line; blank lines and lines starting with `#` are
skipped, and a line that is not an action is a usage error (exit 2) that names its line:

- `panel open` or `panel close`
- `tab routines`, `tab memory`, `tab debugger` or `tab replays` (opens the panel on it)
- `pause` (pauses; nothing when paused already)
- `step into`, `step over` or `step out` (while paused; a warning otherwise)
- `frames N`: runs N frames, even while paused; a breakpoint ends them early and pauses the game
- `select SYMBOL` (or an address, as the Memory tab's "Go to" field takes it)
- `breakpoint SYMBOL` (or an address, as the Debugger's field takes it)
- `mode ROUTINE original`, `mode ROUTINE rust` or `mode ROUTINE verify`
- `menu open` (A on the field), `menu key up|down|left|right|a|b|l|r|y|select|x` (the HD menu's keys: a confirms, b cancels, l and r change a page, y and select reset on the options, x applies the data packs),
  `menu tab all|allies|tools` (the bag's tab), `menu press <key>` (a raw key or `pad <button>` for a Controls box that waits), `menu battle-cheats` (the Cheats screen over the battle the game is in, as F3 opens it) and `menu show field|menu|items|cheats|options|mods` (checks the HD screen): with
  `--mode enhanced` only. A menu line plays its choice to its
  end before the next line; a replay still playing stops there, and the menu presses live (`--replay` with
  `--headless-frames` to the field first)

The twelve reference frames of M6 task 3 (title, flight, field, a message, the command menu, two menu screens, the
battle command screen, an attack, three of the ending) are drawn at 4x from the reference replays and written to
`.work/hd/`. A ROM-gated test (`SSD_ROM_PATH` names the ROM) compares each one with a coarse signature (the channel
means and the edge strength of 128 x 128 pixel cells, and the centre of the edges), not with its bytes: the GPU's
rounding moves the signature by less than one step and passes, while a blanked cell, a shifted or recoloured
picture or a filter fails (see the test's module documentation):
`cargo test -p ssd-app --release --test hd_snapshots -- --ignored`.

Symbols need the symbols file (see "Symbols" below). Nothing is paced: the header shows the rate the window runs at
(60.1 fps). `--ui-size` takes `WIDTHxHEIGHT`, each side from 1 to 4096 (default 1440x900). The PNG is stored
uncompressed, about 5 MB at the default size.

Hook modes: `--hooks original` (default) runs the game's own code. `--hooks rust` runs the routines of
`re/symbols.toml` in Rust. `--hooks verify` runs the original code and checks every Rust routine against it, prints
two tables of the checks at exit (at most 120 columns wide) and exits with code 5 if any differed (reports go to
`--divergence-dir`, default `.work/divergences`; the option is ignored, with a warning, in the other modes).
Exit codes: 0 ok, 2 ROM missing (or a usage error), 3 ROM rejected, 4 symbols error, 5 divergences, 6 replay
rejected (unreadable, another ROM or emulator version, an initial SRAM the cartridge does not take, or `--record`
naming the replay itself).

Symbols: `--symbols` names the file. Without it the app looks for `re/symbols.toml` of the workspace containing the
current directory, then of the workspace containing the executable (a binary built in `target/` and run from
elsewhere), and finally for `symbols.toml` next to the executable. `--hooks original` does not need the file, but reads
one given with `--symbols` all the same, so a typo does not go unnoticed. In `--hooks rust` a routine without `cycles`
in the symbols file (its maximum cost in master cycles) never runs in Rust, its original code runs instead; a warning
at start-up lists such routines. Measure the costs with `--hooks verify`, whose table shows the means and counts the
declared costs that differ (`cost_mismatch`) or are missing (`cost_missing`).

Routines whose cost is long or follows the data (a count, a loop over a table, a decompressor) have two settings, for
the developer who ports them (`crates/ssd-oracle/src/{hooks,timing}.rs` have the details):

- A cost bound function, `fn(&Ctx) -> u64` (`ssd_oracle::BoundFn`), registered next to the routine with
  `RegistryEntry::bounded(name, func, bound)` in `crates/ssd-game/src/registry.rs`. It returns the most CPU-busy master
  cycles this call can cost, from the entry state alone, through the read-only side of `Ctx` (registers, flags and the
  `peek8/16`, `peek_db8/16`, `peek_dp8/16` reads, which have no side effects and see ROM and WRAM only), and must be
  cheap. `--hooks rust` uses it in place of `cycles` for the timing-window check (the call runs in Rust only if no
  timing event can come within the window of that bound; else the original runs, `timing_fallback`), charges it when the
  routine declares no cost, and counts in `cost_over_bound` the calls whose declared cost goes above it; `Verify` counts
  `cost_over_bound` against it too. A routine with a bound needs no `cycles`; one without keeps the static maximum.
  The bound has to be sound (a call that costs more is counted, but in Rust mode its extra time was not checked for
  timing events).
- `budget_frames = N` in a `[[routine]]` of `re/symbols.toml` (1 to 600, default 2): the frames of master cycles the
  original of a `Verify` call, and the original code the routine runs with `Ctx::call_original`, may take before the call
  counts as not returned (`no_return`, and the routine is demoted to the original). `--hooks rust` is unchanged by it: a
  call longer than the timing window still falls back to the original, whatever the budget.

A routine that leaves D or DBR different from its entry (a `PLB` or `TCD` that is not undone) sets them with
`Ctx::set_d` and `Ctx::set_dbr`, as it sets A, X, Y and the flags.

What a bound can unlock has a limit by design: a call that spans a timing event (an NMI, an IRQ, HDMA taking the bus,
a GPDMA requested just before it) can never run atomically in Rust and always falls back. `Verify` counts, per routine,
the calls whose real cost fits the window at their entry (`window_fit`; `probe` prints the same number), the most an
exact bound can unlock.

Replays: `--record run.ssdreplay` records the session (every frame's joypad from power-on and the SRAM at power-on;
needs `--hooks original`). `--replay run.ssdreplay` plays one back from power-on with its own SRAM in memory (the
save file is neither read nor written, and a state saved while it plays stays in memory) and works with any `--hooks` mode; a headless run stops at its end, a windowed
one runs on with all buttons released. Both together record what is played (to another file). F9 (load state) is off
while a replay records, and while one plays until its last frame. A replay is tied to the ROM and to the emulator version it was recorded with. The recording is
written when the run ends, after an error or a panic too, but not when the console is closed or Ctrl+C kills it.

The Japanese Rev 1 ROM plus `--ips` input path is implemented but untested end to end (no Japanese ROM available).

## Developer commands

The xtask commands run from the workspace root, in release (`cargo run --release -p xtask -- <command>`), and need the
cached ROM like `publish-check` (launch the app once with `--rom`). A bad command line exits 64, for every command.

`make-replays` regenerates the reference replays in `.work/replays/` (not tracked): `battle.ssdreplay` (7 200 frames),
`wander.ssdreplay` (12 000 frames), `menus.ssdreplay` (6 878 frames: the map menu, Talk, Look, Item, Menu > Cards,
Status, Order, Text speed and Save to slot 01), `landing.ssdreplay` (6 022 frames: take-off, landing, walking on
foot, Talk, Look, flight and a random battle) and `story-1.ssdreplay` (48 036 frames: the flights and random battles
from the first island to the second, its walk, the Item menu, and the first level-up; no shop or town is reached
yet), recorded from the scripts of `ssd_game::scripts` (`menus`, `landing` and `story-1` are timeline files,
`crates/ssd-game/scripts/*.txt`; see `shots` below). It records them without hooks,
plays each file back with the sync points of `re/symbols.toml` and prints its size and sync-event count. A replay
carries the emulator core version (`ssd_oracle::CORE_VERSION`: the upstream commit and the CRC32 of the vendored `.rs`
files and crate manifests under `third_party/jgenesis`, plus the oracle's own revision; documentation such as
`PATCHES.md` does not count), so after any change to those sources the old replays are rejected: regenerate them.

    cargo run --release -p xtask -- rebase-replays <dir|replay>... [--check]

is the other way, for a change of the vendored core that does not change what it emulates (an output-only patch such
as the render log of M6): it rewrites each replay as made with this build's core, keeping the ROM, WRAM seed, initial
SRAM and inputs (only the core version field and the checksum change), prints `old -> new` per file, and refuses a
file that does not read back intact. It does not prove the new core plays the replays as the old one did (a
`verify-replays` run does); use `make-replays` when it may not. The `secret-*` and `shop` replays are not recorded by
`make-replays`, so this is the way to bring them to a new core. `--check` writes nothing and exits 1 when a replay is
for another core.

    cargo run --release -p xtask -- verify-replays [--hooks rust|original] [dir]

plays every replay of the directory in lockstep (a pure machine next to one with the routines of `re/symbols.toml` in
Rust) and exits 0 when all are green, 1 on a divergence or an error (a missing directory, an unreadable or stale
replay), 2 when a replay had no sync event or the directory has none (unverified). With `--hooks rust` a routine over
its declared maximum cost (or without one) counts as a divergence, and a routine the replay called but never ran in Rust
leaves it unverified; so does, in both modes, a sync point reached inside a dispatch where it would have been an event
(its `sync_inside_dispatch`, listed with the routines' counters). It takes a few minutes.

    cargo run --release -p xtask -- standalone-diff [--verbose] [--cheats-identity] [--mods-identity] <replay>...
    cargo run --release -p xtask --features recomp -- standalone-diff <replay>...

plays each replay on two free-running machines, the original and the standalone game (`--hooks rust`: every routine of
`re/symbols.toml` in Rust, and with the `recomp` feature the recompiled ones too), each fed only the joypad of the replay,
as the shipped game runs. Lockstep (`verify-replays`) pins the Rust side to the original's timeline at every sync event;
this does not, so it is the check that the standalone game *is* the original. The Rust side runs a frame at a time as the
app does (`Machine::run_frame_with`); after each frame the original is brought to the same master cycle and the two are
compared there: frames rendered, NMIs taken, registers, work RAM (the dead stack left out, as lockstep does), VRAM, CGRAM,
OAM and SRAM. It prints the first stop that differs, with what differs and the routines whose call rendered a frame, and
exits 1; otherwise one line per replay and exit 0. While a Rust call's charge is only partly held
(`Machine::charge_pending`: a decompression with the screen off spans several frames, and the routine has written all its
results at once) the registers and work RAM are not compared, everything else is; the line counts those stops. A replay
recorded for another emulator core is refused, as everywhere (`make-replays`). It takes about 20 seconds per short replay.

With `--mods-identity` the standalone game (B) runs on the zero-padded 2 MiB image of the empty data-pack set, with the
redirectors of the packs installed (`ssd_game::mods`), so the same replays must come out identical: the check that a
run on a pack image without packs is the original's. `--cheats-identity` and `--mods-identity` combine. `verify-replays`
takes the same two flags, with the same meaning for its machine B.

    cargo run --release -p xtask -- mod-build <dir> <out.ssdpack> [--local]
    cargo run --release -p xtask -- mod-check <pack>... [--local]

check data packs, each a folder with `pack.toml` (and the files its `[[data]]` entries name) or a `.ssdpack` zip of the
same layout. `mod-check` runs the checks of the set against the cached ROM (the ROM's CRC32 and size, the ids and the
versions, the file sizes, the slots and the dependencies, the data's fit in the image's banks and directory, and that no
file holds 16 bytes of the ROM) and prints each refusal, exit 1; else the placement of each slot, the image's size and
CRC32 and the set id. `mod-build` runs the same checks on one pack and writes it as a `.ssdpack` zip. `--local` allows
the runs of the ROM's bytes, for a pack that is not tracked.

    cargo run --release -p xtask -- art-derive <hd.png> <face|tiles> <out dir> [--palette <file>]

the SNES art of one HD picture (`ssd_mod::derive`, the code the packs use for their faces): a face (128 x 128 pixels)
or a map sheet (multiples of 32 pixels). It writes `palette.toml`, `tiles.bin`, `tiles.lz` and `map.toml` into the
folder, which is under `.work/` when it is in the repository. The HD art of a pack is in `docs/mods/art.md`.

    cargo run --release -p xtask -- kaioken-art <pack dir>

copies the 27 pose variants of the Super Saiyan Kaioken (`battle-goku-ssj-000` to `026`) from `.work/m10/kk-art/variants/` into
`<pack dir>/art/hd/variants/` (for `examples/ssj-kaioken`). They are made locally from the installed HD art, so the pack's `art/`
folder is never tracked (`.gitignore`); the pack loads without them. See `docs/mods/art.md`, "Pose variants".

    cargo run --release -p xtask -- scene-check <file.scene>...
    cargo run --release -p xtask -- scene-dump <scene id hex|all> [--out <dir>]

the scene language of the packs (`ssd_mod::scene`; the commands are in `docs/re/data/scenes.md`). `scene-check` assembles
each scene file and prints its size and the offsets of its label words, or its error with the line and column (exit 1).
`scene-dump` decompiles the ROM's story scripts into the language: the 82 scenes, one script per distinct start, to stdout
or to a folder under `.work/` (or outside the repository); the output is derived from the game and is never tracked. Each
script is the commands its walk reaches. The round trip of the 82 scenes (each script assembles back to the ROM's bytes
over those commands) is the ignored test `every_scene_round_trips_through_the_language`, run with `SSD_ROM_PATH` set and
`cargo test -p xtask -- --ignored`.

The app runs on packs in Enhanced mode: `ssd-recompiled --mode enhanced --mods <folder-or-.ssdpack>` (the option is
repeated for each pack; without it a windowed run takes the set of `mods\mods.toml`, and `--no-mods` runs without packs).
A set the checks refuse on the command line prints why and exits 8. A replay recorded
with packs plays only with the same packs, and one recorded without them only without (exit 6). The redirectors act in
the window and in UI snapshots; a headless run (`--headless-frames`) does not install them.

    cargo run --release -p xtask -- coverage [--exclude name[,name...]] [replay]

(default replay `.work/replays/battle.ssdreplay`) prints the share of the instructions the replay executes that now runs
as Rust: it plays the replay twice, with every routine in `Verify` (which measures the instructions of the original per
call) and then in `Rust` (the machine counts the instructions left original), and lists per routine the calls, Rust
runs, timing fallbacks, mean instructions and estimated Rust instructions, then the totals (`Σ rust_runs ×
mean_instructions`, less what the runs had run through `Ctx::call_original`, against the opcode fetches; percent = rust
/ (rust + original)). A routine whose length varies with where in the frame it is called declares its instructions per call
(`Ctx::set_instructions`, checked in `Verify`) and is counted exactly instead of at its mean. The percent counts instructions, not game logic: routines called from the idle wait (the two RNGs,
about 250 calls a frame on the battle replay) dominate it, and it grows with the idle time per frame. For a conservative
second figure name them, e.g. `coverage --exclude rng_next_a,rng_next_b`: it prints `(rust - excluded) / (rust -
excluded + original)` as well. It exits 0 with a result, 1 on an error, and takes about a minute.

    cargo run --release -p xtask -- repro <path.ssdstate>

runs a divergence report again (`--hooks verify` saves them in `.work/divergences`: a `.ssdstate` at the entry of the
diverging call and the `.txt` next to it, whose first line `ssd-repro: key=$XXXXXX routine=name` names the routine). It
loads the state on the cached ROM, installs that routine of `re/symbols.toml` alone in `Verify` (whatever its mode
there, every call checked) with the Rust function of the current build, requires the loaded state to be at the entry of
that call (CPU about to fetch the routine's first opcode), runs the call (neutral joypad) and prints the comparison as
the report does. It exits 0 when the call now matches the original (with a warning on stderr if the cost the Rust
function declares is wrong or missing), 1 when it still diverges (or on an error: no ROM cache, an unreadable file, the
symbols), 3 when it cannot be reproduced (the state is for another ROM or emulator core, the report has no header, the
routine is gone or moved, the state is not at the entry of its call, or the call is not compared: entry contract,
interrupt, no return; the reason goes to stderr). A report made before the header existed has to be made again.

    cargo run --release -p xtask -- bench

plays the battle script (`ssd_game::scripts::battle_input`) on the cached ROM unhooked to frame 4600, in the battle,
saves the state, then times 600 frames from it, headless, three times for each of two modes (no hooks; every routine of
`re/symbols.toml` in `Rust`; the order alternates). It prints the median run of each mode with the frames per second of
every run, and for `Rust` the hooked calls per second, the calls that ran in Rust and those that fell back to the
original. It asserts nothing; it takes about 30 seconds (most of it playing to frame 4600 and the six timed runs).

    cargo run --release -p xtask -- hd-parity [--frames N] [--jobs N] [replay|dir...]

checks the HD renderer (`crates/ssd-hd`, M6): it plays every replay (default `.work/replays`, `--jobs` at a time, 4 by
default) with the PPU's render log on (`Machine::set_line_log`: per scanline the registers the PPU read, the CGRAM
writes of the frame, VRAM and OAM at the end of the display; an output-only patch of the vendored core, recorded in
`third_party/jgenesis/PATCHES.md`), re-renders each frame at 1x with no art and compares it with the PPU's frame buffer
pixel for pixel; then it plays the replay again with the log off, and the final machine state and the chain of frame
checksums must be the same (the renderer only reads). One row per replay (frames checked, frames and pixels that differ,
frames the renderer refused, mean render time, state and frame checksums), the first differing pixel of each replay
(frame, x, y, both colors) and the PPU features the frames used. Exit 0 when every frame is equal, none was refused and
the machine is unchanged, 1 otherwise. It takes about half an hour for the 28 reference replays. With `SSD_HD_EXPLAIN=<frame>`
the first frame of that number that differs is described on stderr (partial renders, differing rows and columns).
The Enhanced picture at 1x (`ssd_hd::Renderer::render_hd`, scale 1) is compared with the PPU's frame in the same way
(the `enhanced 1x` column; it must be the same frame).

    cargo run --release -p xtask -- bench-hd [--frames N] [--pack DIR] [replay|dir...]

times the drawing of the Enhanced picture: each replay (default `battle` and `wander` of `.work/replays`) is played with
the render log on, and every frame is drawn at 1x, at 2x and at 4x (crisp) and at 4x (smooth); only the drawing is
timed, not the emulation. The last two configurations are the 16:9 view at 4x with the look alone and with the pack's art
(`--pack DIR` names the pack, by default the converter's under the cache directory). It prints the mean, the 99th
percentile and the worst frame per replay and configuration, and over the replays together. The 16:9 configurations share
their pictures as the window does (a frame whose inputs are those of the picture before is not drawn again): the `reused`
column counts those frames. `--check` also draws every frame of the 16:9 configurations afresh, with each table transfer
applied to each colour as it is drawn, and fails when a shared picture differs from its fresh one. The default look at 4x
is the budget of M6: a mean of at most 4 ms a frame, and the same for the 16:9 view with art on the target machine (99th
percentile at most 8 ms).

    cargo run --release -p xtask -- hd-skip-check [--frames N] [--pack DIR] [replay|dir...]

checks the fast-forward's shortcuts: at a speed above 1 the app runs the frames of a batch before the last without drawing
the PPU's pixels, without the render log and without resampling their sound, follows the 16:9 view with the last two only
(`ssd_hd::world::WideRun::skip`), and draws the picture of the last (`redraw_hd`, in the app in the background and shown a
window frame late, and skipped when the picture before is still being drawn: `redraw_hd_pipelined`). Each replay (default `battle` and `wander`) is played with the render log on and
drawn frame by frame (the reference); for each batch of 2, 4, 7, 8 and 16 frames a second machine plays it the app's way, and
at the end of every batch its Enhanced picture, its PPU frame, its work RAM, CPU registers and master cycles must be the
reference's. Exit 0 when everything is equal.

    cargo run --release -p xtask -- bench-app [--frames N] [--speeds A,B] [--scale N] [--hooks MODE] [--panel-closed] [replay...]

times the window's frame loop without a window (the speeds of the panel, 1 2 4 8 by default; `battle`, `wander` and
`shop` of `.work/replays` by default; `--scale` is the Enhanced picture's, 4 by default, always in the 16:9 view with the
pack's art, the routines in Rust as the shipped game runs them: `--hooks original` for the game's own code). It runs the
`ssd-recompiled` built next to it (`cargo build --release -p ssd-app -p xtask [--features recomp]` first), as
`ssd-recompiled --mode enhanced --hooks rust --bench-app <replay>... [--bench-speeds 1,4,8] [--bench-frames N]
[--bench-panel-closed]`. Each frame of that loop is the window's (the emulated frames of the speed, the
Enhanced picture of the one shown, the debug panel open on its first tab, drawn offscreen and waited for on the GPU as a
swap chain with one frame of latency would) but with no clock: it runs flat out and prints, per replay and speed, the
emulated frames per second, whether that holds 60 times the speed, and where a window frame's milliseconds went
(`emulate`: the machine and the HD menu's driver; `hd`: the 16:9 picture; `status`: the header's figures; `views`: the
pictures chosen; `ui`: the interface run and the game texture uploaded; `paint`: the pass painted and submitted;
`wait`: for the GPU) and the cores busy (the process's CPU time over the wall time).

    cargo run --release -p xtask -- hd-wide check [--frames N] [--jobs N] [--widths A,B] [replay|dir...]
    cargo run --release -p xtask -- hd-wide shots [--widths A,B] [label=replay:frame...]

draws every frame of the replays in the wide view (`ssd-hd::world`: 372 pixels is the 16:9 view, 488 the hidden ultra-wide
one, 256 the Classic frame) and checks it. At 256 the frame is the Classic frame. At the wider widths the original's
256 pixels are the Classic frame, except under the objects the original drops (the field's). Beside a picture (battle,
title, cutscenes, interiors) every pixel is black; beside the window the frame is the same at every wide width (the map,
the flight's plane, the seams of the chunks); the field draws no tile from outside its map (a scripted camera can show the game's own words beyond a small map's
edge inside the 256-pixel window: those stay the original's, the margins draw nothing for them, and the lookup
check counts only the tiles inside the map). It prints the frames of each
scene class (field, flight, bordered) and exits 1 when a check fails. `shots` writes the default frame of each scene class
(or the frames named) as PNGs under `.work/hd-wide/`, with a line per frame: its class, its window and its outside checks.

    cargo run --release -p xtask -- gfx-log [replay|dir...]

plays the replays (default `.work/replays`) with the graphics decoders breakpointed and prints one row per LZ stream they
load: its address, its kind (`LZ`, decoded into `$7E:9000`, or `LZ-staging`, decoded into the `$7F` staging buffer), the
consumed and decoded sizes, how often and in how many replays it was loaded, the first load, the scenes (`game_mode` and
map) and where its bytes went (the VRAM, CGRAM or OAM range of the DMA that sent them on). The decoders that are only
counted (`00:C707`, `00:C782`, `04:83B2`, `00:FDB5`) are listed at the end. It is the research catalogue behind the runs of
`hd-convert`; it takes about twenty minutes for the 28 reference replays.

    cargo run --release -p xtask -- hd-convert

builds the HD pack of the cached ROM under `cache\hd\<ROM CRC-32>\<converter>\` in the data directory (`SSD_DATA_DIR`
relocates it; nothing is written into the repository): `hdpack.toml`, `index.bin` (the content keys of every tile, a
64-bit FNV-1a of its bytes as they sit in VRAM, per bit depth), `base\<class>\*.png` (each tile enlarged twice by Scale2x
on its palette indexes, which keeps the indexed art deterministic) and `sheets\<class>\*.png` (the tiles on a labelled
grid, for art production). The streams are read from the ROM alone: the runs of `RUNS` in `xtask/src/hd_convert.rs`, the
bit depths of `bits_of` and the English UI overlay `11:8036`, which the game writes over the decoded `08:8000`. The same
ROM gives the same bytes. It takes a few seconds.

    cargo run --release -p xtask -- hd-coverage [--jobs N] [--assets] [replay|dir...]

plays the replays (default `.work/replays`, `--jobs` at a time, 4 by default) with the render log on, renders each frame
bound to the pack's index (a character binds when its key is unique in the pack, or when it sits in a run that matches
one asset in order, or when every place of its key is in one asset) and reports per replay the visible non-transparent
BG and OBJ pixels outside Mode 7 lines, the share of them bound to an asset, and the unbound characters by bit depth,
object or BG and VRAM address. `--assets` also lists the assets the replays bind and their pixels. It needs the pack
of `hd-convert`.

    cargo run --release -p xtask -- hd-pack [--check]

checks the approved art in `assets/hd/` against the publish rules (images only under `assets/`, no run of ROM bytes, no
16-digit hexadecimal token) and against the pack's index: `assets/hd/hdart.toml` lists the overrides as `[[art]]` entries
with a logical `id`, a `file` under `assets/hd/` and a `mode` (`indexed` or `rgba`). Without `--check` the art is copied
into the pack and the index is rewritten. With no `hdart.toml` nothing is checked and it exits 0.

    cargo run --release -p xtask -- hd-maps dump [--out DIR] [--maps N,..] [--frames N] [--jobs N] [--base SAVE]
    cargo run --release -p xtask -- hd-maps shots [--out DIR] [--pack DIR] [--scale N] [--base SAVE] label=map:x,y...
    cargo run --release -p xtask -- hd-maps scan [--every N] [replay|dir...]

the field maps as pictures, for making and checking the HD maps' art. `dump` boots each map of the game from a crafted
save (as `hd-tour` does), waits for the party to be free and writes, under `.work/maps/dump/map-NN/`, the map's BG2 layer at
1x (`layer.png`: every 8 x 8 cell of the map through the game's own lookup, in the tile set and the colours the map has),
the cells' tile map words (`words.bin`), the palette (`cgram.bin`) and `meta.json`: the pointers of the map's data, a hash of
its content, the tile numbers and palette colours that change over `--frames` frames (water shimmers by palette; no tile
of any map animates), and its map objects. Maps with the same content share art (21 distinct layers in the 48 maps). `shots`
draws places of maps (`label=map:x,y`, the camera's top left; the party starts there, so the game streams the map in as it
does when it is played) as the 16:9 Enhanced picture at `--scale` (4 by default), with the art of `--pack` and without it:
`<label>-art.png` and `<label>-plain.png` under `.work/maps/shots/`. `scan` lists the layouts that replays show per map id.
Everything they write is derived from the game and never tracked.

    cargo run --release -p xtask -- hd-import <art dir> [--sources <folder>] [--masks <folder>] [--pack <pack folder>]

puts the approved art into the pack (after `hd-convert`, which clears it; nothing is tracked). `<art dir>` holds
`class1\index.json` and its PNGs (portraits and faces, 32 x 32 pictures redrawn at 4x: each entry names its stream and
half, `rom 16:C60A half 0`, so its art covers the sixteen characters of that half, which the tile binding finds on screen,
in the palette it was made for: `palette 09:xxxx` of the entry, or the source pictures of `--sources`) and
`class2\index.json` and its PNGs (battle poses and effects at 4x: each entry has the key of its pose, the SHA-1 of the
palette-index array of a fighter's sprites, or of an effect's colours, see `ssd_hd::art`) and
`cards\index.json` and its PNGs (the battle cards: one 64 x 64 picture, 8x, per tile of the UI set that a card is made of,
in a `base` layer of fixed colours and a `roles` layer whose channels are the weights of the tile palette's colours 3, 2
and 1; an entry may name the tile's `set` (`title`: the code is of the title set, not the UI set) and `carve` (the art is drawn over
the tile's own colour 1); `tips` are the tooltip pictures, by the code of the kanji's tile; see `ssd_hd::cards` and `docs/re/data/cards.md`,
copied to `art\cards` of the pack and listed in `art\cards\cards.toml`). The portraits' and poses' PNGs are copied to
`art\picture` and `art\object` of the pack and listed in `art.bin`; `luma` marks greyscale shapes that take their colours
from the game's pixels. `--masks` is a folder of the poses' silhouettes (`<source key>.png`, white where the pose is
opaque): with them a fighter that is only part of a pose (a fade-in's stripes, a sprite cut by the screen's edge) is found
in the pose that holds it and drawn from that pose's art, where the original showed it (in the palette group of its colours, or, for a pose drawn in other colours such as a fade whitened by the screen's colour math, in the group that has most of its colours right, the smallest silhouette first). `--pack` names the pack folder
(default: the converter's under the cache directory). An optional `class3\index.json` (the field's sprites, same format as class 2, ids `field-*`) is imported the same way; an index may carry a `revision` (the art store's polish pass). An optional `maps\index.json` (the field maps' art: per layer, found by
the size and the data pointers of its map, a `source` picture of the layer at 1x and its art at 4x in tiles of 256 x 256 game pixels named by a `tiles`
pattern with `{x}` and `{y}`; see `ssd_hd::mapart`) is linked into `art\maps` of the pack (a hard link where the pack is on the art store's volume) and
listed in `art\maps\maps.toml`; every size is checked.
An optional `mode7\index.json` (the Mode 7 planes' art: per plane an `id`, the content `key` of its VRAM, as `hd-art m7scan` and `m7dump` print it, the `palette` it
was made in (256 colours `RRGGBB`) and its art at 4x in tiles of 256 x 256 game pixels, RGBA, named by a `tiles` pattern with `{x}` and `{y}`; see `ssd_hd::mode7art`)
is linked into `art\mode7` of the pack and listed in `art\mode7\mode7.toml`; every tile is checked for its size.
Every folder of the art store with an `index.json` is read (`classN` are sprites; a folder no reader knows is an error, so a
branch that predates one never drops it from the pack). `bg\index.json` lists the background `layers`: `id`, the content `key`
(`ssd_hd::bgart::layer_key`), the registers that show the layer (`bg`, `mode`, `map_base`, `char_base`, `size`), the tile
`rows` its art covers, `fill` (the art includes the backdrop, so it covers every pixel; otherwise only where its alpha says),
`file` (RGBA, 4x: a game pixel is 4 art pixels) and `source` (RGBA, 1x: the colours of the layer's pixels the art was made
from, which the recolouring needs), copied to `art\bg` of the pack and listed in `art\bg\bg.toml`. Several entries may share a
`key`: an entry that is the art of one palette of the content has `palette_key` (16 hexadecimal digits,
`ssd_hd::bgart::palette_key`) and its own `palette` (the CGRAM it was made in); the entry with no `palette_key` is the art for the
other palettes.

    cargo run --release -p xtask -- hd-pack-public <art store> <out.zip> [--masks <dir>]... [--sources <dir>] [--maps-quality N]
        [--revision TEXT] [--jobs N] [--without-masks] [--no-verify]

builds the shared HD art pack, the `hd-pack.zip` that players download (`ssd_hdpack::public` has the format). It imports
the art store into a scratch pack with the cached ROM (`hd-convert` and `hd-import`, with the silhouettes of `--masks`,
which may be given more than once, and the portraits' source pictures of `--sources`), then writes the pack from the
typed indexes of that import, field by field: the art as WebP (lossless for the sprites, portraits, cards, backgrounds and
Mode 7 planes, each file decoded again and compared with its source; lossy for the maps' tiles, at `--maps-quality`, 92 by
default, and measured), one `index.json` per kind with only the fields the import reads, `masks.bin` (the outline of each
pose, a bit per pixel and no colour) and `hd-pack.toml`. Descriptions, notes, how a picture was made, the source pictures
of backgrounds and maps, the rollback zips and every original pixel stay out; a palette of at most sixteen colours, the 256
colours of a Mode 7 plane, content hashes, sizes and `BB:AAAA` pointers are the only data of the game that an entry carries.
It prints the size of each kind, then installs the zip into a scratch data directory (as the game does on the first launch)
and compares the pack that comes out with the import's, index by index: any difference is an error. `--without-masks` leaves
the outlines out. The game and a pack without source pictures: a background or map layer shown in a palette other than the
one its art was made for is drawn with the art's own colours.

The game installs the pack on its own: with `hd-pack.zip` (or a folder `hd-pack` holding `hd-pack.toml`) next to the
executable, the first `--mode enhanced` or `--hd-scale` launch builds the HD pack of the cached ROM from it
(`ssd_hdpack::install`: the converter, then the import, in a folder beside the pack that is moved into place when it is
whole; a pack made from the same art is left alone), and says what it is doing on the console. `--install-hd-pack <zip or
folder>` installs a pack from anywhere and fails the launch if it cannot (exit 7). Classic never installs anything.

    cargo run --release -p xtask -- hd-pack-to-store <pack.zip|pack dir> <art store> [--masks <dir>]

is the reverse of `hd-pack-public`: it brings an art store up to what a shared pack holds, to rebuild a lost store from
a release. Every art file of the pack is written as a PNG into the store's folder (the field's poses into `class3`) unless
the store has the same pixels; a map tile the store already has keeps its lossless PNG. The pack's entries are merged into
the store's `index.json` by `id`: the fields the pack defines replace the store's and the store's own fields (descriptions,
how the art was made) stay. With `--masks`, the outlines of `masks.bin` are written as PNGs for the keys the folder lacks.
Run `hd-convert` and `hd-import` afterwards.

    cargo run --release -p xtask -- hd-art refpal [--pack DIR] [--maps-dump DIR] [--out FILE] [replay|dir...]
    cargo run --release -p xtask -- hd-art diff --pack A --pack-b B --pack-c C [--every N] [replay|dir...]

`refpal` finds the palettes the art of the backgrounds, pictures and maps was made in, so that a pack with no source
picture of the game recolours that art as one with the source pictures does (a flash, a fade, a cave shown in a second
colour). For each background layer of the pack (`--pack`, which has the source pictures) it plays the replays until a
frame shows the layer's content (its key); the CGRAM index of every pixel of the layer is then known, and the source
picture says the colour of that pixel when the art was made, so each index takes the colour its pixels agree on; the report
says how many votes agreed and how many CGRAM entries no pixel used (those stay unknown and the art keeps its own colour
there). A map layer's palette is the `cgram.bin` of the `hd-maps dump` of a map whose `layer.png` is the layer's source.
The result is `ref-palettes.json` (`{"bg": {id: text}, "maps": {id: text}}`, each text 256 colours of four hexadecimal
digits), which `hd-import` reads from the art folder into the `palette` field of the layers' indexes
(`ssd_hd::bgart::Entry`) and `hd-pack-public` carries in the pack. The renderer recolours from the source picture when the
pack has one and from the palette when not. `diff` is the proof: it draws every Nth frame of the replays in the 16:9 view
with three packs (A the reference with source pictures, B the pack to check, C a control with no source and no palette)
and reports the frames where C differs from A (where the recolouring matters) and where B differs from A (which must be
none).

    cargo run --release -p xtask -- export-public <commit> <out-dir> [--allow-no-rom]

assembles the tree of the public repository from a commit (nothing but its tracked files), without what `tools/public/exclude.txt`
lists, with the public README, licence, notice, ignore file and Cargo config of `tools/public/` in place and this file as
`docs/DEVELOPER.md`, and scans it: the `publish-check` rules (file types, places, 16-byte runs of the ROM, also as byte
literals), `secret` rules (keys, tokens, email addresses, IP addresses, hosts, places on a person's disk) and `process` rules
(words of how the project was made) from `tools/public/scan-rules.txt`, with the findings that were looked at and are fine in
`tools/public/scan-allow.txt`. It exits 1 on any finding. Pictures are allowed in `assets/` and `docs/media/`. The wording of
the sources is fixed in the sources, not in the export.

    cargo run --release -p xtask -- hd-art report [--scale N] [--frames N] [--jobs N] [--pack DIR] [replay|dir...]
    cargo run --release -p xtask -- hd-art scan [--every N] [--pack DIR] <replay>
    cargo run --release -p xtask -- hd-art colours [--scale N] [--frames N] [--jobs N] [--pack DIR] [replay|dir...]
    cargo run --release -p xtask -- hd-art shots [--scale N] [--out DIR] [--pack DIR] [--no-font] label=replay:frame...
    cargo run --release -p xtask -- hd-art bgscan [--out DIR] [replay|dir...]
    cargo run --release -p xtask -- hd-art bgmaps [--frames N] [--jobs N] [--maps N,..] [--out DIR]
    cargo run --release -p xtask -- hd-art mapshots [--scale N] [--out DIR] [--pack DIR] label=MAP:ZONE[:PARTY]:T...
    cargo run --release -p xtask -- hd-art bgdump <replay> <frame>[,<frame>...] <out dir>
    cargo run --release -p xtask -- hd-art m7scan [replay|dir...]
    cargo run --release -p xtask -- hd-art m7dump <replay> <frame> <out dir>
    cargo run --release -p xtask -- hd-art picscan <out dir> [replay|dir...]
    cargo run --release -p xtask -- hd-art items [--out DIR] [--pack DIR]
    cargo run --release -p xtask -- hd-art margins [--maps N,..] [--scale N] [--jobs N] [--list] [--base SAVE] [--pack DIR]
    cargo run --release -p xtask -- hd-art margins [--scale N] [--out DIR] [--base SAVE] label=MAP:X,Y[:DX]...

`report` plays the replays with the art on and draws every frame in the 16:9 view as the Enhanced picture (`--scale`,
1 by default), then says how much of what the art covers is drawn from it: the visible pixels of fighters' and effects'
sprites (drawn from art, of a pose the index lacks, of a partial or cut pose), the pixels drawn from portraits, the pixels of
battle backgrounds (the layers of `ssd_hd::bgart::BATTLE_LAYERS` and their backdrop where nothing is in front) drawn from the
art of a layer, and the mean time of a frame. For the frames drawn as the field it also compares every pixel of the object
layer inside the original window with the ones in the margins of the view: the share drawn from art must match (it exits 1
when the margins fall more than 3 points below the window, once each has 400 pixels); it also exits 1 when a replay shows the
field's map, in any main loop state, for more than 120 frames in a row without drawing the pack's map art for it. `shots` writes the frames named (`label=replay:frame`) at the 16:9 view, 4x by default, three
times: with the art (`<label>-art.png`), without (`-plain.png`) and the two side by side (`-both.png`), in `.work/m6-art/`.
`colours` measures, for each art file, the mean colour the art draws inside the pose's mask against the mean colour of
the original's pixels it replaces, and lists the files that differ most, for fighters and effects, drawn from colour art
and from luma art (a luma file is the original's colour with the art's shading, so its mean should stay close).
`m7scan` plays the replays and lists the Mode 7 planes they show by content: the key (`ssd_hd::mode7art::plane_key`), the frames, the game modes, the palettes and
the ranges of the matrix, with a frame to take the plane from; `m7dump` writes the plane of a frame as it is at 1x (`plane-src.png`, 1024 x 1024, the colour
index 0 as the backdrop; `plane-idx.png`, the indexes; `palette.txt`; `key.txt`; `frame.png`) and prints the matrix of the lines: the research behind `mode7\`.
`bgscan` plays the replays and lists, for each class of battle background layer, its distinct contents with their key, frames,
palettes, scroll ranges and a frame to take the layer from (the research behind `bg\`), and the palettes of the layer's own
colours that were shown for at least 30 frames in a row (the persistent ones, not a fade or a flash: each is a palette variant);
with `--out` it writes the whole image of the layer in each persistent palette (`<class>-<key>-<palette key>-src.png`, `-fill.png`
and `-palette.txt`, the CGRAM the layer reads, for the `palette` field). `bgmaps` does the same for the backgrounds that no
replay reaches: from a crafted save on each map (or the maps of `--maps`) it forces a battle in each of the encounter zones 1 to 3
and plays `--frames` frames of it (900 by default). `mapshots` draws frame `T` of such a battle with the art and without (the
files of `shots`), with the party `PARTY` (`goku,piccolo`) instead of the map's own; zone 0 is the map's field, no battle.
`bgdump` prints each layer's content key (`key`, as `picscan` gives it), writes the frames named and
the whole image of each of their BG layers (`-bgN-src.png`, transparent where the layer is, and `-bgN-fill.png` over the
backdrop) into the folder, with the layers' scroll tables and the objects on screen. `picscan` writes, per replay, one JSON
line per frame (`<out dir>/<replay>.jsonl`): the game mode and, for each BG layer shown, its registers, content key over
the whole map (`key`) and over its first 22 rows (`key22`, the command screen's), scroll range and mosaic, with the
frame's palette hash, brightness and colour math: the research behind the art of the pictures.
`margins` boots each map from a crafted save (as `hd-tour --maps` does), moves the camera over the places of its grid and
draws each as the 16:9 Enhanced picture with the art. Without shots it counts every pixel of the object layer inside the
original window and in the margins, and the share of each drawn from art, per map, and exits 1 when the margins are covered
less than the window (`--list` names the places that have sprites in the margins). With shots (`label=MAP:X,Y` puts the
camera at `X,Y`; `:DX` makes a sequence of three frames, the camera `DX` pixels further each) it writes `<label>.png`
(`<label>-1.png` to `-3.png` for a sequence), 4x by default, to `--out` (`.work/margins/`), with the field groups of the
frame and where their art came from.
`items` audits the 40 objects' pictures (the item ids): for each it writes `orig-NN.png` (the original's pixels of the
object's own half of its set, in its palette) and `menu-NN.png` (what the HD Items menu draws) into `--out` (default
`.work/items/menu`) and prints the half and palette the object uses and the art found for exactly those characters
(its palette distance, whether it is recoloured) or `original` when the pack has none.

    cargo run --release -p xtask -- hd-tour [--class a,b] [--maps [N,..]] [--groups [N,..]] [--replays [name,..]] [--menus] [--title]
        [--only part,..] [--out DIR] [--fresh] [--jobs N] [--stride N] [--frames N] [--seed N] [--config FILE] [--base SAVE]
        [--no-font] [--dump DIR[:EVERY]] [--gaps DIR] [--audit DIR] [--formations DIR] [--list] [--report]

the automatic coverage tour of the HD remaster: it finds the art that is still missing by a report, not by playing. It
visits what the game can show, draws every frame as the 16:9 Enhanced picture with the pack's art index and says, per
class of asset, how much of the visible pixels is drawn from HD art and how much is the pixel-art filter's:
portraits and faces, battle fighters, battle effects, field sprites, map tiles, battle backgrounds, big pictures
(cutscenes, the title, the endings), fonts and text, cards and UI. What it visits (a part each, 287 in all):

- **maps**: each of the 48 maps, from a crafted save (a party of the story's characters on the map; `--base` is a battery
  save with slot 1 used, by default `.work/demo/a-goku-level-10.sav` of this or the main working copy), the camera moved
  over a grid of places whose 16:9 views overlap and cover the whole map and its margins (the camera and the leader are
  written into work RAM at each place, a test-only host action, and the wide renderer is told to trust the map over BG2,
  whose columns the game streams in only as the camera walks); a scene that the map starts is seen through or left;
- **groups**: every battle group of the table and every party fighter, in the way of the `battle-gallery` of the
  `feat/battle-gallery` branch (`tour_battle.rs`: a battle forced from the crafted save by holding the zone's encounter
  level at the roll of `02:D1F6` and writing the group at the set-up `01:F692`, driven by a seeded pad script that plays
  every command, lets a member fall, and brings the enemies down to end it; a party much stronger than the group makes
  the enemies run away before the first command, and the battle is then played again with the party at level 10);
  `--frames` is the length of a battle's play;
- **replays**: the recorded replays of `.work/replays` (the reference ones, the `secret-*` ones of
  `tools/saves/secrets.sh` with the story scenes and the endings, and the title and the intro in `battle`; the replays
  that begin with `battle`'s input do not count that prefix twice);
- **menus**: the map menus from a crafted save with objects (`tour_menus.txt`: Talk, Look, Item with Use and Drop, Cards,
  Status, Order, Text speed, Save, a take-off and a landing; the notes say which `menu_screen`s it reached) and **title**:
  the intro, Start, Continue and the slot list.

`--class` reports (and runs the parts that can show) some classes; `--maps`, `--groups`, `--replays`, `--menus`,
`--title` and `--only` choose parts (`--list` says which are done). A part's result is a file under `.work/tour/parts/`,
so a long tour is resumable: only the parts not yet done (or made with another pack, ROM, core or `ssd_hd::RENDERER_VERSION`)
are played, `--fresh` plays them all again, `--report` reports the parts done and plays none. `--stride N` counts one
frame in N of a replay or a battle. The output is `.work/tour/coverage.json` (per class: pixels, art pixels, percent, the
pixels of poses the index lacks (`missing`) and of fade-ins and cut sprites it cannot place (`partial`), the things with
no art with their pixels, frames, where they were first seen (part and frame) and their cell on the sheet, the
unbound tiles by asset; and the parts, with their notes: scenes that did not end, menu screens reached) and one
thumbnail sheet per class `.work/tour/<class>.png` (a crop of the frame around each such thing, numbered), all derived
from the game and never tracked. It prints a table and the top of each list. The things with no art are named
`pose:<fighter|effect>:<missing|partial>:<key>` (the pose's key, as `hd-art report` gives it), `tile:<asset id>#<tile>` (a
character of the pack) and `char:<obj|bg>:<bits>bpp:<key>` (a character no asset of the pack has). Palette variants count
as covered when the art index finds them. The text counts as drawn from art where the HD font draws it (`--no-font`
leaves it to the filter). The classes of the pixels follow the scene and the pack asset (`tour_account.rs`): in a battle the
poses of the object layer are fighters (the command screen) and effects (an animation), BG pixels are backgrounds; the
glyph tiles of the UI set are the text, the rest of it cards and UI. `tools/hd-tour.toml` marks the classes that are
done and the threshold (99.5 %): the command exits 1 when one of them draws less from the art, 2 when a part failed. `--gaps DIR` also writes what the
art is made from: for each pose with no art (fighter, effect or field group; shadows included) a
`<kind>-<fate>-<key>.json` with its palette-index array, its colours (an effect's as a PNG), its place and the colours of
the palette it was drawn with, and for each portrait stream with a tile that has no art the frame where the
stream shows with the most contrast (`portrait-<asset>.png` and its box), so the missing art can be made from the game's
own pixels. `--audit DIR` writes, per part, `<part>.json`: for every fighter of the battle frames the source of its art
(`exact`: the art made for its pixels; `mirror`: the art of the pose facing the other way, drawn mirrored, which is how a
flipped formation member is drawn; `shaped`: a pose of the same shape in other colours; `held`: a piece of a bigger pose, a
fade-in's stripes or a cut sprite; `none`: the pixel-art filter's) with its art id and the first frame, and the frames that
show a fault: `borrowed` (a whole pose drawn from a piece of another pose's art), `none` (a whole fighter with no art),
`mixed` (fighters of one palette, some from art and some with none) and `split` (two fighters of the same pixels drawn from
different art). The poses whose pixels more than one unit has art for (the three Saibamen are one sprite in three colours,
each redrawn on its own) are all drawn from the unit with the most poses, in the colours of their own palettes.
`--formations DIR` writes `<part>.png`, the 4x picture with the art of the frame of the first 3000 of each battle where
most fighters show (a formation sheet), with `<part>.json` (the frame, and where each pose is and where its art came from). Nothing the tour writes is tracked.

    cargo run --release -p xtask -- hd-art scenes [--replays [name,..]] [--maps [N,..]] [--menus] [--title] [--only part,..]
        [--out DIR] [--jobs N] [--base SAVE]
    cargo run --release -p xtask -- hd-art scenes merge <out dir> <run dir>...

    cargo run --release -p xtask -- hd-art figures [replay|dir...] [--jobs N]
    cargo run --release -p xtask -- hd-art figures --walk MAP:PARTY [--frames N] [--dump] [--out DIR]

`hd-art figures` looks at every field frame for a figure drawn partly from art and partly from the original's pixels (a bare
group of the same palette touching an art-drawn figure) or from two different arts that overlap in one palette, and for a
figure drawn from art whose box does not fit its pose's (`stretched`: the art's width and height disagree on the scale by more
than two pixels of the original); the same checks are the `mixed` and `stretched` reasons of `hd-art scenes`. It lists each
case per replay and exits 1 on any. `--walk` boots a crafted map and party and walks it in the four directions (the map
decides who leads on screen; `--dump` lists every frame's poses, `--out DIR` writes the groups seen with no art as gap files).
The binding draws a field figure whole or not at all: a group with no art of its own is read as a smaller figure plus a piece
only where the piece has art of its own, never leaving an arm of it as the original's pixels.

the scene audit of the HD remaster: it plays the frames of the parts of the tour (every reference replay, a visit of every map
and a walk of the party's characters in the four directions on a map, the menus and the title; `--replays`, `--maps`,
`--menus` and `--title` choose, the battle groups are the tour's), three consecutive frames of every eight of a replay (and
every frame of the other parts: consecutive frames are what the flicker check needs), and finds the **scenes**: ranges of frames of one map and one
game state where something visible is not drawn from the art, as original pixels, as the filter's fallback, or as art that is
drawn wrong. A frame is faulty when it shows (reason, in the report) a map layer or a flight's plane (`map`), a big picture
(`picture`), a battle background (`background`), a character sprite (`sprite`) or a portrait (`portrait`) that is not drawn
from art; a sprite whose art is much darker than the original's pixels (`dark`, a silhouette) or draws under half of its
pixels (`thin`); a figure whose pixels did not change between two frames while its art did (`flicker`: the grouping of
the sprites differs, or a piece of the figure takes its sprites; not when an effect that the original draws over the figure in
one of the frames was drawn from art made with the figure in it); a field sprite whose art lacks a colour the original
has a good share of, or has far more skin than the original (`off_model`, the gi that became a bare chest); a field sprite whose art shows far more blue or purple than the original (`tint`, a ground shadow baked into the art in a hue); and a portrait or face that is not one character's art, whole
(`mixed_portrait`: its cells come from more than one picture, or from pictures of more than one portrait stream, or are in more than
one palette row, or sit beside pixels of the original that no picture draws, or the art is drawn in colours far from the palette it was
made in (a fade, which scales every colour, is not far), or the characters the game put on screen are of another stream than the
picture that draws them). The game's VRAM can hold the tiles of two faces in one portrait (the top rows of the face before, the
bottom rows of the speaker, each with its own palette row); the renderer then draws the portrait whole from the one picture that has
over two thirds of its pixels, or else the one in the colours nearest the game's, or else the one that holds the lower rows, in the
row of that picture's cells, and the audit still reports the frame (the characters of two streams) so that it is looked at. Frames faded to
black or in the mosaic of a fade are left out. The report is `.work/scenes/index.html` (one 4x frame of each scene, plain and
with the art, its part and frame range, the map and the game mode, and what is missing with its pixels) and `scenes.json`; the
poses with no art are also written to `.work/scenes/gaps/` (as `hd-tour --gaps` does), which is what art is made from.
`tools/hd-scenes.toml` lists the scenes that are accepted as they are (`part`, `map`, `reason` and `item`, each a substring or
a value, and a `note` that says why); the command exits 1 while a scene is not covered by one. `SCENES_PORTRAITS=1` prints, for every portrait of every frame, where its cells come from (pictures, streams, palette rows, how far the colours are from the art's), `SCENES_SNAP=N` writes the
picture of every Nth frame, `SCENES_TRACE=A-B` prints what the frames A to B are made of (their poses, with where each one's
art came from) and `SCENES_COLOUR=1` turns on the `colour` check (the backgrounds-only picture against the original's, per 16x16 square; off by default: it is unreliable until the two pictures are aligned, and faults field maps and Mode 7 planes), and the `dropout` check (always on) finds a layer whose content has art in the pack and that a frame shows without drawing it. `SCENES_FULL=1` counts every frame of the replays (not three of eight) and `SCENES_UNTIL=N` stops the replays after N frames, to look at one scene without playing the rest. `merge` puts the reports of runs made in parts (one `--out` folder each, below `<out dir>`) into one report with the exceptions applied.

    cargo run --release -p xtask -- menu-test field --save <file.sav> [--slot N] [name...]
    cargo run --release -p xtask -- menu-test town --replay <file.ssdreplay> --frames N [name...]
    cargo run --release -p xtask -- menu-test diff --save <file.sav> [--slot N] [name...]
    cargo run --release -p xtask -- menu-test items --save <file.sav> [name...]
    cargo run --release -p xtask -- menu-test cheats [--save <file.sav>] [--replays <dir>] [name...]

plays the HD menus (`--mode enhanced`) headless, as a player plays them, from a crafted save (its Continue, then the
field) or from a replay's frame N (the town: the shop replay at frame 4340, before its town menu opens). `field` runs
every entry of the field's command window and of the Menu list, the flying window, the cancels, and an abort (a plan
that cannot reach its state closes the menu, and the party walks again); each check is printed, and the exit code is 1
when one fails. `town` runs the town's command window (four rows, no Fly), its Item and Menu, and its Talk. `diff` plays
each choice with the HD driver and checks that its joypad words (its presses), replayed as a plain Classic run, end in
the same work RAM, every byte, and that the hand-written Classic presses of the same choice reach the same game state
(items, money, party, travel, map and position). `items` plays the bag on saves it crafts from `<file.sav>` (any save with a party, with the window's hooks installed):
the original's own Bubbles, Escape and Dragon Radar; a use of Bulma from a stack of 5 (one unit, the first fighter
healed: the scenarios answer the original's screens with A, as a player does); the Radar on its own (its screen is left to the player with B, and it stays in the bag); a healing item
(Roshi) on the first fighter with the second left alone; Vegeta on the Allies tab, greyed on the map (A says why);
`kinds`, every object the HD offers for use from its own bag with the party hurt (each use is taken and its stack is
consumed, a card's list and the Radar's screen are left to the player, and no use is refused); `dead-input`, the Item
screen's keys on an empty tab and the way back; a pickup, a purchase, a drop (one unit), a save then Continue, a save
changed by Classic play, a full bag, and the recording of an Enhanced session (format 2) played back to the same final
state.
Nothing is written but the report (the crafted saves of `items` go to a temporary directory it removes).

### Recompiler (M3)

    cargo run --release -p xtask -- recompile

turns the code of the cached ROM into Rust (`crates/ssd-recomp`): one function per function variant (entry address and
M/X widths, found from the trace runs of `.work/re/` when there are any (without them the generator works from the
symbols file and the static call graph alone, about 1,100 variants), `re/symbols.toml`, the `JSR`/`JSL` targets it reaches, the native
NMI and IRQ vectors and the sync points), with every memory access through the oracle's bus, the cycles of every bus
cycle charged as the hardware spends them, and a timing-window check before each basic block (long straight stretches
are cut into blocks of about 72 master cycles, so that a timing event keeps the original from running more than a few
instructions). `JMP (abs,X)` and `JSR (abs,X)` get a `match` over the entries of the table next to the jump (and the
traced targets), with a hand-over for any other target; `RTI` pulls P and the address and goes on there in the original.
Where it cannot follow the code (computed jumps without a table or a traced target, `STP`, `BRK`, `COP`, code in RAM) it
hands control back to the original code at that instruction, and so does the generated code at run time when a timing
event (an NMI, an IRQ, HDMA) can come within the next block, a frame was rendered, or a store changes the interrupt or DMA
registers (those are checked even when the address is computed). The output goes to `.work/recomp/generated.rs` (never
tracked: it is derived from the ROM) with `map.txt` next to it (every generated address and where it hands over), and the
command prints how many functions it generated and why it handed over where it did.

What makes a hand-over cheap: every block of a generated function is also registered as a routine (`rm_XXXXXX`, `Rust`
mode only), so when the original arrives at an address inside generated code, wherever it came from (a window that
failed, a `WAI`, an `XCE`, a call the generator could not follow, the return of an interrupt), the machine runs the
generated code from that block on if no event is due within it. The original thus executes only the instructions around
an event. The sync point of the idle loop is generated too: the code stops in front of it (`Ctx::sync_due`) when the
machine would report a sync event there, and the idle loop runs as Rust in between.

The generated code is compiled in by the `recomp` feature (`cargo run --release -p xtask --features recomp -- verify-replays`,
`-p ssd-app --features recomp`): `ssd-recomp-gen` includes `.work/recomp/generated.rs` (or the file `SSD_RECOMP_FILE`
names; empty if there is none) and `ssd_game::hook_table` registers every recompiled function as a routine in the mode
asked (`--hooks rust`, `verify`); routines of `re/symbols.toml` win on the same address. The cheat wrappers (Enhanced) hook routines that generated code would call directly, so their entries are stops of the generator (`ssd_game::cheats::entry_points`): the generated code hands over in front of them while a wrapper is installed, and the wrapper runs; with none installed (Classic) they run as generated. Without the feature, or without
the generated file, nothing changes. Debugging knobs for a build with the feature: `SSD_RECOMP_RANGE=LLLLLL-HHHHHH` keeps
the recompiled routines in that range of entries, `SSD_RECOMP_SKIP=A,B,...` drops entries, `SSD_RECOMP_CHAOS=N` makes
every N-th window check fail so the code hands over far more often than the timing requires (a green lockstep with it is a
test of the hand-over), `SSD_RECOMP_ONLY=1` leaves out the routines of `re/symbols.toml` and `SSD_RECOMP_TRACE=1` prints
the end of every recompiled run, `SSD_RECOMP_STATS=1` counts the hand-overs by address and cause (printed by `causes`).

    cargo run --release -p xtask --features recomp,histogram -- causes <replay>

plays a replay with the recompiled table in `Rust` mode and counts the opcode fetches of the original code by address
(feature `histogram` of `ssd-oracle`: a few percent slower, so it is not in the normal build). It prints which share of the
instructions still run as original code and why (an address the generated code hands over at, with its reason; an address
inside generated code, which is a timing, frame-end or DMA hand-over; RAM code; ROM that was never generated), the
busiest addresses and 128-byte blocks, the routines whose arrival was declined for a timing event, by how far the next
event was, and, with `SSD_RECOMP_STATS=1`, the hand-overs of the generated code by address and cause.

    cargo run --release -p xtask --features recomp -- framecheck <replay>

plays a replay on a pure machine and on one with only the recompiled routines, and compares them after each dispatch
(registers, WRAM, master cycles, whether the CPU is between instructions); it names the first dispatch that differs. It
is how timing errors of a few cycles were found: `verify-replays` only compares at sync events.

    cargo run --release -p xtask --features recomp -- recomp-cover <out.cov> <replay>...

plays replays with the recompiled table in `Rust` mode and writes which blocks of the generated code ran (the generated
`COV` flags, one per block); `fuzz-report` merges such files.

    cargo run --release -p xtask --features recomp -- fuzz-recomp harvest <dir> <replay>...
    cargo run --release -p xtask --features recomp -- fuzz-recomp run [--seed N] [--iters N] [--seeds N] [--part I/N] [--resume] <dir> <out.tsv>
    cargo run --release -p xtask --features recomp -- fuzz-report [--cover f.cov]... [--fuzz out.tsv]...

is the differential fuzzer of the generated code. `harvest` plays the replays on the pure original with a breakpoint at
every generated entry and saves the machine at a few arrivals of each (`<dir>/KKKKKK-<replay>-<n>.state`, 1.3 MB each) and
at a few places of each replay (the "worlds"). `run` takes those states, changes them (registers and flags, the WRAM bytes
the generated function reads, taken from a read log of its run, the joypad), and runs a `Verify` call of the entry from each
(the original on the live copy, the generated function on a snapshot), comparing registers, flags, WRAM, VRAM, CGRAM, OAM,
SRAM, the CPU's writes, the return and the cycles; it keeps the states that reach new blocks and mutates those again.
Entries no replay reaches get synthesised states: a world, the registers set to the entry, a return address pushed. It is
seeded (`--seed`, the seed tag and iteration of a divergence are in `<out>.div`), has an iteration and time budget per entry,
and counts the calls it cannot compare: an NMI enters the original, the original waits for the sound CPU, the generated
code hands over, the routine drops its caller's return address (an early exit), or an iteration never ends (the watchdog
stops the process with exit code 3, writes the iteration to `<out>.hang` and `--resume` skips it). `--repro KEY:TAG:IT`
finds, for one iteration, the first block of the generated function after which its registers, memories or cycles
differ from the original's after the same number of instructions, and lists the original's instructions there.

With the feature, `verify-replays` judges "called but never run in Rust" for the routines of the symbols file only when
the table has no recompiled routine (calls made by generated code do not reach the hooks).

### Headless reverse-engineering tools

For finding, understanding and porting game routines without a debugger or a window (they never open one:
`CONTRIBUTING.md`). Everything they print or write is derived from the game: stdout or `.work/` only, never tracked.

    cargo run --release -p xtask -- disasm <address|symbol> [--len N] [--m8|--m16] [--x8|--x16] [--dbr XX] [--dp XXXX] [--no-trace]

lists a routine of the cached ROM as 65816 assembly: LoROM with its mirrors, an address as `05B572`, `$05B572` or
`05:B572`, a symbol of `re/symbols.toml` (routine, sync point or table). It follows the M and X widths across `REP`,
`SEP`, `PLP` and `XCE` (after `PLP` or `XCE` they are unknown: an immediate that needs the width is marked `M?` or
`X?` with the assumed one), follows local branches to the end of the routine (a `JMP` counts as local only if it
lands in code the routine already reaches, else it ends the listing; `--len N` lists exactly N bytes instead), and
names routines, tables and variables from the symbols file (operands in low RAM are named through the data bank,
`--dbr`, and the direct page, `--dp`, default 0). The entry widths come from `--m8/--m16/--x8/--x16`, else the symbol's
`entry_m8`/`entry_x8`, else what the trace runs of `.work/re/` observed at the address, else M8 X16 marked as assumed;
where the tracking does not know a width the trace fills it in, and a disagreement is noted (`--no-trace` ignores
the traces). Bytes between two parts of a routine that no branch reaches are skipped and said so.

    cargo run --release -p xtask -- routines [--runs <dir>...] [--top N]

builds the routine table from the trace runs of `.work/re/` (made by `tools/re/run_trace.sh`, see `tools/re/README.md`;
`--runs` names the folders instead) and writes `.work/re/routines.csv` and `.work/re/routines.txt`, the text of the
top candidates (default 50) it also prints. Entries are the targets of `calls.csv` and `jumps.csv`, the interrupt
vectors and the routines of the symbols file that ran. A body is the executed instructions reachable from the entry,
decoded with the widths the trace saw, so it holds the paths the traces took, not every path. Per routine: size,
instructions run, calls per run (the entry's hits, or its returns when the body loops back to its entry), callers and
callees, return kind, entry widths, and the flags L leaf (no `JSR`/`JSL`), M register access `$2100-$43FF`, D DMA or
HDMA, S stack trick (`TCS`, `TXS`, `TSX`, `TSC`, or more pulls than pushes), I indirect jump or call, T tail call
into another routine, P pointer access, X block move or wait, R mixed returns; and the direct WRAM it reads and writes
(`7E0010-7E0011`; `7E1400+idx` is the base of an indexed access; pointers and the stack are not seen). Candidates
for porting are leaf, plain `RTS`/`RTL`, without M, D, S, I, T, X and R, up to 256 bytes, called at least once and not
yet in `re/symbols.toml`, ranked by instructions run per square root of the size (hot and small first); `excluded` in
the CSV says why a routine is not one.

    cargo run --release -p xtask -- probe <symbol> [--samples N] [--dir D] [--cycles N] [--frames N]
    cargo run --release -p xtask -- probe --address <24-bit> --returns rts|rtl [--name n] [--entry-m8 b] [--entry-x8 b]
    cargo run --release -p xtask -- probe <symbol> --verify [--quick] [--dir D]

without `--verify` plays the reference replays of `.work/replays/` (or `--dir`) with a counting hook at the routine
(the original always runs, nothing is replaced) and prints what is needed to write the Rust version: calls per
replay, the call sites (the address of the `JSR`/`JSL`), entry widths, direct page and data bank, the values of A, X and Y
at entry and exit and which registers a call changed, the cost of the calls in master cycles (a histogram: the
paths), the WRAM and register writes from the CPU write log (the 64 bytes under S are stack pushes and left out),
the direct WRAM the code reads (static, from its listing), `--samples` full calls (registers in and out, the values
of those reads at entry, the writes) and the listing. A call that an interrupt entered counts in `interrupted` and
gives no cost or writes. `--cycles N` counts the calls costing more than N (the symbol's `cycles` by default).
`--frames N` (default 3) is how long a call may take before it counts as without a return. A `timing window` line says
how many calls would fit the timing window of `Rust` mode with an exact per-call bound (no NMI, IRQ or HDMA within their
real cost from the entry, no GPDMA just before): the share a bound function could unlock for the routine.

With `--verify` the routine must be in `re/symbols.toml` with its Rust function in the registry: first every call of
every replay in `Verify` with only that routine hooked and sample 1 (counters: calls, verified, divergences,
cost_mismatch, cost_missing, cost_over_bound, interrupted, contract_mismatch, no_return, and the first divergences
in full), then the whole-run lockstep with only that routine in `Rust` (as `verify-replays`, so the table shows
`rust_runs` and `timing_fallback`; `--quick` skips it, it takes about a minute a replay). It exits 0 when everything
matched, 1 on a divergence or a cost problem, 2 when no replay called the routine or it never ran in Rust.

    cargo run --release -p xtask -- shots <script|timeline.txt|replay> [--frames a,b,c | --every N] [--upto N] [--sheet COLS]
        [--peek name,addr,...] [--watch name,addr,... [--watch-from N]] [--sram f.sav] [--sram-out f.sav]
        [--state-in f] [--state-out f] [--wram-out f] [--mesen-inputs f] [--mods PACK]... [--wram FRAME:ADDR=HEX]... [--out dir] [--no-png]

plays a script from power-on, headless, and writes the screen at the chosen frames as PNGs in `.work/shots/<script>/`
(or all of them in one half-size contact sheet with `--sheet COLS`, which costs one image to look at instead of
many), printing for each a fingerprint of the frame and the WRAM bytes of `--peek` (variables of the symbols file or
hex bus addresses). The script is a name of `ssd_game::scripts` (`battle`, `wander`, `menus`, `landing`, `story-1`) or a timeline
file: one `<frames> <keys>` per line (keys `up down left right a b x y l r start select` joined by commas, or `-`),
`repeat <k>` ... `end` blocks, `#` comments, and a first line `@prefix <battle|wander> <n>` that takes the first n frames
from that script; frames count from power-on. `--state-out` saves the machine state and `--state-in` continues from
one, so a long prefix is played once while a new part of a script is explored; `--wram-out` writes the 128 KiB of WRAM
(diff two to find a variable); `--mesen-inputs` writes the script as the input timeline of `tools/re/trace.lua`
(`DBZ_INPUTS`) and does not play it. `--watch` prints a line for every change of the named WRAM bytes
(`write frame N name=VALUE`: the CPU writes, frame by frame); with `--watch-from N` the run goes one CPU
cycle at a time from frame N on and the lines carry the program counter after the write, which finds the
code that changes a variable. `--sram-out` writes the cartridge's SRAM at the end: when the script ends with
the map menu's Save it is a save the game wrote itself, a consistent start for the next experiment. How the menu, landing and save replays were authored: a timeline file, a state
at the frame before it, and sheets of the frames in between. A timeline that is good goes into
`crates/ssd-game/scripts/` and gets an entry in `ssd_game::scripts::REPLAYS`, which `make-replays` records.
With `--hd-scale N` (and `--pack DIR`, else the cached pack) the frames are drawn as the Enhanced picture (`--enhanced-look crisp|smooth` picks its look, crisp by default), so a moment
saved with F12 (`--state-in`) is seen in HD with no window. `--gaps DIR` then prints, per frame, each pose with its art
and where the art comes from, the background slots and the line registers, writes the poses with no art into `DIR`,
and for a frame that shows a Mode 7 plane prints the plane's key and writes what `hd-art m7dump` writes into
`DIR/m7-<key>/` (the plane's art in `mode7\` is made from it). `--box x,y,w,h` maps a box of the picture, a letter per
pixel for the layer on top or the picture that draws it. `--wram FRAME:ADDR=HEX` (repeatable) is a host write: the bytes
HEX go into work RAM at the bus address ADDR (`7E0C36=80`, or `$7E:0C36`) before frame FRAME runs, as the Enhanced app's
host writes do; it stages a state, such as the Kaioken's (`docs/mods/art.md`). FRAME must be before the last frame of the run.

    cargo run --release -p xtask -- catalogue [--kind characters|cards|enemies|items|attacks|encounters]

prints the typed game catalogue (`crates/ssd-catalogue`) read from the cached ROM as text tables, every kind or the
one of `--kind`: the twelve characters with their level tables, the 105 units (enemies and other fighters) with stats
and rewards, the encounter groups, the rules of the card deal, the special attacks' names and the named objects of the
story (the closest the game has to items). Stdout only: what it prints is derived from the game and is never
written to a tracked file. The layouts of the tables are in `docs/re/data/`.

    cargo run --release -p xtask -- text-dump [out.tsv]

writes every string the game prints (the message engine's eight string tables, the names, the menu blocks of bank 2, the
ending's texts) decoded, one row per page keyed by its ROM coordinates (`str3:0x3E/p2`, `battle:93`, `unit:0x3B`...), to
`out.tsv` and the lists of the messages to `messages.tsv` beside it, and prints per table the entries, failures, rows with
unknown tiles and lines longer than their window. The files are derived from the game: never track them
(the default is `.work/text/strings-en.tsv`, which git ignores). The tables, the codes and the WRAM bytes that identify the string on
screen are in `docs/re/data/text-engine.md`.

### Saves

The battery save (8 KiB, `ssd.sav` in `saves\`) is three slots of party records, items, money, cards, map and
position, each with a checksum and a backup copy; `docs/re/data/save-format.md` has the layout and what is
known of each field, and `crates/ssd-save` reads and writes it (`SaveFile`, `Slot`; with a real file,
`SSD_SAVE_PATH=... cargo test -p ssd-save -- --ignored` checks the round trip). A save file is the player's
own data and is never tracked (`*.sav` is on the forbidden list of `publish-check`).

    cargo run --release -p xtask -- save-dump <file.sav>
    cargo run --release -p xtask -- save-edit <in.sav> <out.sav> [--slot N] [--to-slot N] [--set field=value ...]
    cargo run --release -p xtask -- replay-from-save <save.sav> <script.txt> <out.ssdreplay> [--slot N] [--title] [--cheats]
    cargo run --release -p xtask -- chapter-save <pack>... --chapter <id> --out <folder>

`chapter-save` writes the save slot a chapter of the packs starts from (`<folder>/ssd.sav`, and `<folder>/ssd.mods` when the chapter sets extended flags): the same two files a chapter session starts from, see `docs/mods/chapters.md`. It refuses to overwrite a file.

`save-dump` prints the status of each slot (empty, good, good from the backup, corrupt) and, with the ROM cached,
the party with names and stats, the items, the zeni, the map and the positions. `save-edit` copies the file and
changes slot N (1 to 3, default 1; `--to-slot` writes the result to another slot, so a slot can be copied) by
the `--set`s in order, then rewrites the slot's checksum and backup. The fields (names and levels need the
cached ROM; the raw forms do not):

- `party=goku,piccolo,gohan` sets the party (names or character ids 0 to 11, at most 9); a character with no record
  in use gets a new one at level 1 (a new record's byte `+1B`, whose meaning is not known, is its place in the
  list);
- `goku.level=N` gives the character the stats of level N the way the game builds them (battle power and KI of the
  rows up to N added, max HP, EXP and the rest from the row; HP and KI full; characters 8 to 10 have no level
  table); `goku.hp`, `.ki`, `.maxhp`, `.maxki`, `.bp` and `.exp` set one value, after the level;
- `zeni=N` (at most 999); `items=turtle,d.radar,DB'1,bulma` (object names or ids, at most ten; `turtle#2` is the second object of that name; each
  with the attribute byte the game keeps for it); `leader=x,y` and `follower=x,y` (map pixels); `map=N` (0 to 47: the map the
  party stands on, which also picks the scene that plays when it loads, see the doc);
- `flags=+3,-0x1F,+0x22` sets and clears story flags (`flags=none` clears all 128 first; numbers decimal or
  `0x`), `scene=N` sets the scene id; what the flags mean is in `docs/re/data/scenes.md`. The party of Nail,
  Tsumuri and Maima (no level table) is built from their unit rows (`party=nail`, then `nail.hp`...);
- `page:OFFSET=HEX` and `image:OFFSET=HEX` write raw bytes (the offset in hexadecimal into the page or the image
  of the slot, bytes as hexadecimal digits) for the fields not decoded yet.

`replay-from-save` writes a replay whose initial SRAM is the save and whose input is the title screen, Continue
(on the slot) and then the script, a timeline file as `shots` takes (`<frames> <keys>` per line, `repeat`;
frames count from the loaded game). The replay plays like any other (`--replay`, `verify-replays`), and
`shots <replay.ssdreplay>` plays it headless with its SRAM and writes screenshots, so a crafted save is checked
by what the game shows after Continue. `shots` also takes `--sram <save.sav>` for a timeline script.
`--mods <folder-or-.ssdpack>` (repeatable) runs the script on the image of those data packs with their redirectors
installed, as the Enhanced app does, so a pack's scene or message shows in the screens; a pack built with `--local`
is accepted. A replay's inputs and save are played as they are; only the data differs.

`tools/saves/secrets.sh BASE.sav [name...]` builds the crafted saves and replays that reach the story's hidden
and conditional events (Raditz's forced fight, the recruits, bosses of the Namek chapters, Nail against Frieza
and Porunga, Goku's Super Saiyan transformation, the ending and the Super Vegeta fight): each recipe is a
`save-edit` of the base save (any save with a used slot 1) and a timeline of `tools/saves/secrets/`; the
results go to `.work/demo/secret-<name>.sav` and `.work/replays/secret-<name>.ssdreplay`.
`docs/re/secrets-coverage.md` says what each one shows and what could not be reached.

`tools/saves/free-progression.sh [name...]` plays, on the same crafted saves, the gates of the story that read a level, once as
the original and once with the example pack `examples/free-progression` (`--mods`), and prints what the story did in each
(`docs/re/data/free-progression.md`, section 7).

`cargo xtask publish-check` and the other gate commands are listed in `CONTRIBUTING.md`.

`export-mlb [out-dir]` writes `dbz.mlb` (Mesen2 labels: Debugger → Labels → Import) and `symbols.lua` (names for
`tools/re/trace.lua`) from `re/symbols.toml` into `.work/re/` by default; it does not need the ROM. `tools/re/` holds
the reverse-engineering scripts only (see its README); everything they produce from the game goes under `.work/`.

## Design

The main README describes the architecture: the crates, how the lockstep verification works and how the
recompiler works.
