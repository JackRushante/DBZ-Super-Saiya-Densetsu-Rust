//! The HD art of a run: the pack the converter and `hd-import` left for the ROM, found once and
//! shared by the Enhanced picture and the HD menus. `--hd-pack` names another pack folder and
//! `--no-hd-art` turns the art off; without art the Enhanced picture is the look alone. The HD text
//! font (the game's UI text redrawn from Share Tech Mono at the picture's scale) is found here
//! too; `--no-hd-font` turns it off.

use ssd_hd::Font;
use ssd_hd::pack::{Overlay, Pack};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// What the command line said about the art.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Use this pack folder instead of the newest one under the cache directory.
    pub pack: Option<PathBuf>,
    /// No art at all.
    pub off: bool,
    /// No HD text: the game's own letters, made bigger by the look.
    pub font_off: bool,
    /// The HD art of the data packs of the run (`--mods`), drawn over the installed pack's.
    pub overlays: Vec<Overlay>,
    /// The image the data packs run on (`--mods`): the field's maps of the packs are in its banks.
    pub image: Option<Vec<u8>>,
}

static CONFIG: Mutex<Config> = Mutex::new(Config {
    pack: None,
    off: false,
    font_off: false,
    overlays: Vec::new(),
    image: None,
});

/// The bytes the field of the Enhanced picture is read from: the image the data packs run on, or else the ROM.
pub fn world(rom: &[u8]) -> Vec<u8> {
    CONFIG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .image
        .clone()
        .unwrap_or_else(|| rom.to_vec())
}
// Found on first use, and forgotten by [`configure`]: Android runs the game more than once in a process (the setup can
// install a new pack between two runs).
static PACK: Mutex<Option<Found<Pack>>> = Mutex::new(None);
static FONT: Mutex<Option<Found<Font>>> = Mutex::new(None);

/// What a search found: a value or nothing, kept either way.
#[derive(Clone)]
struct Found<T>(Option<Arc<T>>);

/// Sets what the command line said (before the first [`pack`]).
pub fn configure(config: Config) {
    *CONFIG.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = config;
    *PACK.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    *FONT.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// The pack with the HD art for `rom`, found on first use; `None` when art is off, there is no
/// pack for the ROM, or the pack holds no art.
pub fn pack(rom: &[u8]) -> Option<Arc<Pack>> {
    cached(&PACK, || {
        let config = CONFIG.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        if config.off {
            return None;
        }
        let found = match &config.pack {
            Some(dir) => match Pack::open(dir) {
                Ok(pack) => Some(pack),
                Err(error) => {
                    log::warn!("the HD pack {}: {error}", dir.display());
                    None
                }
            },
            None => Pack::find(&crate::paths::data_dir().join("cache"), ssd_rom::crc32(rom)),
        };
        let found = found.map(|pack| {
            config.overlays.iter().fold(pack, |pack, overlay| pack.with_overlay(overlay.clone()))
        });
        let pack = found
            .filter(|pack| !pack.art.pictures().is_empty() || !pack.art.objects().is_empty())?;
        log::info!(
            "HD art: {} pictures, {} poses from {}",
            pack.art.pictures().len(),
            pack.art.objects().len(),
            pack.dir.display()
        );
        Some(Arc::new(pack))
    })
}

/// The HD text font for `rom`, made on first use; `None` when it is off or the image has no UI font.
pub fn font(rom: &[u8]) -> Option<Arc<Font>> {
    cached(&FONT, || {
        if CONFIG.lock().unwrap_or_else(std::sync::PoisonError::into_inner).font_off {
            return None;
        }
        // The card art lives in the pack: with no art (or none for this ROM) the cards stay as drawn
        let pack = pack(rom);
        match Font::standard_with_pack(rom, pack.as_ref().map(|pack| pack.dir.as_path())) {
            Ok(font) => Some(Arc::new(font)),
            Err(error) => {
                log::warn!("the HD text font: {error}");
                None
            }
        }
    })
}

/// The value in `slot`, made by `make` on first use.
fn cached<T>(
    slot: &Mutex<Option<Found<T>>>,
    make: impl FnOnce() -> Option<Arc<T>>,
) -> Option<Arc<T>> {
    let mut guard = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.get_or_insert_with(|| Found(make())).0.clone()
}
