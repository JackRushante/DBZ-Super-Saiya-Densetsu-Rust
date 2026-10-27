//! The first launch's choice of the language (Enhanced mode): before the game's first frame, while the PC file holds
//! no language and `--lang` names none, the game's window shows the setup page of `ssd_menus::setup`
//! ([`Page::Language`]) in the HD menus' style, full screen. The arrows, the d-pad, Enter, Space, X and the pad's
//! South and East buttons choose with the keys; a click (desktop) or a tap (the touch screen's fingers, which the game
//! takes from SDL itself) chooses a button. The choice is written to the PC file, so it is asked once. A run that
//! does not persist (a headless run, a snapshot, a replay) never asks.

use crate::egui_input::InputAdapter;
use crate::hd::{Art, Fit};
use crate::video::Video;
use crate::{paths, setup_ui};
use sdl3::event::{Event, WindowEvent};
use sdl3::gamepad::{Button, Gamepad};
use sdl3::keyboard::Scancode;
#[cfg(test)]
use ssd_menus::language::Locale;
use ssd_menus::options::Values;
use ssd_menus::setup::{self, Command, Page, Setup};
use ssd_menus::{Key, Language};
use std::time::Duration;

/// Whether the run asks for the language: an Enhanced run (`enhanced`) that writes the player's files, where the
/// player has a translation, whose PC file (`settings`) holds no language, and whose command line gave none.
#[must_use]
pub fn needed(enhanced: bool, settings: &Values) -> bool {
    enhanced
        && paths::persists()
        && ssd_menus::options::language_row_shown()
        && crate::lang::overridden().is_none()
        && Language::from_values(settings).is_none()
}

/// The key of the page a keyboard key is, if any.
fn key_of(code: Scancode) -> Option<Key> {
    match code {
        Scancode::Up => Some(Key::Up),
        Scancode::Down => Some(Key::Down),
        Scancode::Left => Some(Key::Left),
        Scancode::Right => Some(Key::Right),
        Scancode::Tab => Some(Key::Tab),
        Scancode::Return | Scancode::KpEnter | Scancode::Space | Scancode::X => Some(Key::Confirm),
        _ => None,
    }
}

/// The key of the page a gamepad button is, if any: both South and East confirm (the game's A is East, the system's
/// usual confirm South).
fn button_of(button: Button) -> Option<Key> {
    match button {
        Button::DPadUp => Some(Key::Up),
        Button::DPadDown => Some(Key::Down),
        Button::DPadLeft => Some(Key::Left),
        Button::DPadRight => Some(Key::Right),
        Button::South | Button::East | Button::Start => Some(Key::Confirm),
        _ => None,
    }
}

/// The language whose button is under the point `at` (egui points) of a page drawn over `screen`.
fn tapped(page: &Setup, screen: egui::Rect, at: egui::Pos2) -> Option<Command> {
    let fit = Fit::sized(screen, setup::WIDTH, setup::HEIGHT);
    page.buttons()
        .into_iter()
        .find(|button| button.enabled && fit.rect(button.rect).contains(at))
        .map(|button| button.command)
}

/// Shows the page in the game's window until a language is chosen; `None` when the player closes the window.
///
/// # Errors
///
/// The window cannot be drawn.
pub fn run(
    video: &mut Video,
    events: &mut sdl3::EventPump,
    input: &mut InputAdapter,
    gamepads: (Option<&sdl3::GamepadSubsystem>, &mut Vec<Gamepad>),
) -> anyhow::Result<Option<Language>> {
    let (subsystem, pads) = gamepads;
    let mut page = Setup::new(Page::Language);
    let mut art = Art::empty();
    #[cfg_attr(not(target_os = "android"), allow(unused_mut))]
    let mut background = false;
    loop {
        let mut commands = Vec::new();
        for event in events.poll_iter() {
            video.handle_event(input, &event);
            match event {
                Event::Quit { .. } => return Ok(None),
                Event::KeyDown { scancode: Some(code), repeat: false, .. } => {
                    commands.extend(key_of(code).and_then(|key| page.key(key)));
                }
                Event::GamepadButtonDown { button, .. } => {
                    commands.extend(button_of(button).and_then(|key| page.key(key)));
                }
                Event::GamepadAdded { which, .. } => {
                    if let Some(pad) = subsystem.and_then(|subsystem| subsystem.open(which).ok()) {
                        pads.push(pad);
                    }
                }
                Event::GamepadRemoved { which, .. } => {
                    pads.retain(|pad| pad.id().ok() != Some(which));
                }
                // A finger (the game turns SDL's touch-to-mouse events off where it has touch controls).
                Event::FingerUp { x, y, .. } => {
                    let screen = video.screen_points();
                    let at = egui::pos2(
                        screen.min.x + x * screen.width(),
                        screen.min.y + y * screen.height(),
                    );
                    commands.extend(tapped(&page, screen, at));
                }
                Event::Window { win_event, .. } => match win_event {
                    WindowEvent::Minimized => video.set_minimized(true),
                    WindowEvent::Restored | WindowEvent::Maximized => {
                        video.set_minimized(false);
                        video.handle_resize();
                    }
                    WindowEvent::Resized(..) | WindowEvent::PixelSizeChanged(..) => {
                        video.handle_resize();
                    }
                    #[cfg(target_os = "android")]
                    WindowEvent::FocusLost if !background => {
                        background = true;
                        video.suspend();
                    }
                    #[cfg(target_os = "android")]
                    WindowEvent::FocusGained if background => {
                        background = false;
                        video.resume();
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if background {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        }
        let shown = page.clone();
        video.present(&[], input, |ui, _| {
            commands.extend(setup_ui::show(ui, &shown, &mut art));
        })?;
        if let Some(Command::Language(language)) =
            commands.into_iter().find(|command| matches!(command, Command::Language(_)))
        {
            return Ok(Some(language));
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tap_on_a_button_chooses_its_language_wherever_the_page_is_fitted() {
        let made_up = Locale::unnamed(Language::from_code("xx").unwrap());
        let languages = vec![Locale::english(), made_up, Locale::unnamed(Language::ZH_HK)];
        let page = Setup::with_languages(Page::Language, languages.clone());
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1864.0, 860.0));
        let fit = Fit::sized(screen, setup::WIDTH, setup::HEIGHT);
        for (button, locale) in page.buttons().iter().zip(&languages) {
            let centre = fit.rect(button.rect).center();
            assert_eq!(tapped(&page, screen, centre), Some(Command::Language(locale.language)));
        }
        assert_eq!(tapped(&page, screen, egui::pos2(2.0, 2.0)), None, "the ground chooses nothing");
    }

    #[test]
    fn the_keys_and_buttons_confirm_and_move() {
        assert_eq!(key_of(Scancode::Return), Some(Key::Confirm));
        assert_eq!(key_of(Scancode::Down), Some(Key::Down));
        assert_eq!(key_of(Scancode::Escape), None, "there is no going back");
        assert_eq!(button_of(Button::East), Some(Key::Confirm));
        assert_eq!(button_of(Button::DPadUp), Some(Key::Up));
    }

    #[test]
    fn only_an_enhanced_run_that_persists_with_no_language_asks() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _lang = crate::lang::LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::lang::configure(None);
        paths::set_persistent(true);
        let mut values = Values::new();
        ssd_menus::options::offer_languages(Vec::new());
        assert!(!needed(true, &values), "no translation: never asked");
        ssd_menus::options::offer_languages(vec![Locale::unnamed(Language::ZH_HK)]);
        assert!(needed(true, &values));
        assert!(!needed(false, &values), "Classic never asks");
        Language::ENGLISH.store(&mut values);
        assert!(!needed(true, &values), "asked once");
        paths::set_persistent(false);
        assert!(!needed(true, &Values::new()), "a run that does not persist never asks");
        ssd_menus::options::offer_languages(Vec::new());
    }
}
