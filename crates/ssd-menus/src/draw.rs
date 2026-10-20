//! The draw list: what an HD screen paints, as plain data in logical pixels. The screens build it
//! and the backend of `ssd-app` paints it, so a layout can be checked without a window or a GPU.
use crate::view::Picture;

/// An RGBA colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
    /// An opaque colour from its red, green and blue parts.
    #[must_use]
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self([red, green, blue, 255])
    }
}

/// A rectangle in logical pixels, from its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// The rectangle with its top-left corner at (`x`, `y`).
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// The right edge.
    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    /// The bottom edge.
    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// The rectangle grown by `d` on every side (shrunk for a negative `d`).
    #[must_use]
    pub fn grow(&self, d: f32) -> Self {
        Self::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }

    /// The vertical middle of the rectangle.
    #[must_use]
    pub fn middle_y(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

/// Where a text run lies horizontally, from its `x`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// The run starts at `x`.
    Left,
    /// The run is centred on `x`.
    Centre,
    /// The run ends at `x`.
    Right,
}

/// One step of a draw list.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// The ground of the screen: the field as it was when the menu opened. The app paints the frame
    /// it froze over the game picture (the rect the live frame is drawn in); nothing is painted where
    /// no frame was frozen.
    Backdrop,
    /// A filled rectangle.
    Fill { rect: Rect, color: Color },
    /// A filled disc inside `rect`: a circle when the rectangle is square (the balls of a card).
    Disc { rect: Rect, color: Color },
    /// The outline of a rectangle, drawn inside its bounds.
    Outline { rect: Rect, width: f32, color: Color },
    /// A menu window: cream paper in an orange frame, dark-red lines outside and inside the frame
    /// and a corner cap on each corner. `rect` is the outside of the frame.
    Window { rect: Rect },
    /// A straight line.
    Line { from: (f32, f32), to: (f32, f32), width: f32, color: Color },
    /// A line of text, vertically centred on `y`.
    Text { x: f32, y: f32, text: String, size: f32, color: Color, align: Align },
    /// The orange cursor, a right-pointing triangle with its top-left corner at (`x`, `y`).
    Cursor { x: f32, y: f32 },
    /// A small orange triangle pointing left or right (the cheats' arrows, which the font has no glyph
    /// for), the cursor's size, with its top-left corner at (`x`, `y`).
    Arrow { x: f32, y: f32, left: bool },
    /// A menu window in high contrast: white paper in a black frame with a white line outside it, so it stands off
    /// the field. `rect` is the outside of the frame, as for [`Op::Window`].
    WindowHc { rect: Rect },
    /// A triangle in `rect` pointing right (or left), filled with `fill`, outlined with a wide `edge` line and a
    /// narrower `ring` line on it: the safe cursor, and the arrows in high contrast.
    Pointer { rect: Rect, left: bool, fill: Color, edge: Color, ring: Color },
    /// A picture in a square portrait frame, or an empty box when there is nothing to show (an empty list,
    /// or a character without a corner face). `rect` is the outside of the frame.
    Portrait { rect: Rect, picture: Option<Picture> },
    /// The card of the Cheats screen's preview: the battle's HD card drawn from the pack's card art (the
    /// attack ball with four dots or the Z, the symbol, the defence ball with the numeral seven or the Z)
    /// to fit `rect`, which is the outside of the card's frame. A backend with no card art paints
    /// [`crate::screens::pixel_card`] instead.
    Card { rect: Rect, attack_z: bool, defence_z: bool },
    /// A bar filled to `fraction` (0 to 1) of its well, with a white border. `rect` is the outside.
    Bar { rect: Rect, fraction: f32, color: Color },
}

/// The steps of one screen, in the order they are painted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrawList {
    ops: Vec<Op>,
}

impl DrawList {
    /// An empty list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The steps, in paint order.
    #[must_use]
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Appends a step.
    pub fn push(&mut self, op: Op) {
        self.ops.push(op);
    }

    /// Appends the steps of `other`, in their order.
    pub fn append(&mut self, other: DrawList) {
        self.ops.extend(other.ops);
    }

    /// Appends a filled rectangle.
    pub fn fill(&mut self, rect: Rect, color: Color) {
        self.push(Op::Fill { rect, color });
    }

    /// Appends a text run.
    pub fn text(&mut self, x: f32, y: f32, text: &str, size: f32, color: Color, align: Align) {
        self.push(Op::Text { x, y, text: text.to_owned(), size, color, align });
    }

    /// Replaces the text of each run with what `translate` gives for it (a run it gives `None` for keeps its own).
    pub fn map_text(&mut self, translate: impl Fn(&str) -> Option<String>) {
        for op in &mut self.ops {
            if let Op::Text { text, .. } = op
                && let Some(translated) = translate(text)
            {
                *text = translated;
            }
        }
    }

    /// The steps that are text runs, in paint order (for tests and for reading a screen back).
    pub fn texts(&self) -> impl Iterator<Item = (f32, f32, &str)> {
        self.ops.iter().filter_map(|op| match op {
            Op::Text { x, y, text, .. } => Some((*x, *y, text.as_str())),
            _ => None,
        })
    }
}
