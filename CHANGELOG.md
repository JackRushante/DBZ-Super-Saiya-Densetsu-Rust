# Changelog

## Unreleased

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

- The Saiyans' flight toward Earth: Nappa and Vegeta in the burst are drawn in HD (they were the original pixels), and
  Vegeta flying beside Nappa in the scanner scene is redrawn on-model.

### Fixes

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
