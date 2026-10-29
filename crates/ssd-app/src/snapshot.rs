//! UI snapshots: the debug panel drawn offscreen and written as a PNG, to check the interface
//! without a window. The drawing is the window's ([`crate::canvas`], [`crate::ui`]); only the target
//! differs: a texture here, the surface there. No window is created and no input is read.

use crate::canvas::{self, Canvas, GameImage, GameView};
use crate::egui_input::{Geometry, InputAdapter};
use crate::ui_script::Line;
use anyhow::Context;
use std::path::Path;

/// The size of a snapshot without `--ui-size`: the size the window opens with.
pub const DEFAULT_SIZE: (u32, u32) = (1440, 900);
/// The largest side of a snapshot, in pixels. Past it the picture's readback buffer is larger than
/// wgpu's default buffer limit, and no snapshot needs that much.
pub const MAX_SIDE: u32 = 4096;

/// The texture the interface is drawn into: RGBA, not sRGB, so the colours egui writes are the
/// colours of the file.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Passes of the interface run before the one that is drawn: anchored areas and measured rows take
/// their place from the pass before.
const SETTLE_PASSES: usize = 2;

/// What a snapshot is asked for.
pub struct Request<'a> {
    /// Frames run headless before the script, as `--headless-frames`.
    pub frames: u64,
    /// The PNG to write.
    pub out: &'a Path,
    /// Its size in pixels (width, height).
    pub size: (u32, u32),
    /// The script's lines, applied in order.
    pub script: &'a [Line],
    /// The Enhanced picture (`--hd-scale`), or the Classic one when none.
    pub enhanced: Option<ssd_hd::HdOptions>,
    /// Draw the touch controls (`--touch-ui`); the script's `touch` lines set their state.
    pub touch: bool,
}

/// Where the time of one presented frame went, in the order of the stages: the interface run and the
/// game textures uploaded, the pass painted and submitted, and the wait for the GPU to finish the
/// frame before the last.
#[derive(Debug, Default, Clone, Copy)]
pub struct PresentTimes {
    pub ui: std::time::Duration,
    pub paint: std::time::Duration,
    pub wait: std::time::Duration,
}

/// The adapter an offscreen run draws on: the first backend to try, no surface.
fn offscreen_adapter() -> anyhow::Result<wgpu::Adapter> {
    let instance = canvas::instance();
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .map_err(|e| anyhow::anyhow!("no wgpu adapter: {e}"))
}

/// One line naming the adapter an offscreen run draws on (the backend, the kind of device, the driver).
///
/// # Errors
///
/// No adapter is available.
pub fn describe_gpu() -> anyhow::Result<String> {
    let info = offscreen_adapter()?.get_info();
    Ok(format!(
        "{} ({:?}, {:?}), driver {} {}",
        info.name, info.backend, info.device_type, info.driver, info.driver_info
    ))
}

/// The interface drawn into an offscreen texture, on a device of its own (no window, no surface).
pub struct Offscreen {
    canvas: Canvas,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    /// The last frame presented: the next waits for it, as a window with one frame of latency does.
    last: Option<wgpu::SubmissionIndex>,
}

impl Offscreen {
    /// Sets up wgpu without a window and a `width` x `height` texture to draw into.
    ///
    /// # Errors
    ///
    /// No adapter or device is available.
    pub fn new(width: u32, height: u32) -> anyhow::Result<Self> {
        let adapter = offscreen_adapter()?;
        let info = adapter.get_info();
        log::info!("wgpu adapter {} ({:?}), offscreen", info.name, info.backend);
        let (device, queue) = canvas::device(&adapter)?;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui snapshot"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let canvas = Canvas::new(device, queue, FORMAT);
        Ok(Self { canvas, target, view, width, height, last: None })
    }

    /// Draws the interface over `views` as the window does, and returns its pixels: RGBA, top row
    /// first, `width * height * 4` bytes.
    ///
    /// # Errors
    ///
    /// The pixels cannot be read back from the GPU.
    pub fn draw(
        &mut self,
        views: &[GameView<'_>],
        mut ui: impl FnMut(&mut egui::Ui, &[GameImage]),
    ) -> anyhow::Result<Vec<u8>> {
        let size = (self.width, self.height);
        let max_side = self.canvas.max_texture_side();
        // A snapshot has no pointer and no keys: the input is the empty one, at the size of the
        // target, one point to a pixel.
        let geometry = Geometry { pixel_density: 1.0, pixels_per_point: 1.0 };
        let mut input = InputAdapter::default();
        for _ in 0..SETTLE_PASSES {
            let raw = input.take_raw_input(geometry, size, 0.0, max_side);
            let pass = self.canvas.run(raw, views, &mut ui);
            self.canvas.discard(pass);
        }
        let raw = input.take_raw_input(geometry, size, 0.0, max_side);
        let pass = self.canvas.run(raw, views, &mut ui);
        self.canvas.paint(pass, &self.view, [self.width, self.height]);
        self.read_back()
    }

    /// Presents one frame as the window does, minus the surface: runs the interface over `views`
    /// with the input `input` had since the last frame, paints it, submits it, and waits for the
    /// GPU to finish the frame before. `now` is the interface's clock (seconds).
    ///
    /// # Errors
    ///
    /// The GPU cannot be waited for.
    pub fn present(
        &mut self,
        input: &mut InputAdapter,
        now: f64,
        views: &[GameView<'_>],
        ui: impl FnMut(&mut egui::Ui, &[GameImage]),
    ) -> anyhow::Result<PresentTimes> {
        let geometry = Geometry { pixel_density: 1.0, pixels_per_point: 1.0 };
        let started = std::time::Instant::now();
        let raw = input.take_raw_input(
            geometry,
            (self.width, self.height),
            now,
            self.canvas.max_texture_side(),
        );
        let pass = self.canvas.run(raw, views, ui);
        let ran = std::time::Instant::now();
        let submitted = self.canvas.paint(pass, &self.view, [self.width, self.height]);
        let painted = std::time::Instant::now();
        if let Some(previous) = self.last.replace(submitted) {
            self.canvas
                .device()
                .poll(wgpu::PollType::Wait { submission_index: Some(previous), timeout: None })
                .map_err(|e| anyhow::anyhow!("waiting for the GPU: {e}"))?;
        }
        Ok(PresentTimes { ui: ran - started, paint: painted - ran, wait: painted.elapsed() })
    }

    /// Copies the target into a buffer the CPU can map, and returns its rows without the padding
    /// wgpu adds to each.
    fn read_back(&self) -> anyhow::Result<Vec<u8>> {
        read_texture(
            self.canvas.device(),
            self.canvas.queue(),
            &self.target,
            self.width,
            self.height,
        )
    }
}

/// Copies a 4-byte-per-pixel texture (`COPY_SRC`) into a buffer the CPU can map, and returns its rows
/// without the padding wgpu adds to each: the readback of the snapshots and of the window's screenshots.
///
/// # Errors
///
/// The pixels cannot be read back from the GPU.
pub fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> anyhow::Result<Vec<u8>> {
    let row = width * 4;
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = row.div_ceil(alignment) * alignment;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ui snapshot readback"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui snapshot") });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    queue.submit(std::iter::once(encoder.finish()));
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        // The receiver waits below, so it is there; a failed send would mean it is gone.
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| anyhow::anyhow!("waiting for the GPU: {e}"))?;
    receiver
        .recv()
        .context("the snapshot was not read back")?
        .map_err(|e| anyhow::anyhow!("mapping the snapshot: {e}"))?;
    let mapped =
        buffer.get_mapped_range(..).map_err(|e| anyhow::anyhow!("reading the snapshot: {e}"))?;
    let mut pixels = Vec::with_capacity(row as usize * height as usize);
    for line in mapped.chunks_exact(padded as usize) {
        pixels.extend_from_slice(&line[..row as usize]);
    }
    drop(mapped);
    buffer.unmap();
    Ok(pixels)
}
