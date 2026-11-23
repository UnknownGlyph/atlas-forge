//! Offscreen replay of the application's globe renderer at a fixed size.
//! cargo run --release -p map_bench --bin globe_frames -- <world.atlas>
use map_render::camera::GlobeCamera;
use map_render::gpu::{FrameData, GlobeDetail, LineStyle, MapGpu, ViewUniform};
use map_render::tess::{self, SceneCache};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use wgpu::*;

struct Readback {
    texture: Texture,
    view: TextureView,
    pixels: Buffer,
    size: Extent3d,
    stride: u32,
}

impl Readback {
    fn new(device: &Device, format: TextureFormat, width: u32, height: u32) -> Self {
        let size = Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("globe verification image"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let stride = (width * 4).next_multiple_of(COPY_BYTES_PER_ROW_ALIGNMENT);
        let pixels = device.create_buffer(&BufferDescriptor {
            label: None,
            size: u64::from(stride) * u64::from(height),
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            texture,
            view,
            pixels,
            size,
            stride,
        }
    }

    fn copy(&self, encoder: &mut CommandEncoder) {
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &self.pixels,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.stride),
                    rows_per_image: None,
                },
            },
            self.size,
        );
    }

    fn read(&self, device: &Device) -> Vec<u8> {
        let (tx, rx) = std::sync::mpsc::channel();
        let slice = self.pixels.slice(..);
        slice.map_async(MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let view = slice.get_mapped_range().expect("mapped image");
        let bytes = view
            .chunks_exact(self.stride as usize)
            .flat_map(|row| row[..self.size.width as usize * 4].iter().copied())
            .collect();
        drop(view);
        self.pixels.unmap();
        bytes
    }
}

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(map_io::project::ithaer_test_project)
        .expect("usage: globe_frames <world.atlas>");
    let doc = map_io::project::load(&path).unwrap_or_else(|error| panic!("{error}"));
    let instance = Instance::new(InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
        power_preference: PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor::default()))
        .expect("GPU device");
    println!(
        "GPU: {} ({:?})",
        adapter.get_info().name,
        adapter.get_info().backend
    );
    let (w, h) = (1360, 960);
    let format = TextureFormat::Bgra8Unorm;
    let texture = device
        .create_texture(&TextureDescriptor {
            label: Some("globe replay"),
            size: Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 4,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let mut scene = SceneCache::default();
    scene.update_geometry(&doc, 0.0);
    scene.update_colors(
        doc.style.ocean,
        &map_core::theme::evaluate(&doc).face_colors,
        &Default::default(),
    );
    println!(
        "{}x{}, 4x MSAA, {} triangles, {} lines",
        w,
        h,
        scene.vertex_count() / 3,
        scene.line_count()
    );
    let colors = [
        (doc.style.coast, doc.style.coast_width),
        (doc.style.border, doc.style.border_width),
        (doc.style.internal, doc.style.internal_width),
        (doc.style.river, doc.style.river_width),
        ([255, 120, 0, 255], 2.6),
        ([20, 90, 220, 220], 1.8),
        ([255, 255, 255, 90], 0.8),
        (doc.style.overlay_color, doc.style.overlay_width),
        ([215, 35, 35, 255], 2.4),
        (doc.style.glow_bands()[0], doc.style.coast_glow_width * 2.0),
        (
            doc.style.glow_bands()[1],
            doc.style.coast_glow_width * 4.0 / 3.0,
        ),
        (
            doc.style.glow_bands()[2],
            doc.style.coast_glow_width * 2.0 / 3.0,
        ),
    ]
    .map(|(c, width)| LineStyle {
        color: tess::rgba(c),
        width,
    });
    let glow = if doc.style.coast_glow_width > 0.0 {
        tess::glow_lines(&scene.chunks().lines)
    } else {
        Vec::new()
    };
    let mut frame = FrameData {
        view_id: 1,
        uniform: ViewUniform::for_globe(&GlobeCamera::default(), 0.0, w as f32, h as f32, &colors),
        geometry: scene.chunks().clone(),
        colors: (scene.color_gen, scene.colors.clone()),
        overlay: (1, Arc::new(vec![])),
        glow: (1, Arc::new(glow)),
        graticule: true,
        draw_overlay: true,
        underlays: vec![],
        geometry_gen: scene.geometry_gen,
        globe_detail: GlobeDetail::Prewarm,
    };
    let mut gpu = MapGpu::new(&device, format, 4);
    let mut camera = GlobeCamera::default();
    let mut draw = |frame: &FrameData, readback: Option<&Readback>| {
        let mut encoder = device.create_command_encoder(&Default::default());
        gpu.prepare(&device, &queue, &mut encoder, frame);
        {
            let attachments = [Some(RenderPassColorAttachment {
                view: &texture,
                depth_slice: None,
                resolve_target: readback.map(|r| &r.view),
                ops: Operations {
                    load: LoadOp::Clear(Color::BLACK),
                    store: StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                color_attachments: &attachments,
                ..Default::default()
            });
            gpu.paint(&mut pass, frame.view_id);
        }
        if let Some(r) = readback {
            r.copy(&mut encoder);
        }
        queue.submit([encoder.finish()]);
        device
            .poll(PollType::wait_indefinitely())
            .expect("frame complete");
    };
    draw(&frame, None);
    for (label, detail) in [
        ("vector", GlobeDetail::Full),
        ("moving", GlobeDetail::Moving),
    ] {
        frame.globe_detail = detail;
        let mut times = Vec::new();
        for i in 0..88 {
            camera.center_lon = -180.0 + i as f64 * 360.0 / 88.0;
            camera.center_lat = 25.0 * (i as f64 / 12.0).sin();
            frame.uniform = ViewUniform::for_globe(&camera, 0.0, w as f32, h as f32, &colors);
            let t = Instant::now();
            draw(&frame, None);
            if i >= 8 {
                times.push(t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        times.sort_by(f64::total_cmp);
        println!(
            "{label} ({} frames): median {:.2} ms, p95 {:.2} ms, mean {:.2} ms",
            times.len(),
            times[times.len() / 2],
            times[times.len() * 95 / 100],
            times.iter().sum::<f64>() / times.len() as f64
        );
    }
    let readback = Readback::new(&device, format, w, h);
    camera.center_lon = 179.0;
    camera.center_lat = 23.0;
    frame.uniform = ViewUniform::for_globe(&camera, 0.0, w as f32, h as f32, &colors);
    let mut render_pixels = |frame: &mut FrameData, detail| {
        frame.globe_detail = detail;
        draw(frame, Some(&readback));
        readback.read(&device)
    };
    let full = render_pixels(&mut frame, GlobeDetail::Full);
    let moving = render_pixels(&mut frame, GlobeDetail::Moving);
    let error = full
        .iter()
        .zip(&moving)
        .map(|(&a, &b)| u64::from(a.abs_diff(b)))
        .sum::<u64>() as f64
        / full.len() as f64;
    assert!(
        error < 7.0,
        "moving globe does not match full geometry: {error:.2}"
    );
    let geometry = std::mem::take(&mut frame.geometry);
    let cleared = render_pixels(&mut frame, GlobeDetail::Full);
    assert!(cleared != full, "removed chunks still draw");
    frame.geometry = geometry;
    let restored = render_pixels(&mut frame, GlobeDetail::Full);
    assert!(restored == full, "restored chunks differ from the original rendering");

    frame.colors.0 += 1;
    frame.colors.1 = Arc::new(vec![[0.92, 0.18, 0.16, 1.0]; frame.colors.1.len()]);
    let invalidated = render_pixels(&mut frame, GlobeDetail::Moving);
    let updated = render_pixels(&mut frame, GlobeDetail::Full);
    let diagnostic = invalidated
        .iter()
        .zip(&updated)
        .map(|(&a, &b)| u64::from(a.abs_diff(b)))
        .sum::<u64>() as f64
        / updated.len() as f64;
    let mid = (h as usize / 2 * w as usize + w as usize / 2) * 4;
    assert!(
        diagnostic < 0.05,
        "stale colors were displayed: error {diagnostic:.4}, center {:?} vs {:?}",
        &invalidated[mid..mid + 4],
        &updated[mid..mid + 4]
    );
    println!("Date line screenshot error: {error:.2}/255 per channel; removed and restored chunks verified; stale colors correctly fall back to vectors");
}
