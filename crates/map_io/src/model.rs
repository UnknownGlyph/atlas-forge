//! Self-contained 3D mesh assets for the world book and star viewer.
use base64::Engine;
use glam::{Mat4, Vec3};
use map_core::{underlay::Blob, world_book::Attachment};
use std::{io::Read, path::Path, sync::Arc};

const MAX_INPUT: usize = 48 * 1024 * 1024;
const MAX_VERTICES: usize = 500_000;
const MAX_TRIANGLES: usize = 340_000;

#[derive(Clone)]
pub struct Model {
    /// Model space, centered and normalized to unit radius for framing.
    pub positions: Vec<Vec3>,
    pub triangles: Vec<Triangle>,
}

#[derive(Clone, Copy)]
pub struct Triangle {
    pub vertices: [u32; 3],
    pub color: [u8; 4],
}

pub fn is_model(asset: &Attachment) -> bool {
    asset.bytes.0.starts_with(b"glTF")
        || Path::new(&asset.name)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ["obj", "glb", "gltf"]
                    .iter()
                    .any(|known| ext.eq_ignore_ascii_case(known))
            })
}

/// Import the model and retain the original bytes for attachment export.
pub fn import(path: &Path) -> Result<(Attachment, Model), String> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("模型文件名无效。")?;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take((MAX_INPUT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let asset = Attachment {
        name: name.into(),
        bytes: Blob(Arc::new(bytes)),
    };
    let model = decode(&asset)?;
    Ok((asset, model))
}

pub fn decode(asset: &Attachment) -> Result<Model, String> {
    let bytes = &asset.bytes.0;
    if bytes.len() > MAX_INPUT {
        return Err("3D 模型超过 48 MiB。请先减少网格或贴图体积。".into());
    }
    let is_obj = Path::new(&asset.name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("obj"));
    let (mut positions, mut triangles) = if is_obj { obj(bytes)? } else { gltf(bytes)? };
    if positions
        .iter()
        .any(|p| !p.is_finite() || p.abs().max_element() > 1e18)
    {
        return Err("模型包含无效坐标。".into());
    }
    triangles.retain(|face| {
        let [a, b, c] = face.vertices.map(|i| positions[i as usize]);
        (b - a).cross(c - a).abs().max_element() > 0.0
    });
    if triangles.is_empty() {
        return Err("模型没有可查看的三角面。".into());
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for angle in triangles.iter().flat_map(|t| t.vertices) {
        let p = positions[angle as usize];
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let center = (lo + hi) * 0.5;
    let diameter = (hi - lo).abs().max_element();
    if diameter <= 0.0 {
        return Err("模型几何尺寸无效。".into());
    }
    let radius = triangles
        .iter()
        .flat_map(|t| t.vertices)
        .map(|i| ((positions[i as usize] - center) / diameter).length())
        .fold(0.0_f32, f32::max);
    for point in &mut positions {
        *point = ((*point - center) / diameter) / radius;
    }
    Ok(Model {
        positions,
        triangles,
    })
}

fn obj(bytes: &[u8]) -> Result<(Vec<Vec3>, Vec<Triangle>), String> {
    let body = std::str::from_utf8(bytes).map_err(|_| "OBJ 必须使用 UTF-8 编码。")?;
    let mut positions = Vec::new();
    let mut triangles = Vec::new();
    for (line_number, line) in body.lines().enumerate() {
        let mut items = line.split('#').next().unwrap_or("").split_whitespace();
        match items.next() {
            Some("v") => {
                if positions.len() == MAX_VERTICES {
                    return Err("模型顶点过多。".into());
                }
                let mut vertex = [0.0; 3];
                for value in &mut vertex {
                    *value = items
                        .next()
                        .and_then(|field| field.parse::<f32>().ok())
                        .filter(|p| p.is_finite())
                        .ok_or_else(|| format!("OBJ 第 {} 行的顶点无效。", line_number + 1))?;
                }
                positions.push(Vec3::from_array(vertex));
            }
            Some("f") => {
                let mut face = Vec::new();
                for item in items {
                    if face.len() == 2048 {
                        return Err("OBJ 的单个多边形超过 2,048 个顶点。".into());
                    }
                    let index = item
                        .split('/')
                        .next()
                        .and_then(|n| n.parse::<i64>().ok())
                        .filter(|&n| n != 0)
                        .ok_or("OBJ 面的顶点编号无效。")?;
                    let index = if index > 0 {
                        index - 1
                    } else {
                        positions.len() as i64 + index
                    };
                    if !(0..positions.len() as i64).contains(&index) {
                        return Err("OBJ 面引用了不存在的顶点。".into());
                    }
                    face.push(index as u32);
                }
                if face.len() < 3 {
                    return Err("OBJ 面需要至少三个顶点。".into());
                }
                if triangles.len() + face.len() - 2 > MAX_TRIANGLES {
                    return Err("模型三角面过多。".into());
                }
                push_face(&face, &positions, &mut triangles)?;
            }
            _ => {}
        }
    }
    Ok((positions, triangles))
}

fn push_face(
    face: &[u32],
    positions: &[Vec3],
    triangles: &mut Vec<Triangle>,
) -> Result<(), String> {
    let color = [151, 190, 217, 255];
    if let [a, b, c] = face {
        triangles.push(Triangle {
            vertices: [*a, *b, *c],
            color,
        });
    } else {
        // Project a concave n-gon onto its dominant plane before triangulating.
        let at = |i: usize| positions[face[i] as usize].as_dvec3();
        let normal = (0..face.len())
            .map(|i| at(i).cross(at((i + 1) % face.len())))
            .sum::<glam::DVec3>()
            .abs();
        let drop = if normal.x >= normal.y && normal.x >= normal.z {
            0
        } else if normal.y >= normal.z {
            1
        } else {
            2
        };
        let projected: Vec<_> = (0..face.len())
            .flat_map(|i| {
                let p = at(i);
                [p[(drop + 1) % 3], p[(drop + 2) % 3]]
            })
            .collect();
        let indices =
            earcutr::earcut(&projected, &[], 2).map_err(|_| "无法将 OBJ 的多边形转换为三角面。")?;
        for t in indices.chunks_exact(3) {
            triangles.push(Triangle {
                vertices: [face[t[0]], face[t[1]], face[t[2]]],
                color,
            });
        }
    }
    Ok(())
}

fn gltf(bytes: &[u8]) -> Result<(Vec<Vec3>, Vec<Triangle>), String> {
    use ::gltf::{buffer::Source, mesh::Mode};
    let source =
        ::gltf::Gltf::from_slice(bytes).map_err(|e| format!("无法读取 GLB / glTF 2.0：{e}"))?;
    let mut buffers = Vec::new();
    let mut total = 0;
    for buffer in source.buffers() {
        let data = match buffer.source() {
            Source::Bin => source
                .blob
                .as_ref()
                .ok_or("GLB 缺少内嵌几何数据。")?
                .clone(),
            Source::Uri(uri) => {
                let encoded = uri
                    .strip_prefix("data:")
                    .and_then(|u| u.split_once(";base64,"))
                    .map(|(_, payload)| payload)
                    .ok_or("模型引用外部几何文件。请导出自包含的 GLB 文件。")?;
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|_| "glTF 内嵌数据无效。")?
            }
        };
        total += data.len();
        if data.len() < buffer.length() || total > MAX_INPUT {
            return Err("glTF 几何数据大小无效。".into());
        }
        buffers.push(data);
    }
    let mut positions = Vec::new();
    let mut triangles = Vec::new();
    let scene = source
        .default_scene()
        .or_else(|| source.scenes().next())
        .ok_or("glTF 中没有场景。")?;
    let mut pending: Vec<_> = scene
        .nodes()
        .map(|node| (node, Mat4::IDENTITY, 0))
        .collect();
    let mut visited = 0;
    while let Some((node, transform, depth)) = pending.pop() {
        visited += 1;
        if depth > 96 || visited > 16384 {
            return Err("glTF 场景层级过于复杂。".into());
        }
        let matrix = transform * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives() {
                if !matches!(
                    primitive.mode(),
                    Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan
                ) {
                    continue;
                }
                let accessor = primitive
                    .get(&::gltf::Semantic::Positions)
                    .ok_or("glTF 网格没有顶点位置。")?;
                if accessor.count() > MAX_VERTICES - positions.len() {
                    return Err("模型顶点过多。".into());
                }
                let reader = primitive.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
                let pos = reader.read_positions().ok_or("无法读取 glTF 网格顶点。")?;
                let first = positions.len() as u32;
                positions.extend(pos.map(|p| matrix.transform_point3(Vec3::from_array(p))));
                let count = (positions.len() as u32) - first;
                let index_count = primitive.indices().map_or(count as usize, |a| a.count());
                if index_count >= 3 * MAX_TRIANGLES + 2 {
                    return Err("模型三角面过多。".into());
                }
                let indices: Vec<u32> = reader
                    .read_indices()
                    .map_or_else(|| (0..count).collect(), |i| i.into_u32().collect());
                if indices.iter().any(|&i| i >= count) {
                    return Err("glTF 的三角面引用了不存在的顶点。".into());
                }
                let rgba = primitive
                    .material()
                    .pbr_metallic_roughness()
                    .base_color_factor();
                let color = rgba.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
                let mut push = |vertices: [u32; 3]| -> Result<(), String> {
                    if triangles.len() == MAX_TRIANGLES {
                        return Err("模型三角面过多。".into());
                    }
                    triangles.push(Triangle {
                        vertices: vertices.map(|i| i + first),
                        color,
                    });
                    Ok(())
                };
                match primitive.mode() {
                    Mode::Triangles => {
                        if indices.len() % 3 != 0 {
                            return Err("glTF 三角面索引的长度无效。".into());
                        }
                        for f in indices.chunks_exact(3) {
                            push([f[0], f[1], f[2]])?;
                        }
                    }
                    Mode::TriangleStrip => {
                        for (i, f) in indices.windows(3).enumerate() {
                            push(if i % 2 == 0 {
                                [f[0], f[1], f[2]]
                            } else {
                                [f[1], f[0], f[2]]
                            })?;
                        }
                    }
                    Mode::TriangleFan => {
                        for f in indices
                            .get(1..)
                            .into_iter()
                            .flat_map(|rest| rest.windows(2))
                        {
                            push([indices[0], f[0], f[1]])?;
                        }
                    }
                    _ => {}
                }
            }
        }
        pending.extend(node.children().map(|child| (child, matrix, depth + 1)));
    }
    Ok((positions, triangles))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concave_obj_faces_and_negative_indices_are_kept() {
        let src = b"v 0 0 0\nv 3 0 0\nv 3 3 0\nv 1 1 0\nv 0 3 0\nf -5 -4 -3 -2 -1\n";
        let asset = Attachment {
            name: "station.obj".into(),
            bytes: Blob(Arc::new(src.to_vec())),
        };
        let m = decode(&asset).unwrap();
        assert_eq!(m.triangles.len(), 3);
        assert_eq!(m.positions.len(), 5);
        assert!(m.positions.iter().all(|p| p.length() <= 1.00001));
        assert!(decode(&Attachment {
            name: "broken.obj".into(),
            bytes: Blob(Arc::new(b"v 0 0 0\nf 1 2 3\n".to_vec()))
        })
        .is_err());
        assert!(is_model(&asset));
    }

    #[test]
    fn triangle_in_a_glb_is_decoded_with_its_material() {
        let mut json = serde_json::to_vec(&serde_json::json!({
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0,"translation":[0.0,1.0,0.0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"material":0}]}],
            "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0.8,0.1,0.2,1.0]}}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],
            "bufferViews":[{"buffer":0,"byteLength":36}],"buffers":[{"byteLength":36}]
        })).unwrap();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let mut bytes = b"glTF".to_vec();
        bytes.extend(2u32.to_le_bytes());
        bytes.extend((12u32 + 16 + json.len() as u32 + 36).to_le_bytes());
        bytes.extend((json.len() as u32).to_le_bytes());
        bytes.extend(*b"JSON");
        bytes.extend(&json);
        bytes.extend(36u32.to_le_bytes());
        bytes.extend(*b"BIN\0");
        for x in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bytes.extend(x.to_le_bytes());
        }
        let m = decode(&Attachment {
            name: "ship.glb".into(),
            bytes: Blob(Arc::new(bytes)),
        })
        .unwrap();
        assert_eq!(m.triangles.len(), 1);
        assert_eq!(m.triangles[0].color, [204, 26, 51, 255]);
    }
}
