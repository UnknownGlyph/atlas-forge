//! wgpu resources and draw calls. UI-framework agnostic: the app wraps
//! [`MapGpu::prepare`] / [`MapGpu::paint`] in its own paint callback.

mod glass;
pub use glass::GlassPatch;

use crate::camera::{Camera2D, GlobeCamera};
use crate::tess::{self, Chunk, ChunkId, FillVertex, LineInstance};
use crate::underlay::{Slot, UnderlayGpu, UnderlayUniform};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct ViewUniform {
    pub view_proj: [[f32; 4]; 4],
    pub cam_pos: [f32; 4],
    pub center: [f32; 4],
    pub viewport: [f32; 4],
    pub light: [f32; 4],
    pub line_color: [[f32; 4]; LINE_CLASSES],
    pub line_width: [[f32; 4]; LINE_CLASSES],
}

/// Room for this many classes of lines (`tess::class`).
pub const LINE_CLASSES: usize = 15;

#[derive(Clone, Copy, Debug)]
pub struct LineStyle {
    pub color: [f32; 4],
    pub width: f32,
}

impl ViewUniform {
    pub fn for_2d(cam: &Camera2D, w: f32, h: f32, styles: &[LineStyle]) -> Self {
        let mut u = Self::base(styles);
        u.center = [cam.center[0] as f32, cam.center[1] as f32, cam.zoom as f32, cam.lon0 as f32];
        u.viewport = [w, h, 0.0, cam.proj.gpu_id() as f32];
        u
    }

    pub fn for_globe(cam: &GlobeCamera, lon0: f64, w: f32, h: f32, styles: &[LineStyle]) -> Self {
        let mut u = Self::base(styles);
        u.view_proj = cam.view_proj(w as f64, h as f64).as_mat4().to_cols_array_2d();
        let e = cam.eye();
        u.cam_pos = [e.x as f32, e.y as f32, e.z as f32, 0.0];
        u.center = [0.0, 0.0, 1.0, lon0 as f32];
        u.viewport = [w, h, 1.0, 0.0];
        // light from the upper-left of the viewer
        let l = map_core::projection::sphere_pos(cam.center_lon - 35.0, cam.center_lat + 30.0);
        // (w: the outline's radius, past which the coast bands are cut)
        u.light = [l[0] as f32, l[1] as f32, l[2] as f32, cam.disc_px(w as f64, h as f64) as f32];
        u
    }

    fn base(styles: &[LineStyle]) -> Self {
        let mut u = Self::zeroed();
        u.view_proj = glam::Mat4::IDENTITY.to_cols_array_2d();
        for (i, s) in styles.iter().enumerate().take(LINE_CLASSES) {
            u.line_color[i] = s.color;
            u.line_width[i] = [s.width, 0.0, 0.0, 0.0];
        }
        u
    }
}

/// Data for one frame of one view. Buffers are only uploaded when their
/// generation differs from what the GPU already has.
#[derive(Clone)]
pub struct FrameData {
    pub view_id: u32,
    pub uniform: ViewUniform,
    /// All fill chunks currently in the scene (chunks absent here are freed).
    pub fill: Vec<Chunk<FillVertex>>,
    /// All line chunks currently in the scene.
    pub lines: Vec<Chunk<LineInstance>>,
    pub colors: (u64, Arc<Vec<[f32; 4]>>),
    pub overlay: (u64, Arc<Vec<LineInstance>>),
    /// Bands along the coasts, drawn over the sea and under the land.
    pub glow: (u64, Arc<Vec<LineInstance>>),
    pub graticule: bool,
    pub draw_overlay: bool,
    /// Reference images, bottom first.
    pub underlays: Vec<crate::underlay::UnderlayFrame>,
    pub geometry_gen: u64,
    pub globe_detail: GlobeDetail,
}

/// A moving globe can sample cached surface layers. The full geometry still
/// supplies the stationary view, editing and high magnification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlobeDetail {
    #[default]
    Full,
    Prewarm,
    Moving,
}

const ATLAS_WIDTH: u32 = 2048;
const ATLAS_HEIGHT: u32 = ATLAS_WIDTH / 2;
const ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Limit the raster path to zooms where its angular resolution is useful.
pub fn globe_raster_zoom(pixels_per_rad: f64) -> bool {
    pixels_per_rad <= f64::from(ATLAS_WIDTH) / std::f64::consts::TAU * 3.0
}

struct DynBuf {
    buf: wgpu::Buffer,
    cap: u64,
    len: u32,
    gen: u64,
    serial: u64,
}

impl DynBuf {
    fn new(device: &wgpu::Device, usage: wgpu::BufferUsages, label: &str) -> Self {
        let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: 256, usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        Self { buf, cap: 256, len: 0, gen: u64::MAX, serial: 0 }
    }

    fn upload<T: Pod>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, gen: u64, data: &[T], usage: wgpu::BufferUsages) {
        if gen == self.gen {
            return;
        }
        let bytes: &[u8] = bytemuck::cast_slice(data);
        let need = (bytes.len() as u64).max(16).next_multiple_of(4);
        if need > self.cap {
            let cap = need.next_power_of_two();
            self.buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: cap, usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            self.cap = cap;
            self.serial += 1;
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.buf, 0, bytes);
        }
        self.len = data.len() as u32;
        self.gen = gen;
    }
}

/// Upload changed chunks, create buffers for new ones, free removed ones.
fn sync_chunks<T: Pod>(bufs: &mut HashMap<ChunkId, DynBuf>, chunks: &[Chunk<T>], device: &wgpu::Device, queue: &wgpu::Queue, label: &str) {
    let v = wgpu::BufferUsages::VERTEX;
    if bufs.len() != chunks.len() || chunks.iter().any(|c| !bufs.contains_key(&c.id)) {
        let live: std::collections::HashSet<ChunkId> = chunks.iter().map(|c| c.id).collect();
        bufs.retain(|id, _| live.contains(id));
    }
    for c in chunks {
        bufs.entry(c.id).or_insert_with(|| DynBuf::new(device, v, label)).upload(device, queue, c.gen, &c.data, v);
    }
}

struct ViewRes {
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    color_serial: u64,
    graticule: bool,
    draw_overlay: bool,
    moving: bool,
}

#[derive(PartialEq)]
struct AtlasKey {
    geometry: u64,
    colors: u64,
    glow: u64,
    line_color: [[f32; 4]; LINE_CLASSES],
    line_width: [[f32; 4]; LINE_CLASSES],
}

struct AtlasLayer {
    view: wgpu::TextureView,
    slot: Slot,
}

impl AtlasLayer {
    fn target(device: &wgpu::Device, underlay: &UnderlayGpu, sampler: &wgpu::Sampler) -> Self {
        let view = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("globe motion layer"),
            size: wgpu::Extent3d { width: ATLAS_WIDTH, height: ATLAS_HEIGHT, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ATLAS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }).create_view(&Default::default());
        let slot = underlay.texture_slot(device, &view, sampler);
        Self { view, slot }
    }
}

struct MotionAtlas {
    fills: AtlasLayer,
    lines: AtlasLayer,
    glow: Option<AtlasLayer>,
    sampler: wgpu::Sampler,
    fill_pipe: wgpu::RenderPipeline,
    line_pipe: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    key: Option<AtlasKey>,
}

pub struct MapGpu {
    fill_pipe: wgpu::RenderPipeline,
    line_pipe: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    ocean: wgpu::Buffer,
    ocean_count: u32,
    grat: wgpu::Buffer,
    grat_count: u32,
    fill: HashMap<ChunkId, DynBuf>,
    lines: HashMap<ChunkId, DynBuf>,
    overlay: DynBuf,
    glow: DynBuf,
    colors: DynBuf,
    views: HashMap<u32, ViewRes>,
    underlay: crate::underlay::UnderlayGpu,
    motion_atlas: Option<MotionAtlas>,
    glass: Option<glass::Glass>,
    target_format: wgpu::TextureFormat,
    samples: u32,
}

fn map_pipelines(device: &wgpu::Device, bgl: &wgpu::BindGroupLayout, format: wgpu::TextureFormat, msaa: u32) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("map.wgsl"), source: wgpu::ShaderSource::Wgsl(include_str!("map.wgsl").into()) });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("map layout"), bind_group_layouts: &[Some(bgl)], immediate_size: 0 });
    let target = [Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })];
    let ms = wgpu::MultisampleState { count: msaa.max(1), mask: !0, alpha_to_coverage_enabled: false };
    let fill_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Uint32];
    let fill_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("map fill"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_fill"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<FillVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &fill_attrs })],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: ms,
        fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_fill"), compilation_options: Default::default(), targets: &target }),
        multiview_mask: None,
        cache: None,
    });
    let line_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32];
    let line_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("map lines"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_line"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<LineInstance>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &line_attrs })],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: ms,
        fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_line"), compilation_options: Default::default(), targets: &target }),
        multiview_mask: None,
        cache: None,
    });
    (fill_pipe, line_pipe)
}

impl MapGpu {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, msaa: u32) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let (fill_pipe, line_pipe) = map_pipelines(device, &bgl, format, msaa);
        let ocean_data = tess::ocean_mesh(5.0);
        let ocean = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("ocean"), contents: bytemuck::cast_slice(&ocean_data), usage: wgpu::BufferUsages::VERTEX });
        let grat_data = tess::graticule(15.0);
        let grat = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("graticule"), contents: bytemuck::cast_slice(&grat_data), usage: wgpu::BufferUsages::VERTEX });
        let v = wgpu::BufferUsages::VERTEX;
        Self {
            fill_pipe,
            line_pipe,
            bgl,
            ocean,
            ocean_count: ocean_data.len() as u32,
            grat,
            grat_count: grat_data.len() as u32,
            fill: HashMap::new(),
            lines: HashMap::new(),
            overlay: DynBuf::new(device, v, "overlay"),
            glow: DynBuf::new(device, v, "coast glow"),
            colors: DynBuf::new(device, wgpu::BufferUsages::STORAGE, "colors"),
            views: HashMap::new(),
            underlay: crate::underlay::UnderlayGpu::new(device, format, msaa),
            motion_atlas: None,
            glass: None,
            target_format: format,
            samples: msaa,
        }
    }

    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, f: &FrameData) {
        let v = wgpu::BufferUsages::VERTEX;
        sync_chunks(&mut self.fill, &f.fill, device, queue, "fill chunk");
        sync_chunks(&mut self.lines, &f.lines, device, queue, "line chunk");
        self.overlay.upload(device, queue, f.overlay.0, &f.overlay.1, v);
        self.glow.upload(device, queue, f.glow.0, &f.glow.1, v);
        self.colors.upload(device, queue, f.colors.0, &f.colors.1, wgpu::BufferUsages::STORAGE);
        self.underlay.prepare(device, queue, f.view_id, &f.underlays);
        let serial = self.colors.serial;
        let needs_new = self.views.get(&f.view_id).is_none_or(|v| v.color_serial != serial);
        if needs_new {
            let ubuf = match self.views.remove(&f.view_id) {
                Some(v) => v.ubuf,
                None => device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("view uniform"),
                    size: std::mem::size_of::<ViewUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
            };
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("map bind"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: self.colors.buf.as_entire_binding() },
                ],
            });
            self.views.insert(f.view_id, ViewRes { ubuf, bind, color_serial: serial, graticule: f.graticule, draw_overlay: f.draw_overlay, moving: false });
        }
        let moving = self.prepare_motion(device, queue, encoder, f);
        let vr = self.views.get_mut(&f.view_id).unwrap();
        vr.graticule = f.graticule;
        vr.draw_overlay = f.draw_overlay;
        vr.moving = moving;
        queue.write_buffer(&vr.ubuf, 0, bytemuck::bytes_of(&f.uniform));
    }

    fn prepare_motion(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, f: &FrameData) -> bool {
        if f.globe_detail == GlobeDetail::Full {
            return false;
        }
        let key = AtlasKey {
            geometry: f.geometry_gen, colors: f.colors.0, glow: f.glow.0,
            line_color: f.uniform.line_color, line_width: f.uniform.line_width,
        };
        if f.globe_detail == GlobeDetail::Prewarm {
            let atlas = self.motion_atlas.get_or_insert_with(|| {
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("globe motion sampler"),
                    address_mode_u: wgpu::AddressMode::Repeat,
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                });
                let (fill_pipe, line_pipe) = map_pipelines(device, &self.bgl, ATLAS_FORMAT, 1);
                MotionAtlas {
                    fills: AtlasLayer::target(device, &self.underlay, &sampler),
                    lines: AtlasLayer::target(device, &self.underlay, &sampler),
                    glow: None, sampler, fill_pipe, line_pipe,
                    uniform: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("motion atlas uniform"), size: std::mem::size_of::<ViewUniform>() as u64,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    key: None,
                }
            });
            if atlas.key.as_ref() != Some(&key) {
                if self.glow.len > 0 && atlas.glow.is_none() {
                    atlas.glow = Some(AtlasLayer::target(device, &self.underlay, &atlas.sampler));
                } else if self.glow.len == 0 {
                    atlas.glow = None;
                }
                let mut view = f.uniform;
                view.viewport = [ATLAS_WIDTH as f32, ATLAS_HEIGHT as f32, 0.0, 0.0];
                view.center = [0.0, 0.0, ATLAS_WIDTH as f32 / std::f32::consts::TAU, 0.0];
                for width in &mut view.line_width {
                    width[0] *= 0.65;
                }
                queue.write_buffer(&atlas.uniform, 0, bytemuck::bytes_of(&view));
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("motion atlas bind"),
                    layout: &self.bgl,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: atlas.uniform.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: self.colors.buf.as_entire_binding() },
                    ],
                });
                bake_layer(encoder, &atlas.fills, &atlas.fill_pipe, &bind, self.fill.values(), |pass, b| pass.draw(0..b.len, 0..1));
                bake_layer(encoder, &atlas.lines, &atlas.line_pipe, &bind, self.lines.values(), |pass, b| pass.draw(0..6, 0..b.len));
                if let Some(glow) = &atlas.glow {
                    bake_layer(encoder, glow, &atlas.line_pipe, &bind, std::iter::once(&self.glow), |pass, b| pass.draw(0..6, 0..b.len));
                }
                atlas.key = Some(key);
            }
            return false;
        }
        let Some(atlas) = self.motion_atlas.as_ref().filter(|a| a.key.as_ref() == Some(&key)) else { return false };
        let lit = UnderlayUniform::for_globe_raster(&f.uniform, true);
        let unlit = UnderlayUniform::for_globe_raster(&f.uniform, false);
        UnderlayGpu::set_uniform(&atlas.fills.slot, queue, &lit);
        UnderlayGpu::set_uniform(&atlas.lines.slot, queue, &unlit);
        if let Some(glow) = &atlas.glow { UnderlayGpu::set_uniform(&glow.slot, queue, &unlit); }
        true
    }

    pub fn paint(&self, pass: &mut wgpu::RenderPass<'_>, view_id: u32) {
        let Some(vr) = self.views.get(&view_id) else { return };
        let atlas = self.motion_atlas.as_ref().filter(|_| vr.moving);
        pass.set_bind_group(0, &vr.bind, &[]);
        pass.set_pipeline(&self.fill_pipe);
        pass.set_vertex_buffer(0, self.ocean.slice(..));
        pass.draw(0..self.ocean_count, 0..1);
        self.underlay.paint(pass, view_id, false);
        pass.set_bind_group(0, &vr.bind, &[]);
        // the coast bands: over the sea, the land is drawn over their inner half
        if let Some(glow) = atlas.and_then(|a| a.glow.as_ref()) {
            self.underlay.paint_slot(pass, &glow.slot);
        } else if self.glow.len > 0 {
            pass.set_pipeline(&self.line_pipe);
            pass.set_vertex_buffer(0, self.glow.buf.slice(..));
            pass.draw(0..6, 0..self.glow.len);
        }
        if let Some(atlas) = atlas {
            self.underlay.paint_slot(pass, &atlas.fills.slot);
        } else {
            pass.set_pipeline(&self.fill_pipe);
            for b in self.fill.values().filter(|b| b.len > 0) {
                pass.set_vertex_buffer(0, b.buf.slice(..));
                pass.draw(0..b.len, 0..1);
            }
        }
        self.underlay.paint(pass, view_id, true);
        pass.set_bind_group(0, &vr.bind, &[]);
        pass.set_pipeline(&self.line_pipe);
        if vr.graticule {
            pass.set_vertex_buffer(0, self.grat.slice(..));
            pass.draw(0..6, 0..self.grat_count);
        }
        if let Some(atlas) = atlas {
            self.underlay.paint_slot(pass, &atlas.lines.slot);
        } else {
            for b in self.lines.values().filter(|b| b.len > 0) {
                pass.set_vertex_buffer(0, b.buf.slice(..));
                pass.draw(0..6, 0..b.len);
            }
        }
        if vr.draw_overlay && self.overlay.len > 0 {
            pass.set_bind_group(0, &vr.bind, &[]);
            pass.set_pipeline(&self.line_pipe);
            pass.set_vertex_buffer(0, self.overlay.buf.slice(..));
            pass.draw(0..6, 0..self.overlay.len);
        }
    }
}

fn bake_layer<'a>(
    encoder: &mut wgpu::CommandEncoder,
    layer: &AtlasLayer,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
    buffers: impl IntoIterator<Item = &'a DynBuf>,
    draw: impl Fn(&mut wgpu::RenderPass<'_>, &DynBuf),
) {
    let attachments = [Some(wgpu::RenderPassColorAttachment {
        view: &layer.view, depth_slice: None, resolve_target: None,
        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
    })];
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("prepare globe motion texture"), color_attachments: &attachments, ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    for b in buffers.into_iter().filter(|b| b.len > 0) {
        pass.set_vertex_buffer(0, b.buf.slice(..));
        draw(&mut pass, b);
    }
}
