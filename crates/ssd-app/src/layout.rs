//! Where the game frame goes in the window: the largest whole-number scale that fits, centred
//! in the area the panels leave, with the rest left black (letterbox). All sizes are physical
//! pixels, so the scale stays whole whatever the display scale of the window is.

/// Where the frame is drawn inside an area of pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// The scale of one frame row: a whole number, unless not even ×1 fits (then the frame is
    /// shrunk to the area, `scale < 1`).
    pub scale: f32,
    /// Offset of the frame's top-left corner from the area's.
    pub x: f32,
    pub y: f32,
    /// Size of the drawn frame.
    pub width: f32,
    pub height: f32,
}

/// Width of one frame in pixels at scale `scale`, with the pixel aspect ratio applied.
fn scaled_width(frame_width: u32, pixel_aspect: f64, scale: u32) -> f32 {
    (f64::from(frame_width) * pixel_aspect * f64::from(scale)).round() as f32
}

/// Fits a `frame_width` x `frame_height` frame (pixels with the ratio `pixel_aspect`, width over
/// height) into `area_width` x `area_height` pixels.
///
/// The scale is the largest whole number for which the frame fits both ways. Its height is
/// `frame_height * scale` and its width the frame's width times the pixel aspect times the
/// scale, rounded to a pixel. An area that cannot hold the frame at ×1 gets it shrunk to fit,
/// keeping the aspect.
#[must_use]
pub fn fit_game(
    area_width: f32,
    area_height: f32,
    frame_width: u32,
    frame_height: u32,
    pixel_aspect: f64,
) -> Fit {
    let (area_width, area_height) = (area_width.max(0.0), area_height.max(0.0));
    let aspect = if pixel_aspect.is_finite() && pixel_aspect > 0.0 { pixel_aspect } else { 1.0 };
    if frame_width == 0 || frame_height == 0 {
        return Fit { scale: 0.0, x: 0.0, y: 0.0, width: 0.0, height: 0.0 };
    }
    let mut scale = (area_height / frame_height as f32).floor() as u32;
    while scale > 1 && scaled_width(frame_width, aspect, scale) > area_width {
        scale -= 1;
    }
    let (width, height, scale) = if scale >= 1
        && scaled_width(frame_width, aspect, scale) <= area_width
        && frame_height as f32 * scale as f32 <= area_height
    {
        (scaled_width(frame_width, aspect, scale), (frame_height * scale) as f32, scale as f32)
    } else {
        // Not even ×1 fits: shrink, keeping the aspect.
        let natural_width = scaled_width(frame_width, aspect, 1).max(1.0);
        let shrink = (area_width / natural_width).min(area_height / frame_height as f32);
        ((natural_width * shrink).floor(), (frame_height as f32 * shrink).floor(), shrink)
    };
    Fit {
        scale,
        x: ((area_width - width) / 2.0).floor(),
        y: ((area_height - height) / 2.0).floor(),
        width,
        height,
    }
}

/// The most lines a frame of the machine has (239 with overscan): a taller picture is an HD one, already drawn at
/// a multiple of the machine's resolution.
pub const MACHINE_LINES: u32 = 239;

/// Fits a frame as [`fit_game`] does, at any scale: as big as the area holds, keeping the aspect. For an HD
/// picture, which is drawn smoothed, on a screen its whole multiples do not fill (a phone: the HD picture is
/// about a thousand pixels wide, and twice that is wider than the screen).
#[must_use]
pub fn fit_any(
    area_width: f32,
    area_height: f32,
    frame_width: u32,
    frame_height: u32,
    pixel_aspect: f64,
) -> Fit {
    let aspect = if pixel_aspect.is_finite() && pixel_aspect > 0.0 { pixel_aspect } else { 1.0 };
    if frame_width == 0 || frame_height == 0 || area_width <= 0.0 || area_height <= 0.0 {
        return Fit { scale: 0.0, x: 0.0, y: 0.0, width: 0.0, height: 0.0 };
    }
    let natural_width = (f64::from(frame_width) * aspect) as f32;
    let scale = (area_width / natural_width).min(area_height / frame_height as f32);
    let (width, height) = ((natural_width * scale).floor(), (frame_height as f32 * scale).floor());
    Fit {
        scale,
        x: ((area_width - width) / 2.0).floor(),
        y: ((area_height - height) / 2.0).floor(),
        width,
        height,
    }
}

/// Where a game picture (`width` x `height` pixels, each with the ratio `pixel_aspect`) is drawn in
/// `area`, in points at `pixels_per_point`: the [`fit_game`] fit (`whole`), else the [`fit_any`] one,
/// worked out on the pixel grid. The live frame and the backdrop of an HD menu are both drawn in this
/// rect, so they cannot differ.
#[must_use]
pub fn picture_rect(
    pixels_per_point: f32,
    area: egui::Rect,
    width: u32,
    height: u32,
    pixel_aspect: f64,
    whole: bool,
) -> egui::Rect {
    let ppp = pixels_per_point;
    let origin = (area.min.to_vec2() * ppp).round();
    let end = (area.max.to_vec2() * ppp).floor();
    let pixels = end - origin;
    let fit =
        if whole { fit_game } else { fit_any }(pixels.x, pixels.y, width, height, pixel_aspect);
    let min = (origin + egui::vec2(fit.x, fit.y)) / ppp;
    egui::Rect::from_min_size(min.to_pos2(), egui::vec2(fit.width, fit.height) / ppp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_rect_is_the_fit_on_the_pixel_grid_in_points() {
        // A 1440x900 pixel window at 1.5 points per pixel is 960x600 points; the panel takes 240
        // points, so the area is 720x600 points, which is 1080x900 pixels.
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(720.0, 600.0));
        let rect = picture_rect(1.5, area, 256, 224, 8.0 / 7.0, true);
        // ×4 is 1170 pixels wide, too wide for 1080; ×3 is 878 x 672 pixels, centred in the area.
        assert_eq!(rect.size(), egui::vec2(878.0, 672.0) / 1.5);
        assert_eq!(
            rect.min,
            egui::pos2(101.0 / 1.5, 114.0 / 1.5),
            "((1080 - 878) / 2, (900 - 672) / 2)"
        );
        let again = picture_rect(1.5, area, 256, 224, 8.0 / 7.0, true);
        assert_eq!(rect, again, "the same inputs give the same rect");
    }

    #[test]
    fn any_scale_fills_the_area_one_way_keeping_the_aspect() {
        // an HD picture (1024 x 896, 1170 wide at 8:7) on a phone 1768 pixels wide: ×1 only by whole numbers,
        // about 1.51 by any (the width is the limit)
        let whole = fit_game(1768.0, 1400.0, 1024, 896, 8.0 / 7.0);
        assert!((whole.scale - 1.0).abs() < 1e-6);
        let any = fit_any(1768.0, 1400.0, 1024, 896, 8.0 / 7.0);
        assert!((any.width - 1768.0).abs() < 0.5 && (any.height - 1353.0).abs() < 1.5, "{any:?}");
        assert!(
            (any.x).abs() < 0.5 && (any.y - 23.0).abs() < 1.5,
            "centred the other way: {any:?}"
        );
        assert_eq!(fit_any(0.0, 10.0, 1024, 896, 1.0).width, 0.0);
    }

    #[test]
    fn the_largest_whole_scale_that_fits_is_used() {
        let fit = fit_game(1024.0, 896.0, 256, 224, 1.0);
        assert_eq!((fit.scale, fit.width, fit.height), (4.0, 1024.0, 896.0));
        assert_eq!((fit.x, fit.y), (0.0, 0.0));
        // Height-limited: 1000 / 224 = 4.46.
        let fit = fit_game(2000.0, 1000.0, 256, 224, 1.0);
        assert_eq!((fit.scale, fit.width, fit.height), (4.0, 1024.0, 896.0));
        // Width-limited: 600 / 256 = 2.34.
        let fit = fit_game(600.0, 2000.0, 256, 224, 1.0);
        assert_eq!((fit.scale, fit.width, fit.height), (2.0, 512.0, 448.0));
    }

    #[test]
    fn the_frame_is_centred_with_the_rest_left_for_the_letterbox() {
        let fit = fit_game(1001.0, 900.0, 256, 224, 1.0);
        assert_eq!((fit.scale, fit.width, fit.height), (3.0, 768.0, 672.0));
        assert_eq!(
            (fit.x, fit.y),
            (116.0, 114.0),
            "floor((1001 - 768) / 2), floor((900 - 672) / 2)"
        );
    }

    #[test]
    fn the_pixel_aspect_widens_the_frame_and_still_counts_for_the_fit() {
        let ratio = 8.0 / 7.0;
        let fit = fit_game(1200.0, 896.0, 256, 224, ratio);
        assert_eq!((fit.scale, fit.width, fit.height), (4.0, 1170.0, 896.0));
        // 1169 wide holds ×3 only (×4 is 1170 wide).
        let fit = fit_game(1169.0, 896.0, 256, 224, ratio);
        assert_eq!((fit.scale, fit.width, fit.height), (3.0, 878.0, 672.0));
    }

    #[test]
    fn an_area_too_small_for_one_to_one_shrinks_the_frame_keeping_the_aspect() {
        let fit = fit_game(128.0, 400.0, 256, 224, 1.0);
        assert_eq!((fit.scale, fit.width, fit.height), (0.5, 128.0, 112.0));
        assert_eq!((fit.x, fit.y), (0.0, 144.0));
    }

    #[test]
    fn degenerate_input_gives_an_empty_fit_or_falls_back_to_square_pixels() {
        assert_eq!(fit_game(100.0, 100.0, 0, 224, 1.0).width, 0.0);
        assert_eq!(fit_game(-5.0, 100.0, 256, 224, 1.0).width, 0.0);
        let fit = fit_game(512.0, 448.0, 256, 224, f64::NAN);
        assert_eq!((fit.scale, fit.width), (2.0, 512.0));
    }
}
