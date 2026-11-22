//! The studio's control material samples a small GPU rendering of the map.
//! The swapchain stays write-only and the renderer never reads pixels to the CPU.
use super::{map_pipelines, FrameData, MapGpu, ViewUniform, ATLAS_FORMAT};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct GlassPatch {
    /// Left, top, right, bottom as fractions of the map viewport.
    pub bounds: [f32; 4],
    pub pixels_per_point: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Material {
    bounds: [f32; 4],
    view_size: [f32; 2],
    pixels_per_point: f32,
    _pad: f32,
}

#[derive(PartialEq)]
struct Source {
    camera: ViewUniform,
    geometry: u64,
    colors: u64,
}

struct View {
    size: [u32; 2],
    image: wgpu::TextureView,
    camera: wgpu::Buffer,
    material: wgpu::Buffer,
    surface: wgpu::BindGroup,
    source: Option<Source>,
}

pub(super) struct Glass {
    map: wgpu::RenderPipeline,
    surface: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    views: HashMap<u32, View>,
}

fn buffer<T>(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: std::mem::size_of::<T>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn backing_size(w: f32, h: f32) -> [u32; 2] {
    let scale = (1.0 / 5.0_f32).min(480.0 / w.max(h).max(1.0));
    [
        (w * scale).ceil().max(1.0) as u32,
        (h * scale).ceil().max(1.0) as u32,
    ]
}

impl Glass {
    fn new(
        device: &wgpu::Device,
        map_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let (map, _) = map_pipelines(device, map_layout, ATLAS_FORMAT, 1);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("studio glass"),
            source: wgpu::ShaderSource::Wgsl(include_str!("glass.wgsl").into()),
        });
        let surface = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("studio glass material"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples.max(1),
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let layout = surface.get_bind_group_layout(0);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("studio glass sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            map,
            surface,
            layout,
            sampler,
            views: HashMap::new(),
        }
    }
}

impl View {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: [u32; 2],
    ) -> Self {
        let [width, height] = size;
        let image = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("studio glass map sample"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: ATLAS_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let camera = buffer::<ViewUniform>(device, "studio glass map camera");
        let material = buffer::<Material>(device, "studio glass panel geometry");
        let surface = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("studio glass surface"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&image),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: material.as_entire_binding(),
                },
            ],
        });
        Self {
            size,
            image,
            camera,
            material,
            surface,
            source: None,
        }
    }
}

impl MapGpu {
    pub fn prepare_glass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameData,
        patch: GlassPatch,
    ) {
        let [w, h, _, _] = frame.uniform.viewport;
        let size = backing_size(w, h);
        let glass = self
            .glass
            .get_or_insert_with(|| Glass::new(device, &self.bgl, self.target_format, self.samples));
        if glass
            .views
            .get(&frame.view_id)
            .is_none_or(|view| view.size != size)
        {
            glass.views.insert(
                frame.view_id,
                View::new(device, &glass.layout, &glass.sampler, size),
            );
        }
        let view = glass
            .views
            .get_mut(&frame.view_id)
            .expect("studio glass viewport");
        queue.write_buffer(
            &view.material,
            0,
            bytemuck::bytes_of(&Material {
                bounds: patch.bounds,
                view_size: [w, h],
                pixels_per_point: patch.pixels_per_point,
                _pad: 0.0,
            }),
        );
        let source = Source {
            camera: frame.uniform,
            geometry: frame.geometry_gen,
            colors: frame.colors.0,
        };
        if view.source.as_ref() == Some(&source) {
            return;
        }
        view.source = Some(source);
        let mut camera = frame.uniform;
        let scale = size[0] as f32 / w.max(1.0);
        camera.viewport[0] *= scale;
        camera.viewport[1] *= scale;
        camera.center[2] *= scale;
        queue.write_buffer(&view.camera, 0, bytemuck::bytes_of(&camera));
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("studio glass map source"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: view.camera.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.colors.buf.as_entire_binding(),
                },
            ],
        });
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: &view.image,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.16,
                    g: 0.18,
                    b: 0.21,
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("studio glass map sample"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(&glass.map);
        pass.set_bind_group(0, &bind, &[]);
        pass.set_vertex_buffer(0, self.ocean.slice(..));
        pass.draw(0..self.ocean_count, 0..1);
        for chunk in self.fill.values().filter(|chunk| chunk.len > 0) {
            pass.set_vertex_buffer(0, chunk.buf.slice(..));
            pass.draw(0..chunk.len, 0..1);
        }
    }

    pub fn paint_glass(&self, pass: &mut wgpu::RenderPass<'_>, view_id: u32) {
        let Some(glass) = &self.glass else { return };
        let Some(view) = glass.views.get(&view_id) else {
            return;
        };
        pass.set_pipeline(&glass.surface);
        pass.set_bind_group(0, &view.surface, &[]);
        pass.draw(0..6, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_snapshot_is_small_at_large_window_sizes() {
        assert_eq!(backing_size(1500.0, 900.0), [300, 180]);
        assert_eq!(backing_size(7680.0, 3840.0), [480, 240]);
    }
}
