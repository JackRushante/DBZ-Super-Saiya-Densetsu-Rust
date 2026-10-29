//! The Video page (this PC): the window (display, scale, V-sync), the picture (look, aspect, its size) and the
//! two filters. The rows are applied at once by the app; a change of display, scale or aspect is asked about
//! ("Apply video changes?"), and reverts after ten seconds unless it is kept.

use super::{Kind, RowDef, Source, Values};
use crate::cheats::Cheats;
use std::fmt::Write as _;

/// The scales, 2x to 8x.
pub const SCALES: &[&str] = &["2x", "3x", "4x", "5x", "6x", "7x", "8x"];
/// The displays.
pub const DISPLAYS: &[&str] = &["Window", "Fullscreen"];
/// The aspects: the 16:9 view with the extra map at the sides, and the original picture shape.
pub const ASPECTS: &[&str] = &["16:9", "4:3"];
/// The strengths of the scanlines.
pub const SCANLINES: &[&str] = &["Off", "Light", "Strong"];
/// The strengths of the CRT filter.
pub const CRTS: &[&str] = &["Off", "Soft", "Full"];

/// The width of the 16:9 view in PPU pixels (see `ssd_hd::world::WIDTH_16_9`), the original's width and its height.
pub const WIDTH_16_9: u32 = 372;
pub const WIDTH_4_3: u32 = 256;
pub const HEIGHT: u32 = 224;
/// The seconds a video change waits to be kept before it reverts.
pub const CONFIRM_SECONDS: u32 = 10;

/// The rows of the page.
pub const ROWS: &[RowDef] = &[
    RowDef {
        key: "video.display",
        label: "Display",
        group: "Window",
        help: "Window or fullscreen on the desktop. The change asks for confirmation.",
        kind: Kind::Choice(DISPLAYS),
        source: Source::Stored { default: 0 },
    },
    RowDef {
        key: "video.scale",
        label: "Scale",
        group: "Window",
        help: "How many times the PPU picture is redrawn in HD (2 to 8). Higher looks finer and costs more. The change asks for confirmation.",
        kind: Kind::Cycle(SCALES),
        source: Source::Stored { default: 2 },
    },
    RowDef {
        key: "video.vsync",
        label: "V-Sync",
        group: "Window",
        help: "Waits for the screen's refresh before showing a frame: no tearing, a little more delay.",
        kind: Kind::Choice(&["Off", "On"]),
        source: Source::Stored { default: 0 },
    },
    RowDef {
        key: "video.latency",
        label: "Latency",
        group: "Window",
        help: "Smooth draws the picture while the next frame runs: faster, one frame later. Low shows each frame at once.",
        kind: Kind::Choice(&["Low", "Smooth"]),
        source: Source::Stored { default: 1 },
    },
    RowDef {
        key: "video.look",
        label: "Look",
        group: "Picture",
        help: "Crisp is pixel art made bigger with smoothed diagonals. Smooth is a soft bicubic filter.",
        kind: Kind::Choice(&["Crisp", "Smooth"]),
        source: Source::Stored { default: 0 },
    },
    RowDef {
        key: "video.aspect",
        label: "Aspect",
        group: "Picture",
        help: "16:9 shows the extra map at the sides. 4:3 is the original picture shape. The change asks for confirmation.",
        kind: Kind::Choice(ASPECTS),
        source: Source::Stored { default: 0 },
    },
    RowDef {
        key: "video.size",
        label: "Picture size",
        group: "Picture",
        help: "The size of the picture the game draws at this scale and aspect.",
        kind: Kind::Info,
        source: Source::Info,
    },
    RowDef {
        key: "video.scanlines",
        label: "Scanlines",
        group: "Filters",
        help: "Dark lines between the lines of the original picture, as on a television.",
        kind: Kind::Choice(SCANLINES),
        source: Source::Stored { default: 0 },
    },
    RowDef {
        key: "video.crt",
        label: "CRT filter",
        group: "Filters",
        help: "A tube's look: a soft glow, a dark edge and a faint mask. Soft is light, Full is heavy.",
        kind: Kind::Choice(CRTS),
        source: Source::Stored { default: 0 },
    },
];

/// A line under the rows, if the page has one.
pub const NOTE: Option<&str> =
    Some("Changes ask for confirmation and revert after 10 s if you do not keep them.");

/// The index of the row `key` in `values` (its default when nothing is stored).
#[must_use]
pub fn index(values: &Values, key: &str) -> usize {
    ROWS.iter().find(|row| row.key == key).map_or(0, |row| row.index(values, &Cheats::neutral(), 0))
}

/// The three choices a change of which asks for confirmation: display, scale and aspect.
#[must_use]
pub fn mode(values: &Values) -> (usize, usize, usize) {
    (index(values, "video.display"), index(values, "video.scale"), index(values, "video.aspect"))
}

/// The size in pixels of the picture at scale index `scale` (0 is 2x) and aspect index `aspect`.
#[must_use]
pub fn picture_size(scale: usize, aspect: usize) -> (u32, u32) {
    let scale = 2 + u32::try_from(scale.min(SCALES.len() - 1)).unwrap_or(0);
    let width = if aspect == 1 { WIDTH_4_3 } else { WIDTH_16_9 };
    (width * scale, HEIGHT * scale)
}

/// The text of the "Picture size" row, given the values of the page's rows (their indices, in the order of [`ROWS`]).
#[must_use]
pub fn size_text(shown: &[usize]) -> String {
    let at = |key: &str| {
        ROWS.iter()
            .position(|row| row.key == key)
            .and_then(|at| shown.get(at))
            .copied()
            .unwrap_or(0)
    };
    let (width, height) = picture_size(at("video.scale"), at("video.aspect"));
    format!("{width} x {height}")
}

/// The question's second line: what the video would be if kept.
#[must_use]
pub fn summary(values: &Values) -> String {
    let (display, scale, aspect) = mode(values);
    let (width, height) = picture_size(scale, aspect);
    let mut text = format!(
        "{}, {}, {} ({width} x {height})",
        DISPLAYS[display.min(DISPLAYS.len() - 1)],
        SCALES[scale.min(SCALES.len() - 1)],
        ASPECTS[aspect.min(ASPECTS.len() - 1)],
    );
    for (key, labels, name) in
        [("video.scanlines", SCANLINES, "Scanlines"), ("video.crt", CRTS, "CRT")]
    {
        let chosen = index(values, key);
        if chosen > 0 {
            let _ = write!(text, ", {name} {}", labels[chosen.min(labels.len() - 1)]);
        }
    }
    text
}

/// Puts the keys of the page back to what `before` held (a reverted change).
pub fn restore(values: &mut Values, before: &Values) {
    for row in ROWS {
        match before.get(row.key) {
            Some(value) => values.set(row.key, value),
            None => values.remove(row.key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_is_smooth_unless_the_player_chooses_low() {
        let mut values = Values::new();
        assert_eq!(index(&values, "video.latency"), 1, "Smooth, the picture drawn meanwhile");
        values.set("video.latency", 0);
        assert_eq!(index(&values, "video.latency"), 0, "Low");
        let row = ROWS.iter().find(|row| row.key == "video.latency").unwrap();
        assert!(matches!(row.kind, Kind::Choice(["Low", "Smooth"])));
    }

    #[test]
    fn the_picture_size_follows_the_scale_and_the_aspect() {
        assert_eq!(picture_size(2, 0), (372 * 4, 224 * 4));
        assert_eq!(picture_size(0, 1), (512, 448));
        assert_eq!(size_text(&[0, 4, 0, 1, 0, 1, 0, 0, 0]), "1536 x 1344", "6x, 4:3");
    }

    #[test]
    fn the_summary_names_the_display_the_scale_the_aspect_and_the_filters_that_are_on() {
        let mut values = Values::new();
        assert_eq!(summary(&values), "Window, 4x, 16:9 (1488 x 896)");
        values.set("video.display", 1);
        values.set("video.scale", 4);
        values.set("video.aspect", 1);
        values.set("video.scanlines", 1);
        assert_eq!(summary(&values), "Fullscreen, 6x, 4:3 (1536 x 1344), Scanlines Light");
        values.set("video.crt", 2);
        assert!(summary(&values).ends_with(", CRT Full"));
    }

    #[test]
    fn restoring_forgets_the_keys_the_before_did_not_hold() {
        let mut before = Values::new();
        before.set("video.scale", 5);
        before.set("audio.music", 3);
        let mut now = before.clone();
        now.set("video.scale", 1);
        now.set("video.display", 1);
        now.set("audio.music", 9);
        restore(&mut now, &before);
        assert_eq!(now.get("video.scale"), Some(5));
        assert_eq!(now.get("video.display"), None);
        assert_eq!(now.get("audio.music"), Some(9), "other pages are left alone");
        assert_eq!(mode(&now), (0, 5, 0));
    }

    #[test]
    fn the_rows_have_three_groups_and_one_read_only_line() {
        let infos = ROWS.iter().filter(|row| row.kind == Kind::Info).count();
        assert_eq!((infos, ROWS.len()), (1, 9));
    }
}
