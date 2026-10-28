# Changelog

## 0.3.9 (2026-10-28)

### Features

- HD Save screen (Menu > Save): the three slots show what each holds (the party's portraits, the place, the leader's level, the money and
  the items) beside a strip for the game you are in; an empty slot says so, and A on a slot that holds a game asks "Overwrite slot 0N?"
  with the cursor on No. The original's first "Save?" question is gone: the HD answers it itself, and the original's own Save writes the slot
  (so the bag, the cheats and the mods keep saving with it). Enhanced only.
- HD Order screen (Menu > Order): the party in its slots with a portrait and a number each, slot 1 labelled as the one that leads
  on the map. A grabs a member, Up and Down carry it to its new place, A drops it, B undoes the last move (or goes back when nothing
  was moved), and "Apply order" has the original's own Order screen take the members one by one in the new order, so the game
  writes the order itself. Enhanced only.
- HD Status screen (Menu > Status): the original's two pages on one screen. The party is at the left (portrait, LV, HP and KI bars
  with figures, nine slots, a count like 5/9) and the member under the cursor at the right (big portrait, class kanji with its
  reading and meaning, HP, KI, BP, PLA., EXP, the EXP to the next level, and every technique with its KI cost). Up and Down
  change the member, B goes back and Start closes the menu. Read-only. Enhanced only.
- The HD Cards screen (Enhanced): the Menu list's Cards shows the hand of five drawn big from the game's own cards, dealt one
  by one as the original deals them, with a cursor over it (Left and Right), the kanji named with its reading and meaning,
  the attack and defence numbers (a Z counts as 10) and the party members the card favours (+25% battle power, the battle's
  own rule), with a help line in plain words. B goes back to the Menu list and Start closes the menu.
- HD Text screen (Menu > Text): the five message speeds drawn like the Options sliders, with a sample line that writes itself at the speed under
  the cursor; it is the same setting as Options > Gameplay > Text speed. A keeps the speed (the original's own Text screen takes it), B
  leaves the old one. Enhanced only.
- The HD pack carries Super Saiyan Goku's new art: his 27 battle poses with the hair standing up past the box of the original
  sprite, in the pack itself, with no mod needed (the `ssj-goku-hd` mod becomes redundant; Super Saiyan Kaioken keeps its
  Kaioken poses while the Kaioken is on). The pack can now hold pose art bigger than the original sprite's box (pose variants,
  `variants/` in `hd-pack.zip`); a data pack's variant of the same art wins over it. Download the new `hd-pack.zip` and install it
  again (the game does it on the next Enhanced launch). Enhanced only.

### Fixes

- HD portraits: Yamcha's corner portrait was a battle-worn Goku and is now Yamcha, Vegeta's showed only his forehead and now
  shows his face, and the Saibaman's picture has the bright green of the original. In the new `hd-pack.zip`.

## 0.3.8 (2026-10-28)

### Features

- Super Saiyan Goku HD, a new mod (`ssj-goku-hd.ssdpack`): the Super Saiyan Goku's 27 battle poses redrawn with his hair
  standing up, past the box of the original sprite, which kept it flat and cut it at the top. Super Saiyan Kaioken 1.1.0
  redraws its 27 Kaioken poses on that art (switch both on).
- Language packs (#12, contributed by @lming888 in #13): any `<code>.tsv` in the data folder is a language, with an
  optional `ui-<code>.tsv` for the HD menus and a `#name` / `#font` header; the first-launch picker and Options >
  Accessibility > Language list English and every language found. The choice is kept as its code (the old Hong Kong
  setting is carried over once). Enhanced only.
- Pose variants in every frame: a data pack's `[[pose_variant]]` takes `when = "always"`, drawn whenever the battle draws
  that pose, so a pack can replace any battle pose with bigger art. A `kaioken` variant of the same art wins while the
  Kaioken is on. Enhanced only.

### Fixes

- A long language name no longer runs past its button: the first-launch picker cuts it with an ellipsis to the room its
  button has, measured in the font it is drawn with (a Chinese name by its own glyphs), and the Options' Language row cuts
  the name shown by its cycler when the names do not fit side by side. A translation under a file name that is not a
  language code (`zh_TW.tsv`) is said in the log with the name it should have (`zh-tw.tsv`). Enhanced only.

## 0.3.7 (2026-10-27)

### Features

- Turbo option for held buttons, e.g. the ki training (#9): Options > Controls > Turbo makes a held A (or A and B) press and
  let go again 15, 20 or 30 times a second, for the keyboard, a gamepad and the touch buttons. Off by default; Enhanced only.

### Fixes

- The Gohan card can be called from the HD Items screen in the first Raditz battle (#8, contributed by @lming888 in #11):
  the original makes the story's Gohan usable there (the one way to hurt Raditz), and the HD list now follows that rule,
  on PC and Android. Enhanced only.
- Shop Sell list shows each stack's count (#10): the original lists a stack as one row, so a tag beside the price box now
  shows its units as the Item screen does (`×03`). Selling is fixed in Remastered too: a sale took a unit of the last
  object in the list instead of the one sold, and the row of a stack with units left vanished from the Sell screen at the
  first sale. A sale now takes one unit of the stack chosen and the row stays until the last unit is sold. Enhanced only.

## 0.3.6 (2026-10-22)

### Fixes

- **Porunga**:
  - A Porunga from the item spawner had no wish points (the game gives 150 with the object, 50 a wish): the wishes wrapped
    and never ended, and the wish window stayed with nothing left to choose. A Porunga without its points gets the game's
    (yours too).
  - Using Porunga from the HD Item screen could take another object away (the one after it in the list): the list is now
    held still while an object is in use, so a wish that changes Porunga's points finds Porunga where it was.
  - Remastered: the cursor and the marks of the wishes used are on the right lines (the game put them one line low, on
    the second wish's the third, on the third's an empty line). Classic is unchanged.
- **Save states keep the bag and the cheats**: a state (F5, the touch MENU's Save state, and the state kept when Android
  sends the game to the background) writes the HD bag and the cheats beside it (`slotN.hd`), and loading it brings them
  back. Before, a state loaded in a battle showed the game's old Item list and the cheats went back to the save's.

## 0.3.5 (2026-10-22)

### Fixes

- **Every object used from the HD Item screen was the first one of the bag** (Turtle, for example): the game reads the
  object under its own cursor when the cursor moves, not when A is pressed. The HD now moves the game's cursor onto the
  object it chose before Use, with the game's own arrows, on the map and in a battle.

## 0.3.4 (2026-10-22)

### Fixes

- **Using an object from the HD Item screen** (on the map and in a battle): the HD presses only the original's Use, and
  whatever the object asks next is yours, on the game's own screens: the fighter it is for (Turtle, ChiChi, Oolong, the
  healers), a card to change, Porunga's wishes, the lines of a message (Launch, Korin, Guru, Shenron). Before, the HD
  answered those by itself and covered the game's screens again: in a battle the use stood still for 10 to 25 seconds or
  never ended.

## 0.3.3 (2026-10-22)

### Fixes

- Using an object from the HD Item screen (in a battle or on the map) no longer stands still for two seconds after the
  choice: the press that picks Use or Drop waits for the original's window to be ready, not for a fixed two seconds.

## 0.3.2 (2026-10-22)

### New

- **Install a pack...** on the Data packs screen (Options > Gameplay > Data packs): pick a `.ssdpack` file with the system's
  file picker (Android) or the file dialog (Windows), and the game copies it into its mods folder and lists it, without a
  restart. It replaces the HD pack's button as the way to install a mod on Android.

## 0.3.1 (2026-10-22)

### Fixes

- Remastered: the normal Goku was drawn with the Super Saiyan's art in battle (golden hair from the first fight on), since
  the Super Saiyan set now has more poses than the normal one. The Super Saiyan art is drawn only while Super Saiyan Goku is
  in the party.

## 0.3.0 (2026-10-22)

The first release with data packs (mods), the first two mods and the Android APK.

- **Free progression is under test**: it plays through the early chapters in our runs, but the balance of the scaled
  enemies (a boss can be much harder or much easier than in the original) has not been played end to end yet. Save often
  and switch it off in Data packs if a fight goes wrong.
- **Android performance can be improved a lot**: the game runs the original machine cycle by cycle on one core, and the
  phone's system decides how fast that core may run. On a recent phone x1 is steady, x4 is about 220 frames a second, and a
  long session warms the phone and drains the battery more than it should.

### Data packs (mods)

- **Data packs** change the game in Remastered mode from files in the `mods` folder (a folder with a `pack.toml`, or a
  `.ssdpack` zip): maps made in Tiled, scenes and dialogue in a script language, text, units, attacks, encounter groups,
  the level curve, HD art of faces, maps and poses, chapters with their own saves, and named features. Classic is never
  changed by a pack, and a pack holds no code. The format is in `docs/mods/`.
- **Options > Gameplay > Data packs**: the packs of the folder, each on or off, in an order you choose, with their details
  and the reason a pack is refused (made for another ROM, missing a pack it needs, too big). Apply restarts at the title.
- **Chapters**: a pack's chapters are listed at the title, each with its own saves; Unlock chapters (Cheats) opens them all.
- A `.ssdpack` file carries its HD art (unpacked once into the cache). On Android a mod is installed with the HD pack's
  button (Options > Game > Install the HD pack): the game recognises a data pack and puts it in its mods folder.
- A save made with packs keeps their state (extended flags, battle power above the original's limit) in a file beside
  it (`ssd.mods`), so the original save stays readable by Classic.

### The first mods (separate downloads)

- **Super Saiyan Kaioken** (`ssj-kaioken.ssdpack`, not canon): Goku can use the Kaioken while he is a Super Saiyan, up
  to x10; while both are on, each move costs 10% of his maximum HP. It comes with the Super Saiyan poses redrawn with the
  red and gold aura.
- **Free progression** (`free-progression.ssdpack`, **under test**): the story moves on events, never on levels or battle
  power (Raditz comes after the opening's events, random battles never stop on a level, the recruits wait for the
  Saiyans, the Super Saiyan and the Ginyu fight follow the story), and every enemy, bosses included, follows your party's
  level, with a difficulty of easy, normal or hard (the `difficulty` line of the `pack.toml` inside
  the `.ssdpack`, which is a zip). Fights the story wants you to
  lose stay lost.

### New

- A screenshot (F12, or the debug panel's Save screenshot and Save 1x frame) now writes the machine's
  state beside the picture, named the same with `.ssdstate`, so the moment can be played back headless
  (`xtask shots --state-in`). The PNG is deflate-compressed.
- **Android**, from play on phones:
  - The touch controls keep clear of each other and of the picture on short screens (split screen, a window, a
    foldable's halves).
  - Touch options: **Picture** (Standard, Large, or Full screen with the controls floating over it) and **Show FPS**
    (the frame rate and the sound's health at the top).
  - The touch MENU has a GAME page: **Restart game** and **Quit**, each on a second tap.
  - The touch MENU's **State slot** picks the slot of Save state and Load state (1 to 4).
  - An HD picture fills the area it is given at any size.
- **Item spawner** (Cheats): any object into the bag, with its description and how many (up to 99). Story objects
  (the Dragon Balls, the Dragon Radar, Gohan...) are red, come one at a time and ask first, since they can break the
  story. A game that uses it carries the CHEATS badge.
- **Zeni gained** (Cheats): x1 to x10 on the zeni from selling and from the story's events (the game pays none for a
  battle), at most 999 as ever.
- **Cheats in battle**: F3, or the touch MENU's Cheats in a battle, opens the Cheats screen over the fight; a change
  applies at once and the story fights' own rules stay as they are.
- **Story fix (Remastered)**: in the Saiyans' chapter, Piccolo reaching level 9 (Gohan level 10 without him) no longer
  kills Krillin, Yamcha, Tien and Chiaotzu before they are recruited, nor stops the random battles: with more EXP or
  long training the story no longer runs ahead. Walking into the Saiyans before recruiting them still costs them, as
  in the original. Classic is unchanged.
- The Windows launchers find the game's data under `SSD_DATA_DIR` when it is set, as the game does.
- **Translations** supplied by the player: with a translation's files in the data folder (the game's text keyed by
  `xtask text-dump`, the HD menus' words and a font), Enhanced draws the story, the battle messages, the names and the
  HD menus in that language, asks for the language on the first launch and has a Language row in Options >
  Accessibility (and `--lang`). Nothing is bundled: without the files nothing changes. The first one supported is
  Traditional Chinese (Hong Kong).
- Android: the release build is signed when `SSD_KEYSTORE_PROPERTIES` names a key kept outside the repository.
- Android: the app has its icon, the four-star Dragon Ball in pixel art (adaptive, round and themed).

### HD art

- **Super Saiyan Goku** is drawn from his own art everywhere: in battle (all of his poses, the ki moves and their
  close-ups included), on the field and in the menus.
- The Saiyans' flight toward Earth: Nappa and Vegeta in the burst are drawn in HD (they were the original pixels), and
  Vegeta flying beside Nappa in the scanner scene is redrawn on-model.

### Fixes

- The bag: Porunga, the card changers and the other state objects stack like any object (a saved bag joins its stacks
  when it is loaded), and the battle's Item list is the HD bag, with every object and its count.
- Android: the app tells the system it is a game (game mode, performance mode where the phone has it) and gives it
  performance hints each frame; the Remastered picture is reused when nothing changed. On the test phone x4 went from about
  185 to about 225 frames a second.
- On a phone whose screen lowers its refresh rate when idle, the game keeps its speed (it fell to about 24 fps) and the
  sound no longer crackles: the playback speed is bent by at most 0.5% to keep the sound queue steady.
- The system bars stay hidden on Android 11 and later.

## 0.2.0 (2026-10-16)

### New

- **Options screen** (the Menu list's Options entry), saved with the game or kept for this PC as each page says:
  - **Difficulty**: Easy, Normal and Hard presets over the Cheats screen's numbers (Easy and Hard mark the save with
    the CHEATS badge).
  - **Gameplay**: five text speeds (Slowest to Fastest, the same setting as the Menu's Text entry), message auto-run,
    faster battle animations (x2 to x4, the outcome unchanged), random encounters.
  - **Video**: window or fullscreen, scale 2x to 8x, crisp or smooth look, 16:9 or the original 4:3, scanlines, a CRT
    filter and V-sync, applied at once with a Keep or Revert question.
  - **Audio**: volume, stereo or mono.
  - **Controls**: every button and shortcut remappable on the keyboard and on a gamepad.
  - **Accessibility**: larger text, high-contrast menus and a colour-blind-safe cursor.
- **Screenshots**: F12 (remappable) saves the picture as a PNG in `Pictures\DBZ SSD Recompiled\`; the debug panel can
  also save the original 256-pixel frame.
- **Fast-forward**: x8 keeps its full speed on a typical desktop.

### HD art

- About a hundred new character poses: Vegeta's story poses, battle fragments, small figures of the story pictures.
- The town rooms (houses, shops) in HD, with the same furniture drawn the same way in every room.
- Snake Way, King Kai's planet and the Shenron summoning drawn in HD.
- A redrawn Saibaman portrait, closer to the anime.
- The last field effects, the title's Mode 7 planes, the credits frame.

### Fixes

- Talk and Look in front of a character no longer leave the command window on screen for seconds.
- A dialogue portrait is never drawn half from another character's art.
- A field figure is never drawn half from HD art and half from the original's pixels.
- Purple blobs under some characters' feet removed.
- The Shenron summoning scene no longer turns dark with the dragon's body missing.
- A crash of the Remastered mode on King Kai's planet.
- The HD text font no longer switches off; Use in the Item screen works like the original; the map art is drawn right
  after leaving a building.

Known issues and the roadmap are in the README. The Android port is in progress and not part of this release.

## 0.1.0 (2026-10-14)

First public build: the original game verified frame by frame (Classic), the static recompiler, and the Remastered
mode with HD art, 16:9, HD text, the new menus, the bag and the Cheats screen.
