//! wgpu renderer for the island.
//!
//! Draws a single full-screen triangle whose fragment shader evaluates the
//! island SDF (see `island.wgsl`). The surface is configured for alpha
//! compositing with the desktop so everything outside the island is fully
//! transparent.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use winit::window::Window;

use crate::art::ArtRenderer;
use crate::content::ContentRenderer;
use crate::dashboard::DashData;
use crate::state::{Layout, NowPlaying, Transient};
use crate::text::TextLayer;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    pub resolution: [f32; 2],
    pub center: [f32; 2],
    pub half_size: [f32; 2],
    pub radius: f32,
    pub premultiply: f32,
    pub shadow_radius: f32,
    pub shadow_alpha: f32,
    pub bg_phase: f32,
    pub bg_intensity: f32,
    pub bg_color0: [f32; 4],
    pub bg_color1: [f32; 4],
}

impl Default for Uniforms {
    fn default() -> Self {
        Self {
            resolution: [460.0, 320.0],
            center: [230.0, 40.0],
            half_size: [75.0, 18.5],
            radius: 18.5,
            premultiply: 1.0,
            shadow_radius: 28.0,
            shadow_alpha: 0.33,
            bg_phase: 0.0,
            bg_intensity: 0.35,
            bg_color0: [0.40, 0.20, 0.70, 1.0],
            bg_color1: [0.20, 0.55, 0.80, 1.0],
        }
    }
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    premultiply: bool,
    content: ContentRenderer,
    art: ArtRenderer,
    text: TextLayer,
    pub uniforms: Uniforms,
}

impl Renderer {
    pub fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        // DX12 only reports transparent alpha modes when the swapchain is built
        // from a DirectComposition visual rather than straight off the HWND.
        // `DxgiFromVisual` makes wgpu create that visual from our window for us.
        let mut backend_options = wgpu::BackendOptions::default();
        backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options,
            display: None,
        });

        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("request adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("winland-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        }))
        .expect("request device");

        log::info!("adapter: {} ({:?})", adapter.get_info().name, adapter.get_info().device_type);

        let caps = surface.get_capabilities(&adapter);

        // Pick an alpha mode that lets us composite with the desktop.
        let alpha_mode = if caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else if caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PostMultiplied)
        {
            wgpu::CompositeAlphaMode::PostMultiplied
        } else {
            caps.alpha_modes[0]
        };
        let premultiply = matches!(alpha_mode, wgpu::CompositeAlphaMode::PreMultiplied);
        log::info!(
            "surface alpha modes: {:?}, chose {:?}",
            caps.alpha_modes,
            alpha_mode
        );

        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("island-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("island.wgsl").into()),
        });

        let mut uniforms = Uniforms::default();
        uniforms.resolution = [width as f32, height as f32];
        uniforms.premultiply = if premultiply { 1.0 } else { 0.0 };

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("island-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("island-bg"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("island-pl"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("island-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // We emit premultiplied color for a premultiplied surface.
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let content = ContentRenderer::new(&device, format);
        let art = ArtRenderer::new(&device, format, premultiply);
        let text = TextLayer::new(&device, &queue, format);

        Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            uniform_buf,
            bind_group,
            premultiply,
            content,
            art,
            text,
            uniforms,
        }
    }

    pub fn set_album_art(&mut self, rgba: &[u8], w: u32, h: u32) {
        self.art.set_art(&self.device, &self.queue, rgba, w, h);
    }

    pub fn clear_album_art(&mut self) {
        self.art.clear();
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.uniforms.resolution = [width as f32, height as f32];
    }

    pub fn render(
        &mut self,
        layout: &Layout,
        np: Option<&NowPlaying>,
        transient: Option<&Transient>,
        dash: Option<&DashData>,
        viz: &[f32],
        time: f32,
    ) {
        // Drive the island SDF from the current layout.
        self.uniforms.center = layout.center;
        self.uniforms.half_size = layout.half;
        self.uniforms.radius = layout.radius;
        self.uniforms.premultiply = if self.premultiply { 1.0 } else { 0.0 };
        self.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&self.uniforms));

        self.content.prepare(
            &self.queue,
            self.uniforms.resolution,
            self.premultiply,
            layout,
            np,
            transient,
            dash,
            viz,
            time,
        );
        self.text.prepare(
            &self.device,
            &self.queue,
            self.config.width,
            self.config.height,
            layout,
            np,
            transient,
            dash,
            time,
        );
        if transient.is_none() {
            self.art.prepare(&self.queue, self.uniforms.resolution, layout);
        }

        use wgpu::CurrentSurfaceTexture as Cst;
        let frame = match self.surface.get_current_texture() {
            Cst::Success(t) | Cst::Suboptimal(t) => t,
            Cst::Outdated | Cst::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            // Timeout / Occluded / Validation: skip this frame and try again.
            _ => return,
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("island-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Clear to fully transparent so only the island shows.
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);

            // Content (placeholder art, waveform, progress) on the island.
            self.content.draw(&mut pass);
            // Real album-art texture over the placeholder.
            if transient.is_none() {
                self.art.draw(&mut pass);
            }
            // Text (title, artist, controls) on top of everything.
            self.text.render(&mut pass);
        }

        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
    }
}
