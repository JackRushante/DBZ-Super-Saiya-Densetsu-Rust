//! The drawing shared by the window and the UI snapshots: the game frames as textures, and the egui
//! interface painted through egui-wgpu into any texture. The window paints into its surface, the
//! snapshot into an offscreen texture, both through [`Canvas::paint`], so a snapshot shows what the
//! window draws.

use ssd_hd::HdFrame;
use ssd_oracle::Frame;

/// The game frame as egui draws it.
#[derive(Debug, Clone, Copy)]
pub struct GameImage {
    pub id: egui::TextureId,
    pub width: u32,
    pub height: u32,
    /// Width over height of one pixel (1 when the frame does not say).
    pub pixel_aspect: f64,
}

/// One game view as the canvas uploads it: RGBA pixels of `width` x `height`, and the width over
/// height of one pixel. The Classic picture is the machine's frame ([`GameView::classic`]); the
/// Enhanced one is an `ssd_hd` picture. The Classic picture is drawn with the nearest pixel (its
/// pixels are the game's), the Enhanced one smoothed, so the art's anti-aliased edges are kept
/// when the window is not a whole multiple of the picture.
#[derive(Debug, Clone, Copy)]
pub struct GameView<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub pixel_aspect: f64,
    /// Whether the picture is drawn with linear filtering (see above).
    pub smooth: bool,
    /// Where the original window lies in the picture (x, y, width, height in its pixels), when the picture is
    /// wider than it: the 4:3 view shows only this part.
    pub logical: Option<[u32; 4]>,
    /// The serial of the picture (see [`ssd_hd::world::SharedHd`]), when the view names one: a texture that holds
    /// the picture of this serial already is not uploaded again.
    pub serial: Option<u64>,
}

impl<'a> GameView<'a> {
    /// The machine's frame as the PPU drew it (the Classic picture).
    #[must_use]
    pub fn classic(frame: &Frame<'a>) -> Self {
        Self {
            rgba: bytemuck::cast_slice(frame.pixels),
            width: frame.size.width,
            height: frame.size.height,
            pixel_aspect: pixel_aspect(frame),
            smooth: false,
            logical: None,
            serial: None,
        }
    }

    /// The view names its picture by `serial` (see [`GameView::serial`]).
    #[must_use]
    pub fn with_serial(self, serial: u64) -> Self {
        Self { serial: Some(serial), ..self }
    }

    /// The Enhanced picture `picture`, shown with the pixel aspect of `frame`, the machine's frame
    /// it was made from: the picture keeps the proportions of the Classic one. `smooth` draws it
    /// with linear filtering (the Video page's smooth look); the crisp look keeps every pixel as
    /// it is.
    #[must_use]
    pub fn enhanced(picture: &'a HdFrame, frame: &Frame<'_>, smooth: bool) -> Self {
        Self {
            rgba: &picture.rgba,
            width: picture.width,
            height: picture.height,
            pixel_aspect: pixel_aspect(frame),
            smooth,
            logical: Some([
                picture.logical.x,
                picture.logical.y,
                picture.logical.width,
                picture.logical.height,
            ]),
            serial: None,
        }
    }
}

/// The width over height of one pixel of the frame, 1 when the frame does not say.
fn pixel_aspect(frame: &Frame<'_>) -> f64 {
    match frame.options.pixel_aspect_ratio {
        Some(ratio) => ratio.get(),
        None => 1.0,
    }
}

/// The texture of one game view, uploaded with its frame each time it is drawn.
struct GameTexture {
    texture: wgpu::Texture,
    id: egui::TextureId,
    width: u32,
    height: u32,
    smooth: bool,
    /// The serial of the picture the texture was last uploaded with (`None`: a picture without one).
    serial: Option<u64>,
}

/// One run of the interface: tessellated, waiting to be painted (or discarded).
pub struct Pass {
    /// What egui asks of the system (cursor, clipboard, text input); a window carries it out.
    pub platform: egui::PlatformOutput,
    jobs: Vec<egui::ClippedPrimitive>,
    pixels_per_point: f32,
    /// Textures egui no longer uses: freed once the pass is painted or discarded.
    free: Vec<egui::TextureId>,
}

/// The egui context, the egui-wgpu renderer and the game textures, on one device.
pub struct Canvas {
    egui: egui::Context,
    renderer: egui_wgpu::Renderer,
    /// One texture per game view (the frame of the run, and the two of a verification).
    games: Vec<Option<GameTexture>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// The graphics backends to try, in order. Windows: DX12 (wgpu prefers Vulkan there, which shows wrong
/// colours on some AMD drivers). Android: `WGPU_BACKEND` if set (for example `gl`), else OpenGL ES alone on an
/// emulator (its Vulkan is a software driver that can take the process down), else Vulkan and then OpenGL ES
/// for a device whose Vulkan driver fails. Elsewhere the primary ones.
#[must_use]
pub fn backends_to_try() -> Vec<wgpu::Backends> {
    if cfg!(windows) {
        vec![wgpu::Backends::DX12]
    } else if cfg!(target_os = "android") {
        if let Some(forced) = wgpu::Backends::from_env() {
            vec![forced]
        } else if on_emulator() {
            vec![wgpu::Backends::GL]
        } else {
            vec![wgpu::Backends::VULKAN, wgpu::Backends::GL]
        }
    } else {
        vec![wgpu::Backends::PRIMARY]
    }
}

/// Whether the Android system is an emulator (the system properties of the emulator's hardware).
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
fn on_emulator() -> bool {
    fn property(name: &str) -> String {
        let Ok(name) = std::ffi::CString::new(name) else { return String::new() };
        let mut value = [0 as std::ffi::c_char; 92]; // PROP_VALUE_MAX
        // SAFETY: `name` is NUL-terminated and `value` has the room the call writes (PROP_VALUE_MAX bytes).
        let length = unsafe { libc::__system_property_get(name.as_ptr(), value.as_mut_ptr()) };
        let bytes: Vec<u8> = value.iter().take(length.max(0) as usize).map(|&c| c as u8).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
    let emulated = property("ro.kernel.qemu") == "1"
        || ["ro.hardware", "ro.boot.hardware", "ro.product.board"].iter().any(|name| {
            let value = property(name);
            value.contains("ranchu") || value.contains("goldfish")
        });
    log::info!("running on an emulator: {emulated}");
    emulated
}

#[cfg(not(target_os = "android"))]
fn on_emulator() -> bool {
    false
}

/// A wgpu instance on `backends`.
#[must_use]
pub fn instance_on(backends: wgpu::Backends) -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

/// The wgpu instance of the first backend to try.
#[must_use]
pub fn instance() -> wgpu::Instance {
    instance_on(backends_to_try().first().copied().unwrap_or(wgpu::Backends::PRIMARY))
}

/// A device and its queue on `adapter`, with the default features and limits.
///
/// # Errors
///
/// The adapter cannot create a device.
pub fn device(adapter: &wgpu::Adapter) -> anyhow::Result<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("ssd device"),
        required_features: wgpu::Features::empty(),
        required_limits: required_limits(adapter),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .map_err(|e| anyhow::anyhow!("creating the wgpu device: {e}"))
}

impl Canvas {
    /// A canvas that paints into textures of `format`. It must not be sRGB-aware: egui draws in
    /// gamma space, and a non-sRGB format gives the exact colours.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let renderer = egui_wgpu::Renderer::new(
            &device,
            format,
            egui_wgpu::RendererOptions {
                dithering: false,
                ..egui_wgpu::RendererOptions::default()
            },
        );
        let egui = egui::Context::default();
        crate::theme::install(&egui);
        Self { egui, renderer, games: Vec::new(), device, queue }
    }

    /// The egui context: what the interface knows between passes (focus, the memory of layouts).
    #[must_use]
    pub fn context(&self) -> &egui::Context {
        &self.egui
    }

    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// The largest texture side the device takes: egui's input must say so.
    #[must_use]
    pub fn max_texture_side(&self) -> usize {
        self.device.limits().max_texture_dimension_2d as usize
    }

    /// The texture of game view `slot`, uploaded with the pixels of `view`.
    fn game_image(&mut self, slot: usize, view: &GameView<'_>) -> GameImage {
        let (width, height, smooth) = (view.width, view.height, view.smooth);
        if self.games.len() <= slot {
            self.games.resize_with(slot + 1, || None);
        }
        let rebuild = self.games[slot]
            .as_ref()
            .is_none_or(|g| (g.width, g.height, g.smooth) != (width, height, smooth));
        if rebuild {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("game frame"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let filter = if smooth { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest };
            let id = match self.games[slot].take() {
                Some(old) => {
                    self.renderer.update_egui_texture_from_wgpu_texture(
                        &self.device,
                        &view,
                        filter,
                        old.id,
                    );
                    old.id
                }
                None => self.renderer.register_native_texture(&self.device, &view, filter),
            };
            self.games[slot] =
                Some(GameTexture { texture, id, width, height, smooth, serial: None });
        }
        let game = self.games[slot].as_mut().expect("created above");
        // A picture the texture holds already (the same serial) is not uploaded again
        if view.serial.is_none() || game.serial != view.serial {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &game.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                view.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
            game.serial = view.serial;
        }
        GameImage { id: game.id, width, height, pixel_aspect: view.pixel_aspect }
    }

    /// Runs one pass of the interface: the game views (each of `views` as a texture for `ui` to
    /// place, in order) and what `ui` builds from `raw`. The textures egui needs are uploaded now;
    /// the pass is painted with [`Canvas::paint`] or dropped with [`Canvas::discard`].
    pub fn run(
        &mut self,
        raw: egui::RawInput,
        views: &[GameView<'_>],
        mut ui: impl FnMut(&mut egui::Ui, &[GameImage]),
    ) -> Pass {
        let games: Vec<GameImage> =
            views.iter().enumerate().map(|(slot, view)| self.game_image(slot, view)).collect();
        let mut output = self.egui.run_ui(raw, |root| ui(root, &games));
        let platform = std::mem::take(&mut output.platform_output);
        // Taken out of the output and drained below: epaint asserts in debug builds that a
        // `TexturesDelta` is empty when dropped, and a pass that is not painted must not leave one.
        let mut textures = std::mem::take(&mut output.textures_delta);
        for (id, deltas) in textures.set.drain() {
            for delta in &deltas {
                self.renderer.update_texture(&self.device, &self.queue, id, delta);
            }
        }
        let free: Vec<egui::TextureId> = textures.free.drain().collect();
        let jobs = self.egui.tessellate(output.shapes, output.pixels_per_point);
        Pass { platform, jobs, pixels_per_point: output.pixels_per_point, free }
    }

    /// Paints a pass into `target`, `size` pixels, cleared to black, and submits it. The index of
    /// the submission says when the GPU has finished with it.
    pub fn paint(
        &mut self,
        pass: Pass,
        target: &wgpu::TextureView,
        size: [u32; 2],
    ) -> wgpu::SubmissionIndex {
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: pass.pixels_per_point,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ssd frame") });
        let extra = self.renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &pass.jobs,
            &screen,
        );
        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ssd pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.renderer.render(&mut rpass.forget_lifetime(), &pass.jobs, &screen);
        }
        let submitted =
            self.queue.submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
        self.release(pass.free);
        submitted
    }

    /// Drops a pass that is not painted (the surface is not there this time), freeing its
    /// textures all the same.
    pub fn discard(&mut self, pass: Pass) {
        self.release(pass.free);
    }

    /// Frees textures egui no longer uses.
    fn release(&mut self, free: Vec<egui::TextureId>) {
        for id in free {
            self.renderer.free_texture(&id);
        }
    }
}

/// The limits the device asks for: the defaults, except on Android, where an OpenGL ES adapter has no compute
/// stage and so the adapter's own limits are the ones it can give.
fn required_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    if cfg!(target_os = "android") { adapter.limits() } else { wgpu::Limits::default() }
}
