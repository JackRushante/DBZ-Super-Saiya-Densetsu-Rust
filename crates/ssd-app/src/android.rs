//! The Android entry of the app: `SDL_main`, called by SDL's activity on the thread it starts for the game.
//! Everything else is the desktop program, included from `ssd-app`.
#![cfg(target_os = "android")]
#![allow(unsafe_code)]
#![allow(dead_code)] // `main` is the desktop entry

// The desktop program, as the root of this library: its modules are `crate::...`.
include!("main.rs");

mod android_lifecycle;
mod android_setup;

use std::ffi::{OsString, c_char, c_int};

/// The file of extra launch arguments, one line, split on spaces, read from the data folder (a test aid:
/// `--mode enhanced`, `--install-hd-pack <zip>`, `--bench-app <replay>...`). Without it the game starts in Classic
/// mode. `ENV:NAME=value` sets an environment variable. A `--bench-app` run (README, "Developer commands") has no
/// standard output here: its report goes to `bench-report.txt` in the data folder, and replays named relatively are
/// looked for there too.
const ARGS_FILE: &str = "android-args.txt";

/// The folder of the game's files: the app's internal folder (private to the app, removed with it; a debug
/// build can be reached with `adb shell run-as`).
fn data_dir() -> Option<PathBuf> {
    // SAFETY: SDL is initialised by the activity before it runs `SDL_main`; the string is static to SDL.
    let path = unsafe { sdl3::sys::system::SDL_GetAndroidInternalStoragePath() };
    if path.is_null() {
        return None;
    }
    // SAFETY: a NUL-terminated string that lives as long as SDL.
    let text = unsafe { std::ffi::CStr::from_ptr(path) }.to_string_lossy().into_owned();
    Some(PathBuf::from(text))
}

fn extra_args(dir: &Path) -> Vec<OsString> {
    std::fs::read_to_string(dir.join(ARGS_FILE))
        .map(|text| text.split_whitespace().map(OsString::from).collect())
        .unwrap_or_default()
}

/// The entry point SDL's activity looks up in `libmain.so`.
#[unsafe(no_mangle)]
pub extern "C" fn SDL_main(_argc: c_int, _argv: *mut *mut c_char) -> c_int {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("ssd"),
    );
    forward_stderr();
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
    let mut base = vec![OsString::from("ssd-recompiled")];
    if let Some(dir) = data_dir() {
        log::info!("data folder {}", dir.display());
        // SAFETY: nothing else has started a thread that reads the environment yet.
        unsafe { std::env::set_var("SSD_DATA_DIR", &dir) };
        for arg in extra_args(&dir) {
            // `ENV:NAME=value` sets an environment variable (for example `ENV:WGPU_BACKEND=gl`).
            match arg.to_str().and_then(|a| a.strip_prefix("ENV:")).and_then(|a| a.split_once('=')) {
                // SAFETY: still single-threaded here.
                Some((name, value)) => unsafe { std::env::set_var(name, value) },
                None => base.push(arg),
            }
        }
        if base.iter().any(|arg| arg == "--bench-app") && !base.iter().any(|arg| arg == "--bench-report") {
            base.push(OsString::from("--bench-report"));
            base.push(dir.join("bench-report.txt").into_os_string());
        }
    }
    // One SDL context for the whole life of the activity: the setup screens and the game each open their window
    // from it, and the setup comes back when the game asks for it (Options, or Select + Start in Classic).
    let _sdl = sdl3::init();
    let mut forced = None;
    loop {
        let mut args = base.clone();
        if let Some(dir) = data_dir() {
            if !android_setup::run(&mut args, &dir, forced.take()) {
                return 0;
            }
            let (mode, pack) = android_setup::game_info(&dir);
            ssd_menus::options::set_game_info(&mode, &pack);
            ssd_menus::options::show_game_page(true);
        }
        log::info!("arguments {args:?}");
        let code = run(args);
        log::info!("the game ended with {code:?}");
        match android_setup::take_request() {
            Some(request) => forced = Some(request),
            None => return 0,
        }
    }
}

/// Sends what the program writes to stderr (its error messages) to the log, where `adb logcat` shows it.
fn forward_stderr() {
    let mut fds = [0 as c_int; 2];
    // SAFETY: plain POSIX calls on descriptors this function creates; stderr (2) is replaced by the pipe's
    // write end, and the read end belongs to the thread below.
    let reader = unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 || libc::dup2(fds[1], 2) < 0 {
            return;
        }
        libc::close(fds[1]);
        fds[0]
    };
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        use std::os::fd::FromRawFd;
        // SAFETY: `reader` is a descriptor nothing else owns.
        let file = unsafe { std::fs::File::from_raw_fd(reader) };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            log::error!("{line}");
        }
    });
}
