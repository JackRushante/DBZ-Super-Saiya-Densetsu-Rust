//! Paints a first-launch screen (`ssd_menus::setup`) with the HD menus' own drawing and reads the touches on its
//! buttons. The screens are drawn in their own logical space and fitted to the window, centred.

use crate::hd::{Art, Fit, paint_fit};
use ssd_menus::setup::{self, Command, Setup};

/// Paints `setup` over the whole of `ui`, and returns the command of the button that was tapped or clicked.
pub fn show(ui: &mut egui::Ui, setup: &Setup, art: &mut Art) -> Option<Command> {
    let area = ui.available_rect_before_wrap();
    let fit = Fit::sized(area, setup::WIDTH, setup::HEIGHT);
    ui.painter().rect_filled(area, 0.0, egui::Color32::from_rgb(0x0B, 0x07, 0x06));
    paint_fit(ui, area, &fit, &setup.scene(), art);
    let mut chosen = None;
    for (index, button) in setup.buttons().iter().enumerate() {
        if !button.enabled {
            continue;
        }
        let response = ui.interact(
            fit.rect(button.rect),
            egui::Id::new(("first-launch button", index)),
            egui::Sense::click(),
        );
        if response.clicked() {
            chosen = Some(button.command);
        }
    }
    ui.allocate_rect(area, egui::Sense::hover());
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::Offscreen;
    use ssd_menus::setup::{Page, Status};

    /// The Options' Game page (Android only) draws with its rows; hidden again afterwards.
    #[test]
    fn the_game_page_of_the_options_draws_offscreen() {
        let Ok(mut offscreen) = Offscreen::new(1280, 720) else { return };
        ssd_menus::options::show_game_page(true);
        ssd_menus::options::set_game_info("Enhanced", "Revision 3, 0.9 GB");
        let view = ssd_menus::view::OptionsView {
            category: ssd_menus::options::Category::Game,
            row: 1,
            values: vec![0; 4],
            confirm: None,
            video: None,
            controls: None,
        };
        let list = ssd_menus::screens::options_screen(&view);
        ssd_menus::options::show_game_page(false);
        let mut art = Art::empty();
        let pixels = offscreen
            .draw(&[], |ui, _| {
                let area = ui.available_rect_before_wrap();
                crate::hd::paint(ui, area, &list, &mut art);
            })
            .expect("the page draws");
        if let Some(dir) = std::env::var_os("SSD_SETUP_PREVIEW_DIR").map(std::path::PathBuf::from) {
            let _ = std::fs::create_dir_all(&dir);
            ssd_hd::png::write_rgba(&dir.join("options-game.png"), 1280, 720, &pixels)
                .expect("written");
        }
    }

    /// Draws every page (and a few states) offscreen. With `SSD_SETUP_PREVIEW_DIR` set, the pictures are written
    /// there for a look; without a GPU adapter the test has nothing to draw on and passes.
    #[test]
    fn every_page_draws_offscreen() {
        let Ok(mut offscreen) = Offscreen::new(1864, 860) else { return };
        let dir = std::env::var_os("SSD_SETUP_PREVIEW_DIR").map(std::path::PathBuf::from);
        let mut pages: Vec<(String, Setup)> = Vec::new();
        for (name, page) in [
            ("rom", Page::Rom),
            ("patch", Page::Patch),
            ("mode", Page::Mode),
            ("pack", Page::Pack),
            ("language", Page::Language),
        ] {
            pages.push((name.to_owned(), Setup::new(page)));
        }
        let mut error = Setup::new(Page::Rom);
        error.set_status(Status::Error(
            "This file was not accepted: unknown ROM (CRC-32 0F1E2D3C). Choose another file."
                .to_owned(),
        ));
        pages.push(("rom-error".to_owned(), error));
        let mut done = Setup::new(Page::Rom);
        done.set_status(Status::Done("ROM accepted: English v1.02.".to_owned()));
        pages.push(("rom-done".to_owned(), done));
        let mut busy = Setup::new(Page::Pack);
        busy.set_status(Status::Busy {
            text: "Unpacking the art...".to_owned(),
            percent: Some(62),
        });
        pages.push(("pack-busy".to_owned(), busy));
        let mut finished = Setup::new(Page::Pack);
        finished.set_status(Status::Done("Installed.".to_owned()));
        pages.push(("pack-done".to_owned(), finished));
        let mut failed = Setup::new(Page::Pack);
        failed.set_status(Status::Error(
            "The HD pack was not installed: this is not an HD pack.".to_owned(),
        ));
        pages.push(("pack-error".to_owned(), failed));
        let mut art = Art::empty();
        for (name, setup) in &pages {
            let pixels = offscreen
                .draw(&[], |ui, _| {
                    assert_eq!(show(ui, setup, &mut art), None, "nothing is tapped in a snapshot");
                })
                .expect("the page draws");
            assert!(
                pixels.as_chunks::<4>().0.iter().any(|p| p[..3] == [0xFF, 0xEF, 0xCE]),
                "{name}: the paper"
            );
            if let Some(dir) = &dir {
                let _ = std::fs::create_dir_all(dir);
                ssd_hd::png::write_rgba(&dir.join(format!("setup-{name}.png")), 1864, 860, &pixels)
                    .expect("the picture is written");
            }
        }
    }
}
