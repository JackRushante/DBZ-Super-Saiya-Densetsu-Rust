//! The HD menus in the window (Enhanced mode). The menu driver of `ssd-game` decides what the HD
//! shows and which presses the original menu gets; `ssd-menus` draws the screen as a draw list, and
//! this layer paints it with egui over the game's frame area. The menu logic stays the original's.
//!
//! The names and texts come from the catalogue; the pictures of the objects are decoded from the
//! cached ROM the first time they are drawn and kept as egui textures for the run. The field as it
//! was when the menu opened is kept too: it is the backdrop the HD paints behind its windows.

use crate::paths;
use crate::theme;
use anyhow::Context;
use ssd_catalogue::{Catalogue, Portrait, RomView, face_portrait, object_portrait};
use ssd_game::Ruleset;
use ssd_game::cheats::BLOCK;
use ssd_game::data as d;
use ssd_game::menu::driver::{Binding, Curtain, Driver, Frame as HdFrame};
use ssd_game::menu::{SAVE_LIST_STEP, SAVE_SCREEN};
use ssd_hd::world::{WideFrame, WideHd, WideRun};
use ssd_menus::bag::Sidecar;
use ssd_menus::cheats::{CheatSidecar, MARKER, MARKER_BYTES, SUSPENDED};
use ssd_menus::layout::{self, palette};
use ssd_menus::screens::SlotList;
use ssd_menus::{Color, DrawList, Key, Op, Picture, Rect};
use ssd_oracle::{Frame, FrameLog, Joypad};
use std::collections::HashMap;
use std::path::PathBuf;

/// The sidecar file of the bags, beside the battery save.
const SIDECAR: &str = "ssd.bag";
/// The sidecar file of the cheats of the save slots, beside the bags.
const CHEATS_SIDECAR: &str = "ssd.cheats";
/// The gate of the cheats in work RAM: the suspended byte of the cheat block (nonzero while a scripted fight
/// has the battle cheats off).
const GATE: usize = (BLOCK - 0x7E_0000) as usize + SUSPENDED;
/// Where the block's marker starts in work RAM: before a run writes the block, the power-on fill is no gate.
const BLOCK_MARKER: usize = (BLOCK - 0x7E_0000) as usize + MARKER;
/// The frames a notice of the cheats stays on the field: about two seconds of the game's sixty.
const NOTICE_FRAMES: u32 = 120;
/// The notices: a scripted fight suspends the battle cheats, and the map brings them back.
const OFF_NOTICE: &str = "Cheats off for this fight";
const ON_NOTICE: &str = "Cheats back on";

/// The notices of the cheats on the field, from the gate of the cheat block: one comes when a scripted fight
/// suspends the battle cheats, and one when the map brings them back. A notice stays for [`NOTICE_FRAMES`]
/// frames. A gate that stays up through a fight shows its notice once.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CheatNotice {
    /// Whether the battle cheats were suspended at the last frame.
    suspended: bool,
    /// The notice showing, with the frames it has left.
    shown: Option<(&'static str, u32)>,
}

impl CheatNotice {
    /// Moves the notice on by one frame of `wram`.
    pub fn note(&mut self, wram: &[u8]) {
        let live =
            wram.get(BLOCK_MARKER..BLOCK_MARKER + MARKER_BYTES.len()) == Some(&MARKER_BYTES[..]);
        let suspended = live && wram.get(GATE).is_some_and(|&gate| gate != 0);
        if suspended != self.suspended {
            self.shown = Some((if suspended { OFF_NOTICE } else { ON_NOTICE }, NOTICE_FRAMES));
        }
        self.suspended = suspended;
        if let Some((_, frames)) = self.shown.as_mut() {
            *frames = frames.saturating_sub(1);
            if *frames == 0 {
                self.shown = None;
            }
        }
    }

    /// The text of the notice showing, if one is.
    #[must_use]
    pub fn text(&self) -> Option<&'static str> {
        self.shown.map(|(text, _)| text)
    }
}

/// The game mode of the title's "Which slot?" list (the Continue's slot list).
const TITLE_SLOTS_MODE: u8 = 0x0E;

/// The save-slot list the original shows, from work RAM: the title's Continue list by the game mode, and the Save's
/// slot list by the menu's screen and step. Other steps of the Save (its question and its completion) show none.
#[must_use]
pub fn slot_list(wram: &[u8]) -> Option<SlotList> {
    let byte = |address: u32| wram.get((address - 0x7E_0000) as usize).copied();
    if byte(d::GAME_MODE) == Some(TITLE_SLOTS_MODE) {
        return Some(SlotList::Continue);
    }
    (byte(d::MENU_SCREEN) == Some(SAVE_SCREEN) && byte(d::MENU_STEP) == Some(SAVE_LIST_STEP))
        .then_some(SlotList::Save)
}

/// The HD layer of a run: the menu driver, with the names it shows and the textures it paints, and
/// the 16:9 view of the game's frames the menus are drawn over.
pub struct Hd {
    driver: Driver,
    art: Art,
    wide: WideRun,
    /// The cheats' notices on the field.
    notice: CheatNotice,
}

impl Hd {
    /// The HD layer for the canonical ROM `rom`: its catalogue gives the names and texts, the bag (of the
    /// ruleset's rules) binds to the saves with the sidecar file of the saves directory, and the cheats of the
    /// ruleset are kept in their sidecar file the same way.
    ///
    /// # Errors
    ///
    /// The catalogue cannot be read from the ROM (it is not the canonical image), or a sidecar file
    /// exists and cannot be read.
    pub fn new(rom: &[u8], ruleset: Ruleset) -> anyhow::Result<Self> {
        let catalogue = Catalogue::from_rom(rom)?;
        let sidecar = read_sidecar()?;
        let driver =
            Driver::with_bag(ssd_catalogue::menu_labels(&catalogue), ruleset.bag(), sidecar);
        let driver =
            if ruleset.cheats() { driver.with_cheats(read_cheats_sidecar()?) } else { driver };
        let driver =
            driver.with_options(crate::options::read_slots()?, crate::options::read_settings()?);
        Ok(Self {
            driver,
            art: Art {
                rom: rom.to_vec(),
                pack: crate::hd_art::pack(rom),
                cards: crate::hd_art::pack(rom).and_then(|pack| {
                    match ssd_hd::CardArt::load(&pack.dir) {
                        Ok(cards) => cards,
                        Err(error) => {
                            log::warn!("the HD card art: {error}");
                            None
                        }
                    }
                }),
                card_textures: HashMap::new(),
                textures: HashMap::new(),
                field: None,
                backdrop: None,
            },
            wide: {
                let mut wide = WideRun::new(rom);
                wide.set_art(crate::hd_art::pack(rom));
                wide.set_font(crate::hd_art::font(rom));
                wide
            },
            notice: CheatNotice::default(),
        })
    }

    /// The machine's state was replaced (a state loaded): see [`Driver::reset`].
    pub fn reset(&mut self) {
        self.driver.reset();
    }

    /// A replay plays: the bag is bound by the replay's recorded events, not by the saves.
    pub fn set_replaying(&mut self) {
        self.driver.set_binding(Binding::Recorded);
    }

    /// Writes the sidecar file of the bags (after a save changed one).
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn save_sidecar(&self) -> anyhow::Result<()> {
        write_beside(&sidecar_path(), &self.driver.sidecar().to_bytes())
    }

    /// Writes the sidecar file of the cheats (after a save changed one).
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn save_cheats_sidecar(&self) -> anyhow::Result<()> {
        write_beside(&cheats_sidecar_path(), &self.driver.cheat_sidecar().to_bytes())
    }

    /// Writes the options of the save slots (after a save changed one).
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn save_options_sidecar(&self) -> anyhow::Result<()> {
        crate::options::write_slots(self.driver.option_slots())
    }

    /// The cheats' notices, from the gate of the cheat block (see [`CheatNotice::note`]), once a frame.
    pub fn note_cheats(&mut self, wram: &[u8]) {
        self.notice.note(wram);
    }

    /// The HD's picture over the field while no menu is on screen: the cheats' notice, while one shows, and the
    /// CHEATS badges of the save-slot list the original shows (see [`slot_list`]), for the slots saved with a cheat on.
    #[must_use]
    pub fn overlay(&self, wram: &[u8], sram: Option<&[u8]>) -> Option<DrawList> {
        let mut list = DrawList::new();
        if let Some(text) = self.notice.text() {
            list.append(ssd_menus::screens::notice(text));
        }
        if let (Some(which), Some(sram)) = (slot_list(wram), sram) {
            list.append(ssd_menus::screens::slot_badges(which, self.driver.marked_slots(sram)));
        }
        (!list.ops().is_empty())
            .then(|| crate::options::access::restyle(&self.driver.options().pc, list))
    }

    /// The Item screen shows `tab` of the bag.
    pub fn set_tab(&mut self, tab: ssd_menus::view::Tab) {
        self.driver.set_tab(tab);
    }

    /// The HD's frame: the joypad and the host events of the frame, given the work RAM and the
    /// battery save (see [`Driver::frame`]).
    pub fn frame(&mut self, wram: &[u8], sram: Option<&[u8]>, live: Joypad) -> HdFrame {
        self.driver.frame(wram, sram, live)
    }

    /// A replay's recorded application event, applied to the bag (see [`Driver::replay_event`]).
    ///
    /// # Errors
    ///
    /// The payload is not what the event holds.
    pub fn replay_event(&mut self, tag: u8, payload: &[u8], wram: &[u8]) -> anyhow::Result<()> {
        self.driver.replay_event(tag, payload, wram).map(|_| ()).map_err(anyhow::Error::msg)
    }

    /// The driver of the menus: the curtain, the presses and the HD's screens.
    #[must_use]
    pub fn driver(&self) -> &Driver {
        &self.driver
    }

    /// The options of this PC the menus hold, to change from outside (the touch controls' layout).
    pub fn pc_options_mut(&mut self) -> &mut ssd_menus::options::Values {
        self.driver.pc_options_mut()
    }

    /// Whether a choice of the HD runs its presses now (see [`Driver::busy`]).
    #[must_use]
    pub fn busy(&self) -> bool {
        self.driver.busy()
    }

    /// A navigation key of the HD on the menu of `wram` (see [`Driver::key`]).
    pub fn key(&mut self, key: Key, wram: &[u8]) {
        self.driver.key(key, wram);
    }

    /// Whether the Controls page waits for a key or button, and for which device (see [`Driver::listening`]).
    #[must_use]
    pub fn listening(&self) -> Option<ssd_menus::options::controls::Device> {
        self.driver.listening()
    }

    /// A raw press for the key box that waits for one (see [`Driver::capture`]).
    pub fn capture(&mut self, capture: ssd_menus::options::controls::Capture) {
        self.driver.capture(capture);
    }

    /// The setup a button of the Options' Game page asked for, once (Android restarts the game into it).
    #[cfg(target_os = "android")]
    pub fn take_setup_request(&mut self) -> Option<ssd_menus::options::Opens> {
        self.driver.take_setup_request()
    }

    /// The player's press on the field opens the menu (see [`Driver::open`]).
    pub fn open(&mut self) {
        self.driver.open();
    }

    /// The HD's picture of the menu now, if the curtain is up.
    #[must_use]
    pub fn scene(&self, wram: &[u8]) -> Option<DrawList> {
        self.driver.scene(wram)
    }

    /// The 16:9 view of the frame that just ran (see [`WideRun::note`]); `None` when the frame has no
    /// render log or the view refuses it, and the window shows the PPU's own frame.
    pub fn note_frame(&mut self, wram: &[u8], log: Option<&FrameLog>) -> Option<WideFrame> {
        self.wide.note(wram, log)
    }

    /// The 16:9 view of the frame that just ran as the Enhanced picture at the scale and look of
    /// `options`, with the HD art (see [`WideRun::note_hd`]).
    pub fn note_frame_hd(
        &mut self,
        wram: &[u8],
        log: Option<&FrameLog>,
        options: ssd_hd::HdOptions,
    ) -> Option<WideHd> {
        self.wide.note_hd(wram, log, options)
    }

    /// A frame that ran and is not drawn (a fast-forward's frame the window never shows): the 16:9
    /// view only follows its work RAM, so the frame drawn next starts from the right state.
    pub fn skip_frame(&mut self, wram: &[u8]) {
        self.wide.skip(wram);
    }

    /// [`Hd::redraw_frame_hd`] for a fast-forward, which shows its pictures a window frame late:
    /// the frame is drawn in the background and the picture returned is the one drawn for the
    /// call before (see [`WideRun::redraw_hd_pipelined`]).
    pub fn redraw_frame_hd_pipelined(
        &mut self,
        log: Option<&FrameLog>,
        options: ssd_hd::HdOptions,
    ) -> Option<WideHd> {
        self.wide.redraw_hd_pipelined(log, options)
    }

    /// The 16:9 view of the frame last skipped ([`Hd::skip_frame`]), after all.
    pub fn redraw_frame(&mut self, log: Option<&FrameLog>) -> Option<WideFrame> {
        self.wide.redraw(log)
    }

    /// [`Hd::redraw_frame`] as the Enhanced picture of [`Hd::note_frame_hd`].
    pub fn redraw_frame_hd(
        &mut self,
        log: Option<&FrameLog>,
        options: ssd_hd::HdOptions,
    ) -> Option<WideHd> {
        self.wide.redraw_hd(log, options)
    }

    /// [`Hd::note_field`] for an Enhanced picture: the backdrop is the picture as it is drawn.
    pub fn note_field_picture(&mut self, picture: &ssd_hd::HdFrame) {
        if self.driver.curtain() == Curtain::Down && !self.driver.busy() {
            self.art.note_field_rgba(picture.width, picture.height, &picture.rgba);
        }
    }

    /// Keeps the frame just rendered as the backdrop, while the field is free and no menu press is
    /// under way: the menu opens over it, and the field does not move while the menu is open.
    pub fn note_field(&mut self, frame: &Frame<'_>) {
        if self.driver.curtain() == Curtain::Down && !self.driver.busy() {
            self.art.note_field(frame);
        }
    }

    /// The textures of the pictures and of the backdrop, for painting.
    #[must_use]
    pub fn art(&mut self) -> &mut Art {
        &mut self.art
    }
}

/// The frame the menu opened on, as RGBA pixels, with the generation it was copied at.
struct FieldImage {
    generation: u64,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// The pictures of the objects, and the backdrop, as egui textures decoded on first use.
pub struct Art {
    rom: Vec<u8>,
    /// The HD pack whose art stands for the pictures of the objects, when there is one.
    pack: Option<std::sync::Arc<ssd_hd::pack::Pack>>,
    /// The art of the battle's cards, when the pack has it: the Cheats screen's preview is drawn from it.
    cards: Option<ssd_hd::CardArt>,
    /// The preview card as a texture, by its Z corners (attack, defence), made on first use.
    card_textures: HashMap<(bool, bool), Option<egui::TextureHandle>>,
    textures: HashMap<Picture, Option<egui::TextureHandle>>,
    field: Option<FieldImage>,
    /// The texture of the field at a generation, uploaded when it is first painted.
    backdrop: Option<(u64, egui::TextureHandle)>,
}

impl Art {
    #[cfg(any(target_os = "android", test))]
    /// An art with no ROM and no pack: it paints what needs no picture (the first-launch screens).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            rom: Vec::new(),
            pack: None,
            cards: None,
            card_textures: HashMap::new(),
            textures: HashMap::new(),
            field: None,
            backdrop: None,
        }
    }

    /// Copies the frame as the backdrop (a new generation each time).
    fn note_field(&mut self, frame: &Frame<'_>) {
        let mut rgba = Vec::with_capacity(frame.pixels.len() * 4);
        for color in frame.pixels {
            rgba.extend_from_slice(&[color.r, color.g, color.b, 255]);
        }
        self.keep_field(frame.size.width, frame.size.height, rgba);
    }

    /// Copies RGBA pixels (an Enhanced picture) as the backdrop.
    fn note_field_rgba(&mut self, width: u32, height: u32, rgba: &[u8]) {
        self.keep_field(width, height, rgba.to_vec());
    }

    fn keep_field(&mut self, width: u32, height: u32, rgba: Vec<u8>) {
        let generation = self.field.as_ref().map_or(1, |field| field.generation + 1);
        self.field = Some(FieldImage { generation, width, height, rgba });
    }

    /// The texture of the backdrop, uploaded if the field changed since it was last painted; `None`
    /// before any field was copied. It is painted over the game picture, in the same rect.
    fn backdrop(&mut self, ctx: &egui::Context) -> Option<egui::TextureId> {
        let field = self.field.as_ref()?;
        if field.width == 0 || field.height == 0 {
            return None;
        }
        let stale =
            self.backdrop.as_ref().is_none_or(|(generation, _)| *generation != field.generation);
        if stale {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [field.width as usize, field.height as usize],
                &field.rgba,
            );
            let texture = ctx.load_texture("field backdrop", image, egui::TextureOptions::NEAREST);
            self.backdrop = Some((field.generation, texture));
        }
        self.backdrop.as_ref().map(|(_, texture)| texture.id())
    }

    /// The texture of the preview card with the Z corners given and its size in pixels, drawn from the pack's
    /// card art on first use; `None` when the pack has no card art (or lacks a tile of the card).
    fn card(
        &mut self,
        ctx: &egui::Context,
        attack_z: bool,
        defence_z: bool,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        let cards = self.cards.as_ref()?;
        let entry = self.card_textures.entry((attack_z, defence_z)).or_insert_with(|| {
            let card = cards.preview(attack_z, defence_z)?;
            let image =
                egui::ColorImage::from_rgba_unmultiplied([card.width, card.height], &card.rgba);
            Some(ctx.load_texture(
                format!("preview card {attack_z} {defence_z}"),
                image,
                egui::TextureOptions::LINEAR,
            ))
        });
        entry.as_ref().map(|texture| (texture.id(), texture.size_vec2()))
    }

    /// The texture of `picture` (an object's, or a corner face), decoded on first use; `None` when the ROM has no
    /// picture for it.
    fn texture(&mut self, ctx: &egui::Context, picture: Picture) -> Option<egui::TextureId> {
        let rom = &self.rom;
        let pack = self.pack.as_deref();
        let entry = self.textures.entry(picture).or_insert_with(|| {
            let view = RomView::new(rom).ok()?;
            let (portrait, name) = match picture {
                Picture::Object(object) => {
                    (object_portrait(view, object).ok()?, format!("object {object}"))
                }
                Picture::Face(face) => {
                    (face_portrait(view, face).ok()?, format!("face {face:02X}"))
                }
            };
            // The approved art of the picture itself (found by its characters), when the pack
            // has it: never another picture's art
            if let Some(image) = pack.and_then(|pack| hd_picture(pack, &portrait)) {
                return Some(ctx.load_texture(
                    format!("{name} (HD)"),
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [ssd_catalogue::art::SIDE; 2],
                &rgba(&portrait),
            );
            Some(ctx.load_texture(name, image, egui::TextureOptions::NEAREST))
        });
        entry.as_ref().map(egui::TextureHandle::id)
    }
}

/// The HD art of the picture `portrait` (the object's own half of its set, in the object's
/// palette), as an egui image: the pack's art for those very characters, recoloured to the
/// object's palette when it was made for another.
fn hd_picture(pack: &ssd_hd::pack::Pack, portrait: &Portrait) -> Option<egui::ColorImage> {
    let row: [[u8; 3]; 16] = std::array::from_fn(|index| snes_rgb(portrait.palette[index]));
    let picture = pack.menu_picture(&portrait.characters, &row)?;
    Some(egui::ColorImage::from_rgba_unmultiplied([picture.width, picture.height], &picture.rgba))
}

/// The sidecar file's path: `ssd.bag` in the saves directory.
fn sidecar_path() -> PathBuf {
    paths::saves_dir().join(SIDECAR)
}

/// The sidecar file of the cheats' path: `ssd.cheats` in the saves directory.
fn cheats_sidecar_path() -> PathBuf {
    paths::saves_dir().join(CHEATS_SIDECAR)
}

/// Writes `bytes` to `path`: beside the file and renamed over it, so a crash never leaves half a file. The writer
/// of the player's sidecar and option files: a run that does not persist writes nothing (see [`paths::persists`]).
pub(crate) fn write_beside(path: &std::path::Path, bytes: &[u8]) -> anyhow::Result<()> {
    if !paths::persists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    let partial = PathBuf::from(name);
    std::fs::write(&partial, bytes).with_context(|| format!("writing {}", partial.display()))?;
    std::fs::rename(&partial, path).with_context(|| format!("replacing {}", path.display()))
}

/// The cheats' sidecar: empty when the file does not exist yet. A file that is not a cheats file (damaged,
/// or of another version) is kept aside as `ssd.cheats.bad`, and the cheats of every save start off.
fn read_cheats_sidecar() -> anyhow::Result<CheatSidecar> {
    let path = cheats_sidecar_path();
    match std::fs::read(&path) {
        Ok(bytes) => Ok(CheatSidecar::from_bytes(&bytes).unwrap_or_else(|error| {
            keep_damaged(&path, &path.with_extension("cheats.bad"), &error);
            CheatSidecar::empty()
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(CheatSidecar::empty()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// The sidecar of the bags: empty when the file does not exist yet. A file that is not a bag file
/// (damaged, or of another version) is kept aside as `ssd.bag.bad` and the run starts with none:
/// every bag is then rebuilt from the original's list, and the run is not refused.
fn read_sidecar() -> anyhow::Result<Sidecar> {
    let path = sidecar_path();
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Sidecar::from_bytes(&bytes).unwrap_or_else(|error| {
            keep_damaged(&path, &path.with_extension("bag.bad"), &error);
            Sidecar::empty()
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Sidecar::empty()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// A damaged sidecar, read as none: a run that persists keeps the file aside as `aside`; a run that does not persist
/// leaves it as it is, since it writes nothing.
fn keep_damaged(path: &std::path::Path, aside: &std::path::Path, error: impl std::fmt::Display) {
    if !paths::persists() {
        log::warn!("{}: {error}; read as none, the file is left as it is", path.display());
        return;
    }
    log::warn!("{}: {error}; kept as {}", path.display(), aside.display());
    if let Err(error) = std::fs::rename(path, aside) {
        log::warn!("{}: {error}", aside.display());
    }
}

/// The picture as RGBA pixels: index 0 transparent, the other colours as the SNES shows them (each
/// 5-bit channel widened to 8 bits by repeating its top bits).
#[must_use]
pub fn rgba(portrait: &Portrait) -> Vec<u8> {
    let mut out = Vec::with_capacity(portrait.pixels.len() * 4);
    for &index in &portrait.pixels {
        if index == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        let [red, green, blue] = snes_rgb(portrait.palette[usize::from(index)]);
        out.extend_from_slice(&[red, green, blue, 255]);
    }
    out
}

/// A 15-bit SNES colour (`0BBBBBGGGGGRRRRR`) as 8-bit red, green and blue.
#[must_use]
pub fn snes_rgb(colour: u16) -> [u8; 3] {
    let channel = |shift: u16| {
        let value = ((colour >> shift) & 0x1F) as u8;
        (value << 3) | (value >> 2)
    };
    [channel(0), channel(5), channel(10)]
}

/// The navigation keys of the HD that were pressed between two frames of the host's joypad: the
/// d-pad, A to confirm, B to cancel, the shoulders to change the bag's tab or the options page (L goes back), Y and
/// Select to reset an options page or all of them (the keyboard and the
/// gamepad alike). A held button counts once, when it goes down.
#[must_use]
pub fn keys_pressed(previous: Joypad, now: Joypad) -> Vec<Key> {
    [
        (now.up && !previous.up, Key::Up),
        (now.down && !previous.down, Key::Down),
        (now.left && !previous.left, Key::Left),
        (now.right && !previous.right, Key::Right),
        (now.a && !previous.a, Key::Confirm),
        (now.b && !previous.b, Key::Cancel),
        (now.l && !previous.l, Key::TabBack),
        (now.y && !previous.y, Key::ResetPage),
        (now.select && !previous.select, Key::ResetAll),
        (now.r && !previous.r, Key::Tab),
    ]
    .into_iter()
    .filter_map(|(pressed, key)| pressed.then_some(key))
    .collect()
}

/// The egui colour of a draw list colour.
fn colour(color: Color) -> egui::Color32 {
    let [red, green, blue, alpha] = color.0;
    egui::Color32::from_rgba_unmultiplied(red, green, blue, alpha)
}

/// Where a logical point lies in the picture the menu is painted over: the logical space is scaled
/// uniformly to fit the picture and centred in it, so it never leaves the picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Points per logical pixel.
    pub scale: f32,
    /// The point of the logical origin.
    pub origin: egui::Pos2,
}

impl Fit {
    /// The fit of the logical space in `picture`.
    #[must_use]
    pub fn new(picture: egui::Rect) -> Self {
        Self::sized(picture, layout::WIDTH, layout::HEIGHT)
    }

    /// The fit of a logical space of `width` x `height` in `picture` (the first-launch screens have their own).
    #[must_use]
    pub fn sized(picture: egui::Rect, width: f32, height: f32) -> Self {
        let scale = (picture.width() / width).min(picture.height() / height).max(0.0);
        let size = egui::vec2(width, height) * scale;
        Self { scale, origin: picture.center() - size / 2.0 }
    }

    /// The point of a logical point.
    #[must_use]
    pub fn point(&self, x: f32, y: f32) -> egui::Pos2 {
        self.origin + egui::vec2(x, y) * self.scale
    }

    /// The egui rectangle of a logical rectangle.
    #[must_use]
    pub fn rect(&self, rect: Rect) -> egui::Rect {
        egui::Rect::from_min_size(
            self.point(rect.x, rect.y),
            egui::vec2(rect.w, rect.h) * self.scale,
        )
    }
}

/// Paints a draw list over `picture`, the rect the game's frame is drawn in, with the textures of
/// `art`. The backdrop covers the picture; the logical layout is fitted to it (see [`Fit`]). The
/// painting is clipped to the picture as well, so nothing can be drawn over the letterbox.
pub fn paint(ui: &egui::Ui, picture: egui::Rect, list: &DrawList, art: &mut Art) {
    paint_fit(ui, picture, &Fit::new(picture), list, art);
}

/// [`paint`] with the fit of the list's own logical space.
pub fn paint_fit(ui: &egui::Ui, picture: egui::Rect, fit: &Fit, list: &DrawList, art: &mut Art) {
    let fit = *fit;
    let painter = ui.painter_at(picture);
    for op in list.ops() {
        match op {
            Op::Backdrop => {
                if let Some(id) = art.backdrop(ui.ctx()) {
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    painter.image(id, picture, uv, egui::Color32::WHITE);
                }
            }
            Op::Fill { rect, color } => {
                painter.rect_filled(fit.rect(*rect), 0.0, colour(*color));
            }
            Op::Disc { rect, color } => {
                let area = fit.rect(*rect);
                painter.circle_filled(area.center(), area.width() / 2.0, colour(*color));
            }
            Op::Outline { rect, width, color } => {
                let stroke = egui::Stroke::new(width * fit.scale, colour(*color));
                painter.rect_stroke(fit.rect(*rect), 0.0, stroke, egui::StrokeKind::Inside);
            }
            Op::Window { rect } => window(&painter, &fit, *rect),
            Op::WindowHc { rect } => window_contrast(&painter, &fit, *rect),
            Op::Pointer { rect, left, fill, edge, ring } => {
                let (tip, base) =
                    if *left { (rect.x, rect.right()) } else { (rect.right(), rect.x) };
                let points = [(base, rect.y), (tip, rect.middle_y()), (base, rect.bottom())]
                    .map(|(x, y)| fit.point(x, y));
                for (colour_of, width) in [(edge, POINTER_EDGE), (ring, POINTER_RING)] {
                    let stroke = egui::Stroke::new(width * fit.scale, colour(*colour_of));
                    painter.add(egui::Shape::convex_polygon(
                        points.to_vec(),
                        colour(*fill),
                        stroke,
                    ));
                }
            }
            Op::Line { from, to, width, color } => {
                let stroke = egui::Stroke::new(width * fit.scale, colour(*color));
                painter.line_segment([fit.point(from.0, from.1), fit.point(to.0, to.1)], stroke);
            }
            Op::Text { x, y, text, size, color, align } => {
                let anchor = match align {
                    ssd_menus::Align::Left => egui::Align2::LEFT_CENTER,
                    ssd_menus::Align::Centre => egui::Align2::CENTER_CENTER,
                    ssd_menus::Align::Right => egui::Align2::RIGHT_CENTER,
                };
                let font = theme::game(size * fit.scale);
                painter.text(fit.point(*x, *y), anchor, text, font, colour(*color));
            }
            Op::Cursor { x, y } => {
                let points =
                    [(0.0, 0.0), (layout::CURSOR_WIDTH, 9.0), (0.0, layout::CURSOR_HEIGHT)]
                        .map(|(dx, dy)| fit.point(x + dx, y + dy));
                let stroke = egui::Stroke::new(2.0 * fit.scale, colour(palette::DARK_RED));
                painter.add(egui::Shape::convex_polygon(
                    points.to_vec(),
                    colour(palette::ORANGE),
                    stroke,
                ));
            }
            Op::Arrow { x, y, left } => {
                let (tip, base) =
                    if *left { (0.0, layout::CURSOR_WIDTH) } else { (layout::CURSOR_WIDTH, 0.0) };
                let points = [(base, 0.0), (tip, 9.0), (base, layout::CURSOR_HEIGHT)]
                    .map(|(dx, dy)| fit.point(x + dx, y + dy));
                let stroke = egui::Stroke::new(2.0 * fit.scale, colour(palette::DARK_RED));
                painter.add(egui::Shape::convex_polygon(
                    points.to_vec(),
                    colour(palette::ORANGE),
                    stroke,
                ));
            }
            Op::Portrait { rect, picture } => portrait(ui, &painter, &fit, art, *rect, *picture),
            Op::Card { rect, attack_z, defence_z } => {
                match art.card(ui.ctx(), *attack_z, *defence_z) {
                    Some((id, size)) => {
                        // The card's art keeps its proportions, centred in the rect
                        let area = fit.rect(*rect);
                        let scale = (area.width() / size.x).min(area.height() / size.y);
                        let drawn = egui::Rect::from_center_size(area.center(), size * scale);
                        let uv =
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        painter.image(id, drawn, uv, egui::Color32::WHITE);
                    }
                    None => paint_fit(
                        ui,
                        picture,
                        &fit,
                        &ssd_menus::screens::pixel_card(*rect, *attack_z, *defence_z),
                        art,
                    ),
                }
            }
            Op::Bar { rect, fraction, color } => {
                painter.rect_filled(fit.rect(*rect), 0.0, colour(palette::WHITE));
                let well = rect.grow(-layout::BAR_BORDER);
                painter.rect_filled(fit.rect(well), 0.0, colour(palette::BLACK));
                let filled = Rect::new(well.x, well.y, well.w * fraction.clamp(0.0, 1.0), well.h);
                painter.rect_filled(fit.rect(filled), 0.0, colour(*color));
            }
        }
    }
}

/// The stroke widths of a [`Op::Pointer`]: the wide outline and the narrower line on it.
const POINTER_EDGE: f32 = 5.0;
const POINTER_RING: f32 = 2.0;

/// A menu window in high contrast: a white line outside a black frame, and white paper.
fn window_contrast(painter: &egui::Painter, fit: &Fit, rect: Rect) {
    let black = colour(palette::BLACK);
    let white = colour(palette::WHITE);
    painter.rect_filled(fit.rect(rect.grow(layout::LINE)), 0.0, white);
    painter.rect_filled(fit.rect(rect), 0.0, black);
    painter.rect_filled(fit.rect(rect.grow(-layout::FRAME)), 0.0, white);
}

/// A menu window: the dark-red line outside the orange frame, the frame, the dark-red line inside
/// it, the cream paper, and a corner cap on each corner.
fn window(painter: &egui::Painter, fit: &Fit, rect: Rect) {
    let dark = colour(palette::DARK_RED);
    painter.rect_filled(fit.rect(rect.grow(layout::LINE)), 0.0, dark);
    painter.rect_filled(fit.rect(rect), 0.0, colour(palette::ORANGE));
    painter.rect_filled(fit.rect(rect.grow(-layout::FRAME)), 0.0, dark);
    painter.rect_filled(
        fit.rect(rect.grow(-(layout::FRAME + layout::LINE))),
        0.0,
        colour(palette::CREAM),
    );
    let overhang = layout::CAP_OVERHANG;
    let corners = [
        (rect.x - overhang, rect.y - overhang),
        (rect.right() - layout::CAP + overhang, rect.y - overhang),
        (rect.x - overhang, rect.bottom() - layout::CAP + overhang),
        (rect.right() - layout::CAP + overhang, rect.bottom() - layout::CAP + overhang),
    ];
    for (x, y) in corners {
        let cap = Rect::new(x, y, layout::CAP, layout::CAP);
        painter.rect_filled(fit.rect(cap), 0.0, dark);
        painter.rect_filled(fit.rect(cap.grow(-layout::LINE)), 0.0, colour(palette::CAP));
    }
}

/// A square portrait frame: white line, black border, and the picture of the object at the largest
/// whole scale that fits (or the empty box with its label).
fn portrait(
    ui: &egui::Ui,
    painter: &egui::Painter,
    fit: &Fit,
    art: &mut Art,
    rect: Rect,
    picture: Option<Picture>,
) {
    painter.rect_filled(fit.rect(rect.grow(2.0)), 0.0, colour(palette::WHITE));
    painter.rect_filled(fit.rect(rect), 0.0, colour(palette::BLACK));
    let inner = rect.grow(-layout::PORTRAIT_FRAME);
    let texture = picture.and_then(|picture| art.texture(ui.ctx(), picture));
    // Nothing to show (an empty list, or a face with no picture): the box stays empty, never a
    // stand-in text
    if let Some(id) = texture {
        let whole = (inner.w / layout::PICTURE).floor().max(1.0);
        let side = layout::PICTURE * whole;
        let picture = Rect::new(
            inner.x + (inner.w - side) / 2.0,
            inner.y + (inner.h - side) / 2.0,
            side,
            side,
        );
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(id, fit.rect(picture), uv, egui::Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_menus::Style;

    #[test]
    fn a_run_that_does_not_persist_writes_no_sidecar_and_leaves_a_damaged_one_as_it_is() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let bag = sidecar_path();
        paths::set_persistent(false);
        write_beside(&bag, b"bags").unwrap();
        assert!(!bag.exists() && !paths::saves_dir().exists(), "no sidecar, no saves folder");
        // A damaged sidecar is read as none, and the file stays where it is.
        std::fs::create_dir_all(paths::saves_dir()).unwrap();
        std::fs::write(&bag, b"garbage").unwrap();
        assert!(read_sidecar().is_ok());
        assert_eq!(std::fs::read(&bag).unwrap(), b"garbage", "the damaged file is unchanged");
        assert!(!bag.with_extension("bag.bad").exists());
        // A windowed run writes the sidecar, and keeps a damaged one aside.
        paths::set_persistent(true);
        write_beside(&bag, b"bags").unwrap();
        assert_eq!(std::fs::read(&bag).unwrap(), b"bags");
        std::fs::write(&bag, b"garbage").unwrap();
        read_sidecar().unwrap();
        assert!(bag.with_extension("bag.bad").exists(), "the damaged file was moved aside");
        assert!(!bag.exists());
        paths::set_persistent(false);
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn the_colours_widen_five_bits_to_eight_by_repeating_their_top_bits() {
        assert_eq!(snes_rgb(0x0000), [0, 0, 0]);
        assert_eq!(snes_rgb(0x7FFF), [255, 255, 255]);
        // 26 (0b11010) widens to 0b11010110 = 214, as the console shows it.
        assert_eq!(snes_rgb(26 << 5), [0, 214, 0]);
        assert_eq!(snes_rgb(0x001F), [255, 0, 0]);
    }

    #[test]
    fn index_zero_is_transparent_and_the_others_take_the_palette() {
        let mut palette = [0_u16; 16];
        palette[1] = 0x001F;
        let portrait = Portrait { half: 0, characters: Vec::new(), pixels: vec![0, 1], palette };
        let pixels = rgba(&portrait);
        assert_eq!(pixels, [0, 0, 0, 0, 255, 0, 0, 255]);
    }

    #[test]
    fn the_scene_fits_the_area_and_keeps_its_proportions() {
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1440.0, 900.0));
        let fit = Fit::new(area);
        assert!((fit.scale - 1.125).abs() < 1e-6, "width is the limit: 1440 / 1280");
        assert_eq!(fit.point(0.0, 0.0), egui::pos2(0.0, 45.0), "centred vertically");
        let square = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 720.0));
        assert!((Fit::new(square).scale - 0.5).abs() < 1e-6);
    }

    /// The egui context the menus' text is measured with: the theme's fonts, laid out once. The
    /// first pass uploads the font atlas, which nothing paints here, so its delta is cleared (epaint
    /// asserts in debug builds that a delta is handled before it is dropped).
    fn measuring_context() -> egui::Context {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        ctx
    }

    /// The boxes, in points, of what `list` paints over `fit`'s picture: a window with its frame and
    /// its corner caps, a portrait with its white line, the bars, the cursors and arrows, the text
    /// runs (measured in the font they are painted in). The backdrop is the picture itself: left out.
    fn painted_boxes(ctx: &egui::Context, fit: &Fit, list: &DrawList) -> Vec<egui::Rect> {
        let logical = |rect: Rect| fit.rect(rect);
        let mut boxes = Vec::new();
        for op in list.ops() {
            match op {
                Op::Backdrop => {}
                Op::Fill { rect, .. }
                | Op::Disc { rect, .. }
                | Op::Outline { rect, .. }
                | Op::Bar { rect, .. } => {
                    boxes.push(logical(*rect));
                }
                Op::Window { rect } | Op::WindowHc { rect } => {
                    boxes.push(logical(rect.grow(layout::CAP_OVERHANG.max(layout::LINE))));
                }
                Op::Portrait { rect, .. } => boxes.push(logical(rect.grow(2.0))),
                Op::Card { rect, .. } => boxes.push(logical(*rect)),
                Op::Pointer { rect, .. } => boxes.push(logical(rect.grow(POINTER_EDGE / 2.0))),
                Op::Cursor { x, y } | Op::Arrow { x, y, .. } => {
                    boxes.push(logical(Rect::new(
                        *x,
                        *y,
                        layout::CURSOR_WIDTH,
                        layout::CURSOR_HEIGHT,
                    )));
                }
                Op::Line { from, to, width, .. } => {
                    let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
                    let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
                    boxes.push(logical(Rect::new(x0, y0, x1 - x0, y1 - y0).grow(width / 2.0)));
                }
                Op::Text { x, y, text, size, align, .. } => {
                    boxes.push(text_box(ctx, fit, (*x, *y), text, *size, *align));
                }
            }
        }
        boxes
    }

    /// The box, in points, of a text run of `text` at `size` logical pixels, anchored at `at` in logical pixels,
    /// measured in the font it is painted in.
    fn text_box(
        ctx: &egui::Context,
        fit: &Fit,
        at: (f32, f32),
        text: &str,
        size: f32,
        align: ssd_menus::Align,
    ) -> egui::Rect {
        let galley = ctx.fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                text.to_owned(),
                theme::game(size * fit.scale),
                egui::Color32::WHITE,
            )
        });
        let anchor = match align {
            ssd_menus::Align::Left => egui::Align2::LEFT_CENTER,
            ssd_menus::Align::Centre => egui::Align2::CENTER_CENTER,
            ssd_menus::Align::Right => egui::Align2::RIGHT_CENTER,
        };
        anchor.anchor_size(fit.point(at.0, at.1), galley.size())
    }

    /// The screens of the menus in their worst cases: every command and row, the longest names, a
    /// party of nine (the most the records hold), a full bag of 24 stacks, and each sheet.
    fn every_screen() -> Vec<DrawList> {
        let plain = plain_screens();
        let styles = [
            Style { large_text: true, ..Style::default() },
            Style { contrast: true, safe_cursor: true, ..Style::default() },
            Style { large_text: true, contrast: true, safe_cursor: true },
        ];
        let mut screens = plain.clone();
        for style in styles {
            screens.extend(plain.iter().map(|list| style.apply(list.clone())));
        }
        screens
    }

    /// The screens in the design as drawn: see [`every_screen`].
    fn plain_screens() -> Vec<DrawList> {
        use ssd_menus::cheats::{Cheats, Row, Setting, ZMode, ZTarget};
        use ssd_menus::view::{
            BagView, Member, MenuView, Screen, Selected, Sheet, Slot, StackView, Tab,
        };
        let member = |name: &str, face| Member {
            name: name.into(),
            level: 99,
            hp: 999,
            max_hp: 999,
            ki: 999,
            max_ki: 999,
            bp: 16_777_215,
            face,
        };
        let party: Vec<Member> =
            (0..9).map(|i| member(&format!("Tien Shinhan {i}"), Some(28))).collect();
        let base = MenuView {
            screen: Screen::Command,
            bag: None,
            town: false,
            flying: false,
            cursor: 0,
            page: 0,
            zeni: 999,
            party: party.clone(),
            items: Vec::new(),
            selected: None,
        };
        let mut screens = Vec::new();
        for cursor in 0..5 {
            screens.push(ssd_menus::screens::command(&MenuView { cursor, ..base.clone() }));
        }
        for cursor in 0..4 {
            screens.push(ssd_menus::screens::command(&MenuView {
                town: true,
                cursor,
                ..base.clone()
            }));
        }
        for cursor in 0..3 {
            screens.push(ssd_menus::screens::command(&MenuView {
                flying: true,
                cursor,
                ..base.clone()
            }));
        }
        for cursor in 0..7 {
            screens.push(ssd_menus::screens::scene(&MenuView {
                screen: Screen::Menu,
                cursor,
                ..base.clone()
            }));
        }
        let items: Vec<Slot> = (0..24)
            .map(|i| Slot { object: i, attribute: 0xC0, name: "Dragon Radar Plus".into() })
            .collect();
        let stacks: Vec<StackView> = (0..24)
            .map(|i| StackView {
                id: i,
                object: i as u8,
                name: "Dragon Radar Plus".into(),
                count: 99,
                usable: i % 2 == 0,
                droppable: true,
                ally: i % 3 == 0,
            })
            .collect();
        let selected = Selected {
            object: 2,
            name: "Dragon Radar Plus".into(),
            price: Some(255),
            help: vec!["Restores 25".into(), "HP and KI at once.".into(), "A third line.".into()],
            battle: true,
            field: true,
        };
        let bag = |sheet, notice| BagView {
            tab: Tab::All,
            stacks: stacks.clone(),
            used: 24,
            slots: 60,
            full: true,
            usable_here: true,
            droppable_here: true,
            sheet,
            choice: 1,
            notice,
        };
        for sheet in [Sheet::Browse, Sheet::Choose, Sheet::Confirm] {
            for page in 0..3 {
                for cursor in [0, 7] {
                    let view = MenuView {
                        screen: Screen::Items,
                        bag: Some(bag(sheet, Some("Bag full: nothing more can be taken."))),
                        items: items.clone(),
                        page,
                        cursor,
                        selected: Some(selected.clone()),
                        ..base.clone()
                    };
                    screens.push(ssd_menus::screens::items(&view));
                }
            }
        }
        screens.push(ssd_menus::screens::items(&MenuView {
            screen: Screen::Items,
            items,
            selected: None,
            ..base
        }));
        // The cheats screen: each row under the cursor, with nothing on and with every setting on (the longest
        // On list, the longest names), and the notice of a scripted fight.
        let names: Vec<String> = (0..12).map(|id| format!("Tsumuri Maima {id}")).collect();
        let mut busy = Cheats::neutral();
        for setting in Setting::ALL {
            busy.set_index(setting, setting.steps().len() - 1);
        }
        busy.set_random_battles_off(true);
        busy.set_z_mode(ZMode::Both);
        busy.set_z_target(ZTarget::Fighter(11));
        busy.set_zeni(true);
        busy.set_endless(true);
        for row in Row::ALL {
            for cheats in [Cheats::neutral(), busy] {
                let view = ssd_menus::view::CheatsView { cheats, row, names: names.clone() };
                screens.push(ssd_menus::screens::cheats_screen(&view));
            }
        }
        // The Options screen: each page with the cursor on each of its rows (the two questions lie over the page they ask about, which
        // the text check cannot take).
        for category in ssd_menus::options::Category::ALL {
            for row in 0..category.rows().len().max(1) {
                let view = ssd_menus::view::OptionsView {
                    category,
                    row,
                    values: vec![1; category.rows().len()],
                    confirm: None,
                    video: None,
                    controls: None,
                };
                screens.push(ssd_menus::screens::options_screen(&view));
            }
        }
        screens.push(ssd_menus::screens::notice("Cheats off for this fight"));
        for which in [SlotList::Continue, SlotList::Save] {
            screens.push(ssd_menus::screens::slot_badges(which, [true; 3]));
        }
        screens
    }

    #[test]
    fn every_element_of_every_hd_screen_lies_inside_the_game_picture() {
        let ctx = measuring_context();
        let screens = every_screen();
        // A picture of the Classic frame (256 x 224 at 8:7), a square one, a widescreen one, at the
        // sizes the window takes with the panel open and closed, and at display scales of 1.25 and 1.5.
        let areas = [
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1040.0, 800.0)),
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 720.0)),
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(640.0, 480.0)),
            egui::Rect::from_min_size(egui::pos2(24.0, 40.0), egui::vec2(2560.0, 1360.0)),
        ];
        // ... the Classic frame (256 x 239 at 8:7), and the 16:9 frame of the widescreen view
        // (372 x 239 at 8:7), both in the live picture's rect.
        let frames = [
            (256, 224, 8.0 / 7.0),
            (256, 239, 8.0 / 7.0),
            (372, 239, 8.0 / 7.0),
            (256, 224, 1.0),
            (320, 180, 1.0),
        ];
        for ppp in [1.0, 1.25, 1.5] {
            for area in areas {
                for ((width, height, aspect), whole) in
                    frames.into_iter().flat_map(|frame| [(frame, true), (frame, false)])
                {
                    let picture =
                        crate::layout::picture_rect(ppp, area, width, height, aspect, whole);
                    assert!(area.contains_rect(picture), "{picture:?} in {area:?}");
                    let fit = Fit::new(picture);
                    let canvas = fit.rect(Rect::new(0.0, 0.0, layout::WIDTH, layout::HEIGHT));
                    assert!(picture.expand(0.01).contains_rect(canvas), "the logical space fits");
                    for (index, list) in screens.iter().enumerate() {
                        for boxed in painted_boxes(&ctx, &fit, list) {
                            assert!(
                                picture.expand(0.01).contains_rect(boxed),
                                "screen {index} at {ppp} ppp, {width}x{height} in {area:?}: \
                                 {boxed:?} is outside the picture {picture:?}"
                            );
                            assert!(
                                canvas.expand(0.01).contains_rect(boxed),
                                "screen {index}: {boxed:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn no_text_leaves_its_window_and_no_two_runs_share_their_capitals() {
        let ctx = measuring_context();
        let screens = every_screen();
        // The logical space at one point a pixel, so the sizes are the design's.
        let fit = Fit::new(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(layout::WIDTH, layout::HEIGHT),
        ));
        for (index, list) in screens.iter().enumerate() {
            let windows: Vec<Rect> = list
                .ops()
                .iter()
                .filter_map(|op| match op {
                    Op::Window { rect } | Op::WindowHc { rect } => Some(*rect),
                    _ => None,
                })
                .collect();
            // Each run with its capitals' band: the cap height centred on the run's box, which two runs must not share.
            let mut runs: Vec<(String, egui::Rect)> = Vec::new();
            for op in list.ops() {
                let Op::Text { x, y, text, size, align, .. } = op else { continue };
                let boxed = text_box(&ctx, &fit, (*x, *y), text, *size, *align);
                // The innermost window the run starts in: the run lies inside its cream paper.
                let innermost = windows
                    .iter()
                    .filter(|w| w.x <= *x && *x <= w.right() && w.y <= *y && *y <= w.bottom())
                    .min_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)));
                if let Some(window) = innermost {
                    let paper = fit.rect(window.grow(-(layout::FRAME + layout::LINE)));
                    assert!(
                        paper.expand(0.01).contains_rect(boxed),
                        "screen {index}: {text:?} at {boxed:?} leaves its window's paper {paper:?}"
                    );
                }
                let cap = 0.7 * size * fit.scale;
                let band =
                    egui::Rect::from_center_size(boxed.center(), egui::vec2(boxed.width(), cap));
                runs.push((text.clone(), band));
            }
            for (i, (a_text, a_band)) in runs.iter().enumerate() {
                for (b_text, b_band) in &runs[i + 1..] {
                    // A shadow is the same text drawn twice, so two equal runs may share their place.
                    if a_text == b_text {
                        continue;
                    }
                    let shared = a_band.intersect(*b_band);
                    assert!(
                        shared.width() <= 0.5 || shared.height() <= 0.5,
                        "screen {index}: {a_text:?} at {a_band:?} and {b_text:?} at {b_band:?} share {shared:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_scripted_fight_shows_the_off_notice_and_the_map_the_on_notice_for_two_seconds() {
        let mut wram = vec![0_u8; 0x2_0000];
        let mut notice = CheatNotice::default();
        notice.note(&wram);
        assert_eq!(notice.text(), None, "no cheat block: no notice");
        wram[GATE] = 1;
        notice.note(&wram);
        assert_eq!(
            notice.text(),
            None,
            "a gate byte without the block's marker is the power-on fill"
        );
        wram[BLOCK_MARKER..BLOCK_MARKER + MARKER_BYTES.len()].copy_from_slice(&MARKER_BYTES);
        notice.note(&wram);
        assert_eq!(notice.text(), Some(OFF_NOTICE), "the fight suspends the cheats");
        for _ in 1..NOTICE_FRAMES - 1 {
            notice.note(&wram);
        }
        assert_eq!(notice.text(), Some(OFF_NOTICE), "it shows for two seconds");
        notice.note(&wram);
        assert_eq!(notice.text(), None, "then it goes");
        for _ in 0..NOTICE_FRAMES {
            notice.note(&wram);
        }
        assert_eq!(notice.text(), None, "a gate that stays up shows its notice once");
        wram[GATE] = 0;
        notice.note(&wram);
        assert_eq!(notice.text(), Some(ON_NOTICE), "the map brings the cheats back");
    }

    #[test]
    fn the_original_slot_lists_are_found_from_the_game_mode_and_the_menu_step() {
        let mut wram = vec![0_u8; 0x2_0000];
        assert_eq!(slot_list(&wram), None);
        wram[0x24] = TITLE_SLOTS_MODE;
        assert_eq!(slot_list(&wram), Some(SlotList::Continue), "the title's Which slot?");
        wram[0x24] = 0x87;
        assert_eq!(slot_list(&wram), None, "the title's menu is not a slot list");
        let screen = (d::MENU_SCREEN - 0x7E_0000) as usize;
        let step = (d::MENU_STEP - 0x7E_0000) as usize;
        wram[screen] = SAVE_SCREEN;
        assert_eq!(slot_list(&wram), None, "the Save's step 0 is its question");
        wram[step] = SAVE_LIST_STEP;
        assert_eq!(slot_list(&wram), Some(SlotList::Save), "the Save's Which slot?");
        wram[screen] = 4;
        assert_eq!(slot_list(&wram), None, "the Menu list is no slot list");
    }

    #[test]
    fn the_font_of_the_menus_is_registered_in_the_theme() {
        let _ = theme::game(32.0);
    }

    #[test]
    fn a_button_is_a_key_when_it_goes_down_and_the_menu_reads_the_d_pad_and_a_b() {
        let none = Joypad::default();
        let down = Joypad { down: true, ..Joypad::default() };
        let down_and_a = Joypad { down: true, a: true, ..Joypad::default() };
        assert_eq!(keys_pressed(none, down), [Key::Down]);
        assert_eq!(keys_pressed(down, down), [], "held, not pressed again");
        assert_eq!(keys_pressed(down, down_and_a), [Key::Confirm]);
        let b = Joypad { b: true, ..Joypad::default() };
        assert_eq!(keys_pressed(none, b), [Key::Cancel]);
        assert_eq!(keys_pressed(none, Joypad { start: true, ..Joypad::default() }), []);
        let l = Joypad { l: true, ..Joypad::default() };
        let r = Joypad { r: true, ..Joypad::default() };
        assert_eq!(
            (keys_pressed(none, l), keys_pressed(none, r)),
            (vec![Key::TabBack], vec![Key::Tab])
        );
        let y = Joypad { y: true, ..Joypad::default() };
        let select = Joypad { select: true, ..Joypad::default() };
        assert_eq!(keys_pressed(none, y), [Key::ResetPage]);
        assert_eq!(keys_pressed(none, select), [Key::ResetAll]);
    }
}
