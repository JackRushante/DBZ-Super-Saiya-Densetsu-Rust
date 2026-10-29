use jgenesis_common::frontend::{AudioOutput, Color, FrameSize, RenderFrameOptions, Renderer};
use std::convert::Infallible;

pub trait AudioSink {
    fn push(&mut self, left: f32, right: f32);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NullAudio;

impl AudioSink for NullAudio {
    fn push(&mut self, _left: f32, _right: f32) {}
}

/// Captures the last frame jgenesis renders (pixels + size + its own render options).
#[derive(Clone, Default)]
pub(crate) struct FrameCapture {
    pub pixels: Vec<Color>,
    pub size: Option<FrameSize>,
    pub options: RenderFrameOptions,
    /// The PPU is not drawing its pixels (`Machine::set_compose_pixels`): a 256-pixel frame is not copied either, as
    /// the buffer holds nothing of it. The size and options are noted all the same.
    pub skip_pixels: bool,
}

impl Renderer for FrameCapture {
    type Err = Infallible;

    fn render_frame(
        &mut self,
        frame_buffer: &[Color],
        frame_size: FrameSize,
        _target_fps: f64,
        options: RenderFrameOptions,
    ) -> Result<(), Infallible> {
        let len = (frame_size.width * frame_size.height) as usize;
        self.size = Some(frame_size);
        self.options = options;
        // Only a frame of the size the PPU leaves undrawn, over a copy of that size: a hi-res frame is always drawn
        if self.skip_pixels
            && frame_size.width <= 256
            && frame_size.height <= 239
            && self.pixels.len() == len
        {
            return Ok(());
        }
        self.pixels.clear();
        self.pixels.extend_from_slice(&frame_buffer[..len]);
        Ok(())
    }
}

pub(crate) struct AudioAdapter<'a>(pub &'a mut dyn AudioSink);

impl AudioOutput for AudioAdapter<'_> {
    type Err = Infallible;

    fn push_sample(&mut self, l: f64, r: f64) -> Result<(), Infallible> {
        self.0.push(l as f32, r as f32);
        Ok(())
    }
}
