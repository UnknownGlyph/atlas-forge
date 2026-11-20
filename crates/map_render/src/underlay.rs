//! Reference images drawn under (or over the fills of) the map. See
//! `underlay.wgsl`: each fragment is unprojected and looked up through the
//! image's georeference, so alignment is exact in every projection.

use crate::camera::{Camera2D, GlobeCamera};
use bytemuck::{Pod, Zeroable};
use map_core::underlay::Georef;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Decoded RGBA8 pixels (straight alpha).
#[derive(Debug)]
pub struct RgbaImage {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
}

/// Halve an image with a 2×2 box filter (odd edges clamp).
pub fn half(img: &RgbaImage) -> RgbaImage {
    let (w, h) = ((img.w / 2).max(1), (img.h / 2).max(1));
    let mut out = vec![0u8; (w * h * 4) as usize];
    let px = |x: u32, y: u32, c: usize| img.pixels[((y.min(img.h - 1) * img.w + x.min(img.w - 1)) * 4) as usize + c] as u32;
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let s = px(2 * x, 2 * y, c) + px(2 * x + 1, 2 * y, c) + px(2 * x, 2 * y + 1, c) + px(2 * x + 1, 2 * y + 1, c);
                out[((y * w + x) * 4) as usize + c] = ((s + 2) / 4) as u8;
            }
        }
    }
    RgbaImage { w, h, pixels: out }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct UnderlayUniform {
    pub inv_view_proj: [[f32; 4]; 4],
    pub center: [f32; 4],
    pub viewport: [f32; 4],
    pub light: [f32; 4],
    pub m0: [f32; 4],
    pub m1: [f32; 4],
    pub src: [f32; 4],
}

impl UnderlayUniform {
    /// `size` is the image's pixel size the georeference is expressed in.
    fn base(g: &Georef, size: [u32; 2], opacity: f32) -> Self {
        let mut u = Self::zeroed();
        let (w, h) = (size[0].max(1) as f64, size[1].max(1) as f64);
        u.m0 = [(g.m[0][0] / w) as f32, (g.m[0][1] / w) as f32, (g.m[0][2] / w) as f32, 0.0];
        u.m1 = [(g.m[1][0] / h) as f32, (g.m[1][1] / h) as f32, (g.m[1][2] / h) as f32, 0.0];
        u.src = [g.proj.gpu_id() as f32, g.lon0 as f32, opacity, 0.0];
        u.inv_view_proj = glam::Mat4::IDENTITY.to_cols_array_2d();
        u
    }

    pub fn for_2d(g: &Georef, size: [u32; 2], opacity: f32, cam: &Camera2D, w: f32, h: f32) -> Self {
        let mut u = Self::base(g, size, opacity);
        u.center = [cam.center[0] as f32, cam.center[1] as f32, cam.zoom as f32, cam.lon0 as f32];
        u.viewport = [w, h, 0.0, cam.proj.gpu_id() as f32];
        u
    }

    pub fn for_globe(g: &Georef, size: [u32; 2], opacity: f32, cam: &GlobeCamera, w: f32, h: f32) -> Self {
        let mut u = Self::base(g, size, opacity);
        u.inv_view_proj = cam.view_proj(w as f64, h as f64).inverse().as_mat4().to_cols_array_2d();
        u.viewport = [w, h, 1.0, 0.0];
        let l = map_core::projection::sphere_pos(cam.center_lon - 35.0, cam.center_lat + 30.0);
        u.light = [l[0] as f32, l[1] as f32, l[2] as f32, 0.0];
        u
    }

    /// The interactive globe samples already premultiplied map layers.
    pub(crate) fn for_globe_raster(view: &crate::gpu::ViewUniform, lit: bool) -> Self {
        let mut u = Self::zeroed();
        u.inv_view_proj = glam::Mat4::from_cols_array_2d(&view.view_proj).inverse().to_cols_array_2d();
        u.viewport = view.viewport;
        u.light = view.light;
        u.src = [0.0, view.center[3], 1.0, if lit { 1.0 } else { 2.0 }];
        u.m0 = [1.0 / std::f32::consts::TAU, 0.0, 0.5, 0.0];
        u.m1 = [0.0, -1.0 / std::f32::consts::PI, 0.5, 0.0];
        u
    }
}

/// One underlay in one frame.
#[derive(Clone)]
pub struct UnderlayFrame {
    /// Identity of the image (textures are uploaded once per key).
    pub key: u64,
    pub image: Arc<RgbaImage>,
    pub uniform: UnderlayUniform,
    pub above_land: bool,
}

pub(crate) struct Slot {
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

impl Slot {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, texture: &wgpu::TextureView, sampler: &wgpu::Sampler) -> Self {
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("underlay uniform"),
            size: std::mem::size_of::<UnderlayUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("underlay bind"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(texture) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        });
        Self { ubuf, bind }
    }
}

pub(crate) struct UnderlayGpu {
    pipe: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: HashMap<u64, wgpu::TextureView>,
    /// (view id, key) → uniform buffer + bind group.
    slots: HashMap<(u32, u64), Slot>,
    /// Keys drawn by each view in its latest frame, in draw order.
    pub(crate) drawn: HashMap<u32, Vec<(u64, bool)>>,
}

impl UnderlayGpu {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, msaa: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("underlay.wgsl"), source: wgpu::ShaderSource::Wgsl(include_str!("underlay.wgsl").into()) });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("underlay bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("underlay layout"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("underlay"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: msaa.max(1), mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("underlay sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        Self { pipe, bgl, sampler, textures: HashMap::new(), slots: HashMap::new(), drawn: HashMap::new() }
    }

    fn upload(device: &wgpu::Device, queue: &wgpu::Queue, img: &RgbaImage) -> wgpu::TextureView {
        let levels = 32 - img.w.max(img.h).max(1).leading_zeros();
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("underlay"),
            size: wgpu::Extent3d { width: img.w, height: img.h, depth_or_array_layers: 1 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut owned: Option<RgbaImage> = None;
        for level in 0..levels {
            let cur = owned.as_ref().unwrap_or(img);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &cur.pixels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(cur.w * 4), rows_per_image: Some(cur.h) },
                wgpu::Extent3d { width: cur.w, height: cur.h, depth_or_array_layers: 1 },
            );
            if level + 1 < levels {
                owned = Some(half(cur));
            }
        }
        tex.create_view(&Default::default())
    }

    pub(crate) fn texture_slot(&self, device: &wgpu::Device, texture: &wgpu::TextureView, sampler: &wgpu::Sampler) -> Slot {
        Slot::new(device, &self.bgl, texture, sampler)
    }

    pub(crate) fn set_uniform(slot: &Slot, queue: &wgpu::Queue, uniform: &UnderlayUniform) {
        queue.write_buffer(&slot.ubuf, 0, bytemuck::bytes_of(uniform));
    }

    pub(crate) fn paint_slot(&self, pass: &mut wgpu::RenderPass<'_>, slot: &Slot) {
        pass.set_pipeline(&self.pipe);
        pass.set_bind_group(0, &slot.bind, &[]);
        pass.draw(0..3, 0..1);
    }

    pub(crate) fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, view_id: u32, frames: &[UnderlayFrame]) {
        for f in frames {
            if !self.textures.contains_key(&f.key) {
                let view = Self::upload(device, queue, &f.image);
                self.textures.insert(f.key, view);
                // bind groups made for an older texture with this key are stale
                self.slots.retain(|(_, k), _| *k != f.key);
            }
            let slot = self.slots.entry((view_id, f.key)).or_insert_with(|| {
                Slot::new(device, &self.bgl, &self.textures[&f.key], &self.sampler)
            });
            Self::set_uniform(slot, queue, &f.uniform);
        }
        self.drawn.insert(view_id, frames.iter().map(|f| (f.key, f.above_land)).collect());
        // free what no view draws any more
        let live: HashSet<u64> = self.drawn.values().flatten().map(|d| d.0).collect();
        self.textures.retain(|k, _| live.contains(k));
        self.slots.retain(|(v, k), _| self.drawn.get(v).is_some_and(|d| d.iter().any(|x| x.0 == *k)));
    }

    pub(crate) fn paint(&self, pass: &mut wgpu::RenderPass<'_>, view_id: u32, above_land: bool) {
        let Some(d) = self.drawn.get(&view_id) else { return };
        for (key, above) in d {
            if *above != above_land {
                continue;
            }
            if let Some(s) = self.slots.get(&(view_id, *key)) {
                self.paint_slot(pass, s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn half_averages_and_handles_odd_sizes() {
        let img = RgbaImage { w: 3, h: 1, pixels: vec![0, 0, 0, 255, 100, 100, 100, 255, 200, 200, 200, 255] };
        let h = half(&img);
        assert_eq!((h.w, h.h), (1, 1));
        assert_eq!(h.pixels, vec![50, 50, 50, 255]);
    }
}
