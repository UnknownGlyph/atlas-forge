use super::{center, index, Region, H, W};
use crate::area::Pattern;
use crate::symbol::SymbolKind;
use crate::{geom, Document, FaceKind, LonLat, BACKGROUND};
use imbl::OrdMap;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Surface {
    #[default]
    Unknown,
    Land,
    Water,
    Lake,
    Forest,
    Ice,
}
impl Surface {
    pub const ALL: [Self; 6] = [
        Self::Unknown,
        Self::Land,
        Self::Water,
        Self::Lake,
        Self::Forest,
        Self::Ice,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Unknown => "未指定",
            Self::Land => "陆地",
            Self::Water => "海洋",
            Self::Lake => "湖泊",
            Self::Forest => "森林",
            Self::Ice => "冰雪",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SurfaceEdit {
    pub surface: Option<Surface>,
    pub height: Option<f32>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ground {
    pub surface: Surface,
    pub height: f32,
    pub river: bool,
    pub forest: bool,
}
impl Ground {
    pub fn water(self, unknown: f64) -> f64 {
        match self.surface {
            Surface::Unknown => unknown,
            Surface::Lake | Surface::Water => 1.,
            _ => 0.,
        }
    }
}

// Rasterize each ring once by scan line. Unwrapped spans wrap back to the
// global grid; crossing a hole toggles the same mask as crossing the exterior.
fn toggle_ring(mask: &mut [bool], touched: &mut Vec<usize>, ring: &[geom::P2]) {
    if ring.len() < 3 {
        return;
    }
    let mut cuts: Vec<Vec<f64>> = vec![Vec::new(); H];
    let row = |lat: f64| {
        ((90. - lat) * H as f64 / 180. - 0.5)
            .ceil()
            .clamp(0., H as f64) as usize
    };
    let mut prev = ring[ring.len() - 1];
    for &pt in ring {
        let (low, high) = if prev[1] < pt[1] {
            (prev, pt)
        } else {
            (pt, prev)
        };
        for (y, crossings) in cuts
            .iter_mut()
            .enumerate()
            .take(row(low[1]))
            .skip(row(high[1]))
        {
            let lat = center(y * W).lat;
            crossings.push(low[0] + (high[0] - low[0]) * (lat - low[1]) / (high[1] - low[1]));
        }
        prev = pt;
    }
    for (y, xs) in cuts.iter_mut().enumerate() {
        xs.sort_by(f64::total_cmp);
        for pair in xs.as_chunks::<2>().0 {
            let col = |lon: f64| ((lon + 180.) * W as f64 / 360. - 0.5).ceil() as i32;
            for x in col(pair[0])..col(pair[1]) {
                let ix = y * W + x.rem_euclid(W as i32) as usize;
                mask[ix] = !mask[ix];
                touched.push(ix);
            }
        }
    }
}
pub fn sample(
    doc: &Document,
    edits: &OrdMap<usize, SurfaceEdit>,
    background: Surface,
) -> Vec<Ground> {
    let fixed_background = doc.terrain.is_some() || doc.background_kind() == FaceKind::Land;
    let background = if background != Surface::Unknown || !fixed_background {
        background
    } else if doc.background_kind() == FaceKind::Land {
        Surface::Land
    } else {
        Surface::Water
    };
    let mut ground = vec![
        Ground {
            surface: background,
            ..Ground::default()
        };
        W * H
    ];
    let mut mask = vec![false; W * H];
    let mut touched = Vec::new();
    let sea = ocean_faces(doc);
    for (&id, face) in &doc.topo.faces {
        if id == BACKGROUND {
            continue;
        }
        toggle_ring(&mut mask, &mut touched, &doc.topo.ring_planar(&face.outer));
        for hole in &face.holes {
            let mut points = doc.topo.ring_coords(hole);
            points.reverse();
            toggle_ring(&mut mask, &mut touched, &geom::ring_to_planar(&points));
        }
        let surface = if doc.face_kind_at(id, doc.year) == Some(FaceKind::Land) {
            Surface::Land
        } else if sea.contains(&id) {
            Surface::Water
        } else {
            Surface::Lake
        };
        for i in touched.drain(..) {
            if std::mem::take(&mut mask[i]) {
                ground[i].surface = surface;
            }
        }
    }
    if let Some(t) = &doc.terrain {
        for (i, g) in ground.iter_mut().enumerate() {
            let p = center(i);
            if g.water(0.) < 1. {
                g.height = t.at(p.lon, p.lat).max(0.) as f32;
            }
        }
    }
    for area in doc.areas.values().filter(|a| {
        matches!(a.pattern, Pattern::Mountains | Pattern::Trees)
            && crate::symbol::in_years(a.from, a.to, doc.year)
    }) {
        for ring in area.rings() {
            toggle_ring(&mut mask, &mut touched, &geom::unwrap_path(ring, None));
        }
        for i in touched.drain(..) {
            if !std::mem::take(&mut mask[i]) {
                continue;
            }
            let g = &mut ground[i];
            if area.pattern == Pattern::Mountains {
                g.height = g.height.max(1800.);
            } else {
                g.forest = true;
            }
            if g.surface == Surface::Unknown {
                g.surface = Surface::Land;
            }
        }
    }
    for s in doc.symbols.values().filter(|s| {
        matches!(s.kind, SymbolKind::Peak | SymbolKind::Volcano)
            && crate::symbol::in_years(s.from, s.to, doc.year)
    }) {
        let g = &mut ground[index(s.at)];
        g.height = g.height.max(2600.);
        if g.surface == Surface::Unknown {
            g.surface = Surface::Land;
        }
    }
    for line in doc.rivers.values().filter(|line| line.route.is_none()) {
        for pair in line.pts.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let dx = geom::lon_delta(a.lon, b.lon);
            let dy = b.lat - a.lat;
            let n = (dx.abs().max(dy.abs()) / 1.5).ceil().max(1.) as usize;
            for k in 0..=n {
                let t = k as f64 / n as f64;
                ground[index(LonLat::new(a.lon + dx * t, a.lat + dy * t))].river = true;
            }
        }
    }
    for (&i, e) in edits {
        if let Some(g) = ground.get_mut(i) {
            if let Some(s) = e.surface {
                g.surface = s;
                g.forest = s == Surface::Forest;
            }
            if let Some(h) = e.height {
                g.height = h;
            }
        }
    }
    ground
}
pub fn footprint(center_point: LonLat, radius: f64) -> Vec<usize> {
    let mut cells: Vec<_> = (0..W * H)
        .filter(|&i| {
            crate::measure::great_circle(center_point, center(i), 1.).to_degrees() <= radius
        })
        .collect();
    if cells.is_empty() {
        cells.push(index(center_point));
    }
    cells
}
pub fn region_cells(region: Region) -> impl Iterator<Item = usize> {
    (0..W * H).filter(move |&i| region.contains(center(i)))
}

pub(crate) fn ocean_faces(doc: &Document) -> std::collections::HashSet<crate::topo::FaceId> {
    // A water face joined to the exterior ocean is open water. An inland
    // lake has no such path, even when its holes contain islands.
    let mut sea = std::collections::HashSet::new();
    let mut queue = vec![BACKGROUND];
    while let Some(id) = queue.pop() {
        if doc.face_kind_at(id, doc.year) != Some(FaceKind::Water) || !sea.insert(id) {
            continue;
        }
        let face = &doc.topo.faces[&id];
        for arc in face.outer.iter().chain(face.holes.iter().flatten()) {
            let edge = &doc.topo.edges[&arc.edge];
            let next = if edge.left == id {
                edge.right
            } else {
                edge.left
            };
            if !sea.contains(&next) {
                queue.push(next);
            }
        }
    }
    sea
}
