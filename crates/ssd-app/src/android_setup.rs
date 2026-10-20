//! The setup on Android: the ROM and the HD art pack come from the system file picker (Storage Access Framework),
//! through SDL's file dialog. SDL returns a `content://` address; SDL's I/O streams read it, so a large pack is
//! copied in blocks and never held in memory. The screens (`ssd_menus::setup`) are drawn with the HD menus' own
//! drawing in a window of their own; the file picker itself is Android's. The work (copy, check, install) runs on
//! a thread, and the screen shows its steps.
//!
//! The same flow is reached later: the Options' Game page (Enhanced) and Select + Start held in Classic end the
//! game and ask for a page (`request`), and the launch loop runs the setup again.
#![cfg(target_os = "android")]
#![allow(unsafe_code)]

use crate::egui_input::InputAdapter;
use crate::hd::Art;
use crate::video::Video;
use crate::{paths, rom_setup, setup_ui};
use sdl3::event::{Event, WindowEvent};
use sdl3::gamepad::Button;
use sdl3::keyboard::Scancode;
use ssd_hdpack::install::{self, Outcome};
use ssd_menus::Key;
use ssd_menus::setup::{Command, Page, Setup, Status};
use std::ffi::{CStr, CString, OsString, c_char, c_int, c_void};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// A ROM image is a few megabytes; a bigger file is not one, and is refused before it is copied.
const MAX_ROM_BYTES: i64 = 16 << 20;
/// An IPS patch is small.
const MAX_PATCH_BYTES: i64 = 4 << 20;

/// What the game asked the setup for when it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forced {
    /// The mode choice (Options > Game > Change mode; Select + Start held in Classic).
    Mode,
    /// The HD pack install (Options > Game > Install the HD pack).
    Pack,
    /// No page: the game starts again (the touch MENU's GAME page: Restart).
    Relaunch,
}

static REQUEST: Mutex<Option<Forced>> = Mutex::new(None);

/// Asks for the setup: the game ends, and the launch loop runs the page asked for.
pub fn request(forced: Forced) {
    *REQUEST.lock().unwrap_or_else(PoisonError::into_inner) = Some(forced);
}

/// The request made while the game ran, once.
pub fn take_request() -> Option<Forced> {
    REQUEST.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// The two lines of the Options' Game page: the mode, and the HD pack's state (its revision and size).
pub fn game_info(dir: &Path) -> (String, String) {
    let mode = if read_mode(dir) == Some(true) { "Enhanced" } else { "Classic" };
    let pack = match rom_setup::resolve_rom(None, None, false) {
        Ok(rom_setup::RomSetup::Ready(rom)) => {
            ssd_hd::pack::Pack::find(&paths::data_dir().join("cache"), ssd_rom::crc32(&rom))
                .map_or_else(
                    || "Not installed".to_owned(),
                    |pack| {
                        let size = folder_size(&pack.dir) as f64 / (1u64 << 30) as f64;
                        match pack_revision(&pack.dir) {
                            Some(revision) => format!("Revision {revision}, {size:.1} GB"),
                            None => format!("Installed, {size:.1} GB"),
                        }
                    },
                )
        }
        _ => "Not installed".to_owned(),
    };
    (mode.to_owned(), pack)
}

/// The revision of the pack's art, from the file the installer left in the pack's folder.
fn pack_revision(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(install::INSTALLED)).ok()?;
    let line = text.lines().find(|line| line.trim_start().starts_with("revision"))?;
    let value = line.split_once('=')?.1.trim().trim_matches('"');
    (!value.is_empty()).then(|| value.to_owned())
}

fn folder_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => folder_size(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

fn mode_file(dir: &Path) -> PathBuf {
    dir.join("android-mode.txt")
}

/// The saved mode: `Some(true)` for Enhanced, `Some(false)` for Classic, `None` when none was chosen yet.
fn read_mode(dir: &Path) -> Option<bool> {
    match std::fs::read_to_string(mode_file(dir)).as_deref().map(str::trim) {
        Ok("enhanced") => Some(true),
        Ok("classic") => Some(false),
        _ => None,
    }
}

fn has(args: &[OsString], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn has_value(args: &[OsString], flag: &str, value: &str) -> bool {
    args.windows(2).any(|pair| pair[0] == flag && pair[1] == value)
}

fn pack_installed(rom: &[u8]) -> bool {
    ssd_hd::pack::Pack::find(&paths::data_dir().join("cache"), ssd_rom::crc32(rom)).is_some()
}

/// Where the flow stands: what is known, and so which page comes next.
struct Flow {
    rom: Option<Vec<u8>>,
    /// The mode: `Some(true)` Enhanced.
    mode: Option<bool>,
    /// The mode came from the arguments: it is not asked.
    mode_fixed: bool,
    /// The launch arguments allow the HD art.
    wants_art: bool,
    forced: Option<Forced>,
    mode_done: bool,
    pack_done: bool,
}

impl Flow {
    /// The page that is still to be shown; `None` when the setup is complete.
    fn next_page(&self) -> Option<Page> {
        let Some(rom) = self.rom.as_deref() else { return Some(Page::Rom) };
        if !self.mode_fixed && (self.mode.is_none() || (self.forced == Some(Forced::Mode) && !self.mode_done)) {
            return Some(Page::Mode);
        }
        let wants_pack = self.wants_art
            && ((self.mode == Some(true) && !pack_installed(rom))
                || (self.forced == Some(Forced::Pack) && !self.pack_done));
        wants_pack.then_some(Page::Pack)
    }
}

/// Runs the setup the launch still needs (or `forced` asks for), adding arguments for it. `false` means the
/// player quit.
pub fn run(args: &mut Vec<OsString>, dir: &Path, forced: Option<Forced>) -> bool {
    let mut note = None;
    let rom = match rom_setup::resolve_rom(None, None, false) {
        Ok(rom_setup::RomSetup::Ready(bytes)) => Some(bytes),
        Ok(rom_setup::RomSetup::Missing) => None,
        Err(error) => {
            note = Some(format!("The saved ROM cannot be used ({error:#}). Choose it again."));
            None
        }
    };
    let mode_fixed = has(args, "--mode");
    let mode = if mode_fixed { Some(has_value(args, "--mode", "enhanced")) } else { read_mode(dir) };
    let wants_art = !["--no-hd-art", "--hd-pack", "--install-hd-pack"].iter().any(|flag| has(args, flag));
    let mut flow = Flow { rom, mode, mode_fixed, wants_art, forced, mode_done: false, pack_done: false };
    let Some(first) = flow.next_page() else {
        finish(args, &flow);
        return true;
    };
    match screens(&mut flow, dir, first, note) {
        Ok(true) => {
            finish(args, &flow);
            true
        }
        Ok(false) => false,
        Err(error) => {
            log::error!("the setup screens failed: {error:#}");
            false
        }
    }
}

/// The launch arguments the setup decided.
fn finish(args: &mut Vec<OsString>, flow: &Flow) {
    if !flow.mode_fixed && flow.mode == Some(true) {
        args.push(OsString::from("--mode"));
        args.push(OsString::from("enhanced"));
    }
    let enhanced = flow.mode == Some(true) || has_value(args, "--mode", "enhanced");
    let art_missing = flow.rom.as_deref().is_some_and(|rom| !pack_installed(rom));
    if enhanced && flow.wants_art && art_missing {
        args.push(OsString::from("--no-hd-art"));
    }
}

// ---- the work, off the screen's thread ----

enum Msg {
    /// A step of the work.
    Status(Status),
    /// The ROM file was read: accepted, or why not.
    Rom(Result<Vec<u8>, RomProblem>),
    /// The pack was installed, or why not.
    Pack(Result<(), String>),
}

enum RomProblem {
    /// A Japanese Rev 1 image: it needs the patch (the copy stays in `rom-import.tmp`).
    NeedsPatch,
    Refused(String),
}

/// The pick in progress.
struct Pick {
    receiver: Receiver<Option<String>>,
}

/// The screen loop. `Ok(true)`: the setup is complete; `Ok(false)`: the player quit.
fn screens(flow: &mut Flow, dir: &Path, first: Page, note: Option<String>) -> anyhow::Result<bool> {
    // The game turns SDL's touch-to-mouse events off (the touch controls read fingers); the setup screens are
    // egui, which reads the mouse, so a tap is a click here.
    sdl3::hint::set("SDL_TOUCH_MOUSE_EVENTS", "1");
    let sdl = sdl3::init().map_err(|e| anyhow::anyhow!("{e}"))?;
    let video_subsystem = sdl.video().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut video = Video::new(&video_subsystem, "Dragon Ball Z: Super Saiya Densetsu")?;
    let mut events = sdl.event_pump().map_err(|e| anyhow::anyhow!("{e}"))?;
    let gamepads = sdl.gamepad().ok();
    let mut pads = Vec::new();
    let mut input = InputAdapter::default();
    let mut art = Art::empty();
    let mut setup = Setup::new(first);
    if let Some(note) = note {
        setup.set_status(Status::Error(note));
    }
    let (sender, messages) = mpsc::channel::<Msg>();
    let mut pick: Option<(Pick, Page)> = None;
    let mut backgrounded = false;
    let mut starting = false;
    loop {
        let mut keys = Vec::new();
        for event in events.poll_iter() {
            video.handle_event(&mut input, &event);
            match event {
                Event::Quit { .. } => return Ok(false),
                Event::Window { win_event: WindowEvent::FocusLost, .. }
                | Event::AppDidEnterBackground { .. } => {
                    if !backgrounded {
                        backgrounded = true;
                        video.suspend();
                    }
                }
                Event::Window { win_event: WindowEvent::FocusGained, .. }
                | Event::AppDidEnterForeground { .. } => {
                    if backgrounded {
                        backgrounded = false;
                        video.resume();
                    }
                }
                Event::Window { win_event: WindowEvent::Resized(..) | WindowEvent::PixelSizeChanged(..), .. } => {
                    video.handle_resize();
                }
                Event::KeyDown { scancode: Some(code), repeat: false, .. } => {
                    keys.extend(match code {
                        Scancode::Up => Some(Key::Up),
                        Scancode::Down => Some(Key::Down),
                        Scancode::Left => Some(Key::Left),
                        Scancode::Right => Some(Key::Right),
                        Scancode::Return | Scancode::KpEnter | Scancode::Space => Some(Key::Confirm),
                        Scancode::Escape | Scancode::AcBack | Scancode::Backspace => Some(Key::Cancel),
                        Scancode::Tab => Some(Key::Tab),
                        _ => None,
                    });
                }
                Event::GamepadButtonDown { button, .. } => {
                    keys.extend(match button {
                        Button::DPadUp => Some(Key::Up),
                        Button::DPadDown => Some(Key::Down),
                        Button::DPadLeft => Some(Key::Left),
                        Button::DPadRight => Some(Key::Right),
                        Button::South => Some(Key::Confirm),
                        Button::East => Some(Key::Cancel),
                        _ => None,
                    });
                }
                Event::GamepadAdded { which, .. } => {
                    if let Some(subsystem) = &gamepads {
                        if let Ok(pad) = subsystem.open(which) {
                            pads.push(pad);
                        }
                    }
                }
                _ => {}
            }
        }
        if backgrounded {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        }

        let mut commands: Vec<Command> = keys.into_iter().filter_map(|key| setup.key(key)).collect();
        // The work's messages and the picker's answer.
        while let Ok(message) = messages.try_recv() {
            match message {
                Msg::Status(status) => setup.set_status(status),
                Msg::Rom(Ok(bytes)) => {
                    flow.rom = Some(bytes);
                    setup.set_status(Status::Done("ROM accepted.".to_owned()));
                }
                Msg::Rom(Err(RomProblem::NeedsPatch)) => setup.go(Page::Patch),
                Msg::Rom(Err(RomProblem::Refused(reason))) => {
                    setup.set_status(Status::Error(format!("This file was not accepted: {reason}")));
                }
                Msg::Pack(Ok(())) => {
                    flow.pack_done = true;
                    setup.set_status(Status::Done("Installed.".to_owned()));
                }
                Msg::Pack(Err(reason)) => {
                    setup.set_status(Status::Error(format!("The HD pack was not installed: {reason}")));
                }
            }
        }
        if let Some((waiting, page)) = &pick {
            match waiting.receiver.try_recv() {
                Ok(Some(uri)) => {
                    let page = *page;
                    pick = None;
                    log::info!("picker returned a file for {page:?}");
                    start_work(page, uri, dir, &sender, &mut setup);
                }
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    log::info!("the picker was cancelled");
                    pick = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }

        let shown = setup.clone();
        let mut tapped = None;
        video.present(&[], &mut input, |ui, _| {
            tapped = setup_ui::show(ui, &shown, &mut art);
        })?;
        commands.extend(tapped);

        for command in commands {
            match command {
                Command::Pick if pick.is_none() => {
                    pick = Some((Pick { receiver: pick_file() }, setup.page()));
                }
                Command::Pick => {}
                Command::Back => {
                    let _ = std::fs::remove_file(dir.join("rom-import.tmp"));
                    setup.go(Page::Rom);
                }
                Command::Mode { enhanced } => {
                    let _ = std::fs::write(mode_file(dir), if enhanced { "enhanced" } else { "classic" });
                    flow.mode = Some(enhanced);
                    flow.mode_done = true;
                    match flow.next_page() {
                        Some(page) => setup.go(page),
                        None => starting = true,
                    }
                }
                Command::Skip => {
                    // Without the pack, Enhanced has no art: Classic it is, and Options (Classic: Select + Start) can
                    // bring the choice back. A pack that is already installed stays as it is.
                    if flow.rom.as_deref().is_some_and(|rom| !pack_installed(rom)) {
                        let _ = std::fs::write(mode_file(dir), "classic");
                        flow.mode = Some(false);
                    }
                    flow.pack_done = true;
                    starting = flow.next_page().is_none();
                }
                Command::Continue => {
                    // The ROM was accepted (or the pack installed): on to what is still to do.
                    if setup.page() == Page::Pack {
                        starting = true;
                    } else {
                        match flow.next_page() {
                            Some(page) => setup.go(page),
                            None => starting = true,
                        }
                    }
                }
            }
        }
        if starting {
            // One last frame, so that the window does not go blank while the game loads.
            setup.go(Page::Pack);
            setup.set_status(Status::Busy { text: "Starting...".to_owned(), percent: Some(100) });
            let shown = setup.clone();
            video.present(&[], &mut input, |ui, _| {
                setup_ui::show(ui, &shown, &mut art);
            })?;
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Starts the work for the file the picker returned.
fn start_work(
    page: Page,
    uri: String,
    dir: &Path,
    sender: &Sender<Msg>,
    setup: &mut Setup,
) {
    let sender = sender.clone();
    let dir = dir.to_path_buf();
    match page {
        Page::Rom => {
            setup.set_status(Status::Busy { text: "Reading the file...".to_owned(), percent: None });
            std::thread::spawn(move || {
                let _ = sender.send(Msg::Rom(check_rom(&uri, &dir)));
            });
        }
        Page::Patch => {
            setup.set_status(Status::Busy { text: "Applying the patch...".to_owned(), percent: None });
            std::thread::spawn(move || {
                let _ = sender.send(Msg::Rom(check_patch(&uri, &dir)));
            });
        }
        Page::Pack => {
            setup.set_status(Status::Busy { text: "Copying hd-pack.zip...".to_owned(), percent: Some(0) });
            std::thread::spawn(move || {
                let _ = sender.send(Msg::Pack(install_pack_from(&uri, &dir, &sender)));
            });
        }
        Page::Mode => {}
    }
}

fn check_rom(uri: &str, dir: &Path) -> Result<Vec<u8>, RomProblem> {
    let tmp = dir.join("rom-import.tmp");
    copy_uri(uri, &tmp, MAX_ROM_BYTES, &mut |_| ()).map_err(|e| RomProblem::Refused(e.to_string()))?;
    match rom_setup::resolve_rom(Some(&tmp), None, false) {
        Ok(rom_setup::RomSetup::Ready(bytes)) => {
            let _ = std::fs::remove_file(&tmp);
            Ok(bytes)
        }
        Ok(rom_setup::RomSetup::Missing) => {
            let _ = std::fs::remove_file(&tmp);
            Err(RomProblem::Refused("no ROM in this file".to_owned()))
        }
        Err(error) => {
            let text = format!("{error:#}");
            if text.contains(&ssd_rom::RomError::PatchRequired.to_string()) {
                // The copy stays for the patch page.
                Err(RomProblem::NeedsPatch)
            } else {
                let _ = std::fs::remove_file(&tmp);
                Err(RomProblem::Refused(text))
            }
        }
    }
}

fn check_patch(uri: &str, dir: &Path) -> Result<Vec<u8>, RomProblem> {
    let rom = dir.join("rom-import.tmp");
    let ips = dir.join("patch-import.tmp");
    let result = copy_uri(uri, &ips, MAX_PATCH_BYTES, &mut |_| ())
        .map_err(|e| e.to_string())
        .and_then(|()| {
            rom_setup::resolve_rom(Some(&rom), Some(&ips), false).map_err(|e| format!("{e:#}"))
        });
    let _ = std::fs::remove_file(&ips);
    match result {
        Ok(rom_setup::RomSetup::Ready(bytes)) => {
            let _ = std::fs::remove_file(&rom);
            Ok(bytes)
        }
        Ok(rom_setup::RomSetup::Missing) => Err(RomProblem::Refused("no ROM in this file".to_owned())),
        Err(reason) => Err(RomProblem::Refused(reason)),
    }
}

fn install_pack_from(uri: &str, dir: &Path, sender: &Sender<Msg>) -> Result<(), String> {
    let rom = match rom_setup::resolve_rom(None, None, false) {
        Ok(rom_setup::RomSetup::Ready(bytes)) => bytes,
        _ => return Err("the ROM is not set up".to_owned()),
    };
    let zip = dir.join("hd-pack-import.zip");
    let status = |text: &str, percent: Option<u8>| {
        let _ = sender.send(Msg::Status(Status::Busy { text: text.to_owned(), percent }));
    };
    let result = copy_uri(uri, &zip, i64::MAX, &mut |percent| {
        status("Copying hd-pack.zip...", u8::try_from(percent.clamp(0, 100)).ok());
    })
    .map_err(|e| e.to_string())
    .and_then(|()| {
        let source = install::find_source(Some(&zip), None).ok_or("this is not an HD pack")?;
        let cache = paths::data_dir().join("cache");
        let mut say = |line: &str| {
            eprintln!("{line}");
            let percent = line
                .rsplit_once(", ")
                .and_then(|(_, tail)| tail.strip_suffix('%'))
                .and_then(|figure| figure.parse::<u8>().ok());
            if line.contains("unpacking the art") {
                status("Unpacking the art...", percent);
            } else if line.contains("reading your ROM") {
                status("Building the graphics from your ROM...", None);
            } else if line.contains("putting the art") {
                status("Putting the art in place...", None);
            }
        };
        match install::install(&rom, &source, &cache, &mut say, false) {
            Ok(Outcome::Current(_) | Outcome::Installed { .. }) => Ok(()),
            Err(error) => Err(format!("{error:#}")),
        }
    });
    let _ = std::fs::remove_file(&zip);
    result
}

// ---- picker ----

unsafe extern "C" fn picked(userdata: *mut c_void, files: *const *const c_char, _filter: c_int) {
    // SAFETY: `userdata` is the box `pick_file` gave away; this callback owns it and frees it after the send (the
    // receiving side may be gone by then, so it cannot be the one that frees it).
    let sender = unsafe { Box::from_raw(userdata.cast::<Sender<Option<String>>>()) };
    let first = if files.is_null() {
        None
    } else {
        // SAFETY: SDL passes a NULL-terminated list of NUL-terminated strings.
        let first = unsafe { *files };
        (!first.is_null()).then(|| unsafe { CStr::from_ptr(first) }.to_string_lossy().into_owned())
    };
    let _ = sender.send(first);
}

/// Opens the system file picker (any file); the `content://` address of the file chosen arrives on the receiver,
/// `None` if the player cancelled.
fn pick_file() -> Receiver<Option<String>> {
    let (sender, receiver) = mpsc::channel();
    let userdata = Box::into_raw(Box::new(sender)).cast::<c_void>();
    // SAFETY: the callback takes the box back (see `picked`).
    unsafe {
        sdl3::sys::dialog::SDL_ShowOpenFileDialog(
            Some(picked),
            userdata,
            std::ptr::null_mut(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            false,
        );
    }
    receiver
}

/// Copies the file at `uri` to `dest` in blocks; `progress` gets the percent when the size is known. A file bigger
/// than `max` bytes is refused before anything is copied.
fn copy_uri(
    uri: &str,
    dest: &Path,
    max: i64,
    progress: &mut dyn FnMut(i32),
) -> Result<(), std::io::Error> {
    use sdl3::sys::iostream::{SDL_CloseIO, SDL_GetIOSize, SDL_IOFromFile, SDL_ReadIO};
    let fail = |text: String| std::io::Error::other(text);
    let name = CString::new(uri).map_err(|e| fail(e.to_string()))?;
    // SAFETY: plain SDL calls on a stream this function opens and closes.
    let stream = unsafe { SDL_IOFromFile(name.as_ptr(), c"rb".as_ptr()) };
    if stream.is_null() {
        return Err(fail(format!("the file cannot be opened: {}", sdl3::get_error())));
    }
    // SAFETY: `stream` is open.
    let size = unsafe { SDL_GetIOSize(stream) };
    let result = (|| {
        if size > max {
            return Err(fail("this file is too big to be what is asked for".to_owned()));
        }
        let mut out = std::io::BufWriter::new(std::fs::File::create(dest)?);
        let mut block = vec![0_u8; 1 << 20];
        let (mut done, mut last) = (0_i64, -1);
        loop {
            // SAFETY: `block` is valid for its length.
            let read = unsafe { SDL_ReadIO(stream, block.as_mut_ptr().cast(), block.len()) };
            if read == 0 {
                break;
            }
            out.write_all(&block[..read])?;
            done += read as i64;
            if size > 0 {
                let percent = i32::try_from(done * 100 / size).unwrap_or(100).min(100);
                if percent != last {
                    last = percent;
                    progress(percent);
                }
            }
        }
        out.flush()?;
        if size > 0 && done != size {
            return Err(fail(format!("the file ended after {done} of {size} bytes")));
        }
        Ok(())
    })();
    // SAFETY: the stream is open and not used again.
    unsafe { SDL_CloseIO(stream) };
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}
