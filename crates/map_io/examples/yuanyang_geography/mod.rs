//! Authored continental outline and border controls in the geography brief's
//! local coordinate system. Provincial boundaries are seeded by geography,
//! with shared curved edges added to the finished topology.
use super::{Ally, Region};
pub mod districts;
pub mod statistics;
use districts::AreaPiece;
use geo::{
    Area, BooleanOps, BoundingRect, Contains, Coord, LineString, MultiPolygon, Point, Polygon,
};
use map_core::{
    build::SourcePolygon,
    geom::{self, LonLat, P2},
    repair::{self, RepairOptions},
    topo::{FaceId, Topology, BACKGROUND},
};
use std::collections::BTreeMap;

type Shape = MultiPolygon<f64>;

pub fn location(x: f64, y: f64) -> LonLat {
    LonLat::new(108.0 + x * 0.42, 57.0 - y * 0.37)
}

// The coast controls include the western sunset peninsula and its bays, the
// eastward cape with Longyin's estuary below it, the southern peninsula and
// the eastern peninsula across the gulf.
const CONTINENT: &[P2] = &[
    [7., 25.],
    [5., 22.],
    [8., 18.],
    [14., 16.],
    [16., 11.],
    [12., 9.],
    [9., 4.],
    [11., -2.],
    [4., -6.],
    [7.5, -10.5],
    [5., -15.],
    [9.3, -18.],
    [13.5, -17.6],
    [10., -11.6],
    [13.2, -12.7],
    [15.0, -9.5],
    [17.6, -10.1],
    [16.2, -15.8],
    [19.2, -14.2],
    [19.8, -19.4],
    [25., -21.4],
    [28., -18.6],
    [25.5, -15.2],
    [29.4, -12.5],
    [31., -17.],
    [38., -16.],
    [44., -20.],
    [51., -22.],
    [61., -20.],
    [72., -24.],
    [80., -19.],
    [92., -19.],
    [98., -15.],
    [105., -11.],
    [106., -6.],
    [101., -3.],
    [98., 2.],
    [94., 4.],
    [88., 10.],
    [90., 14.],
    [86., 18.],
    [90., 23.],
    [87., 29.],
    [91., 34.],
    [91., 38.],
    [99.8, 40.9],
    [98., 43.1],
    [94.4, 43.2],
    [96.2, 45.8],
    [92.1, 47.9],
    [88.8, 48.6],
    [90.3, 44.1],
    [87.2, 43.9],
    [85.8, 45.4],
    [84., 48.],
    [81., 50.],
    [83.5, 52.],
    [87., 51.5],
    [90., 54.],
    [92., 58.],
    [96., 59.],
    [100., 57.],
    [105., 60.],
    [109., 65.],
    [116., 65.],
    [120., 69.],
    [118., 71.9],
    [114.3, 70.4],
    [112.4, 71.7],
    [114.1, 74.2],
    [111.2, 75.],
    [114.2, 76.2],
    [116.8, 75.1],
    [121.3, 80.1],
    [119.7, 82.5],
    [115.6, 81.5],
    [112., 84.],
    [107., 85.],
    [104., 81.],
    [100., 82.],
    [96., 78.5],
    [94., 74.],
    [90., 75.],
    [90.5, 80.],
    [85.5, 81.],
    [81., 85.],
    [76., 83.],
    [74., 85.],
    [70., 84.],
    [68., 87.],
    [62., 92.],
    [57., 94.5],
    [53.4, 90.],
    [53.9, 87.6],
    [50., 83.8],
    [47.2, 84.5],
    [45.6, 87.6],
    [46.8, 91.9],
    [44.7, 96.6],
    [40.5, 99.],
    [36.8, 94.4],
    [40., 94.],
    [39., 90.],
    [35., 89.],
    [31., 92.],
    [26., 96.],
    [22., 94.],
    [18., 96.],
    [15., 91.],
    [10., 94.],
    [5., 96.],
    [2., 93.],
    [-2., 94.],
    [-6., 92.],
    [-3., 86.],
    [-7., 81.],
    [-6., 75.],
    [-3., 72.],
    [-4., 66.],
    [0., 62.],
    [5., 63.],
    [8., 60.],
    [9., 57.],
    [7., 54.],
    [10., 51.],
    [9., 47.],
    [13., 44.],
    [13., 41.],
    [9., 40.],
    [7., 37.],
    [10., 35.],
    [12., 31.],
    [10., 28.],
    [6., 28.],
];

const BORDER_MASKS: &[(&str, &[P2])] = &[
    (
        "ML",
        &[
            [-35., -35.],
            [47., -35.],
            [47., -17.],
            [43., -14.],
            [44., -10.],
            [40., -6.],
            [38., -1.],
            [34., 1.],
            [30., 3.],
            [25., 4.],
            [20., 5.],
            [16., 3.],
            [12., 6.],
            [-35., 8.],
        ],
    ),
    (
        "PS",
        &[
            [18., -35.],
            [127., -35.],
            [127., 8.],
            [105., 5.0],
            [96., 10.3],
            [90., 9.9],
            [86., 6.7],
            [83., 7.2],
            [80., 10.4],
            [75., 10.2],
            [71., 8.6],
            [68., 5.0],
            [64., 4.2],
            [61., 3.0],
            [54., 3.9],
            [51., 2.1],
            [47., 3.4],
            [43., 6.0],
            [39., 4.9],
            [32., 3.],
            [25., 1.],
            [18., -7.],
        ],
    ),
    (
        "XJ",
        &[
            [-25., 56.],
            [8., 56.],
            [11., 60.],
            [14., 63.],
            [10., 68.],
            [11., 73.],
            [9., 76.],
            [12., 79.],
            [13., 83.],
            [15., 88.],
            [18., 93.],
            [21., 101.],
            [21., 106.],
            [-25., 106.],
        ],
    ),
    (
        "DS",
        &[
            [88., 55.],
            [93., 53.],
            [121., 54.],
            [128., 61.],
            [128., 94.],
            [90., 96.],
            [84., 84.],
            [86., 80.],
            [84., 75.],
            [85., 71.],
            [88., 68.],
            [86., 65.],
            [86., 61.],
        ],
    ),
];

fn polygon(points: impl IntoIterator<Item = P2>) -> Polygon<f64> {
    Polygon::new(
        LineString::from(points.into_iter().map(|[x, y]| (x, y)).collect::<Vec<_>>()),
        vec![],
    )
}

// Catmull-Rom controls for the broad shape, then small tapered perturbations
// along its normal for coves and capes. Vertices are generated only once.
fn coast(controls: &[P2], amplitude: f64) -> Polygon<f64> {
    let n = controls.len();
    polygon((0..n).flat_map(|i| {
        let (p0, p1, p2, p3) = (
            controls[(i + n - 1) % n],
            controls[i],
            controls[(i + 1) % n],
            controls[(i + 2) % n],
        );
        let delta = [p2[0] - p1[0], p2[1] - p1[1]];
        let norm = delta[0].hypot(delta[1]);
        let steps = (norm / 0.36).ceil().max(4.0) as usize;
        (0..steps).map(move |j| {
            let t = j as f64 / steps as f64;
            let curve = |axis: usize| {
                let (a, b, c, d) = (p0[axis], p1[axis], p2[axis], p3[axis]);
                b + t
                    * 0.5
                    * (c - a + t * (2. * a - 5. * b + 4. * c - d + t * (3. * (b - c) + d - a)))
            };
            let noise = amplitude.min(norm * 0.14)
                * (std::f64::consts::PI * t).sin().powi(2)
                * (0.62 * (10.2 * t + i as f64 * 1.83).sin()
                    + 0.28 * (22.6 * t + i as f64 * 2.96).sin());
            [
                curve(0) - delta[1] / norm * noise,
                curve(1) + delta[0] / norm * noise,
            ]
        })
    }))
}

pub fn landform(points: &[[u32; 2]], domain: &[map_core::doc::PlanarPolygon]) -> Vec<Vec<LonLat>> {
    let controls: Vec<_> = points.iter().map(|p| p.map(f64::from)).collect();
    let outline = coast(&controls, 0.34);
    let contour = polygon(outline.exterior().coords().map(|c| {
        let p = location(c.x, c.y);
        [p.lon, p.lat]
    }));
    let line =
        |ring: &[P2]| LineString::from(ring.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>());
    let territory = MultiPolygon::new(
        domain
            .iter()
            .map(|p| {
                Polygon::new(
                    line(&p.outer),
                    p.holes.iter().map(|ring| line(ring)).collect(),
                )
            })
            .collect(),
    );
    territory
        .intersection(&contour)
        .0
        .iter()
        .flat_map(|p| {
            std::iter::once(p.exterior())
                .chain(p.interiors())
                .map(|ring| {
                    ring.coords()
                        .take(ring.0.len().saturating_sub(1))
                        .map(|c| LonLat::new(c.x, c.y))
                        .collect()
                })
        })
        .collect()
}

/// Interpolate each river reach through the named confluences.
pub fn watercourse(points: &[[u32; 2]]) -> Vec<LonLat> {
    let at = |i: usize| points[i].map(f64::from);
    let mut out: Vec<_> = (0..points.len() - 1)
        .flat_map(|i| {
            let (a, b, c, d) = (
                at(i.saturating_sub(1)),
                at(i),
                at(i + 1),
                at((i + 2).min(points.len() - 1)),
            );
            let steps = (((b[0] - c[0]).hypot(b[1] - c[1])) / 0.55).ceil().max(2.0) as usize;
            (0..steps).map(move |j| {
                let t = j as f64 / steps as f64;
                let coordinate = |k: usize| {
                    b[k] + t
                        * 0.5
                        * (c[k] - a[k]
                            + t * (2. * a[k] - 5. * b[k] + 4. * c[k] - d[k]
                                + t * (3. * (b[k] - c[k]) + d[k] - a[k])))
                };
                location(coordinate(0), coordinate(1))
            })
        })
        .collect();
    let [x, y] = at(points.len() - 1);
    out.push(location(x, y));
    out
}

fn island([x, y]: P2, rx: f64, ry: f64, rotation: f64) -> Polygon<f64> {
    let ring: Vec<_> = (0..18)
        .map(|i| {
            let t = std::f64::consts::TAU * i as f64 / 18.0;
            let bump = 1.0 + 0.18 * (t * 3.0 + 0.6).cos() + 0.09 * (5.0 * t).sin();
            let (a, b) = (rx * bump * t.cos(), ry * bump * t.sin());
            [
                x + a * rotation.cos() - b * rotation.sin(),
                y + a * rotation.sin() + b * rotation.cos(),
            ]
        })
        .collect();
    coast(&ring, rx.min(ry) * 0.13)
}

fn candidates(domain: &Shape, steps: usize) -> Vec<P2> {
    let Some(bounds) = domain.bounding_rect() else {
        return Vec::new();
    };
    (0..steps)
        .flat_map(|x| {
            (0..steps).filter_map(move |y| {
                let sample = |n| (n as f64 + 0.5) / steps as f64;
                let point = [
                    bounds.min().x + bounds.width() * sample(x),
                    bounds.min().y + bounds.height() * sample(y),
                ];
                domain
                    .contains(&Point::new(point[0], point[1]))
                    .then_some(point)
            })
        })
        .collect()
}

fn squared_distance(p: P2, q: P2) -> f64 {
    (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)
}

fn seats(domain: &Shape, requested: &[(&str, P2)]) -> Result<Vec<P2>, String> {
    let spots = candidates(domain, 155);
    let mut assigned = Vec::new();
    for &(code, preferred) in requested {
        let available = |p: P2| assigned.iter().all(|&s| squared_distance(s, p) > 0.75);
        let at = if available(preferred) && domain.contains(&Point::new(preferred[0], preferred[1]))
        {
            preferred
        } else {
            spots
                .iter()
                .copied()
                .filter(|&p| available(p))
                .min_by(|&a, &b| {
                    squared_distance(a, preferred).total_cmp(&squared_distance(b, preferred))
                })
                .ok_or_else(|| format!("{code} 找不到区划种子"))?
        };
        assigned.push(at);
    }
    Ok(assigned)
}

fn partition(domain: &Shape, preferred: P2, count: usize) -> Result<Vec<(P2, Shape)>, String> {
    let candidates = candidates(domain, 38);
    let first = if domain.contains(&Point::new(preferred[0], preferred[1])) {
        preferred
    } else {
        candidates
            .iter()
            .copied()
            .min_by(|&a, &b| {
                squared_distance(a, preferred).total_cmp(&squared_distance(b, preferred))
            })
            .ok_or("缺少可绘制内区")?
    };
    let mut sites = vec![first];
    while sites.len() < count {
        let distance = |p: P2| {
            sites
                .iter()
                .map(|&q| squared_distance(p, q))
                .fold(f64::INFINITY, f64::min)
        };
        let score = |p: P2| {
            let tilt = 1.0
                + (p[0] * 0.81 + p[1] * 1.38).sin() * 0.24
                + (p[0] * 1.17 - p[1] * 0.53).cos() * 0.13;
            distance(p) * tilt
        };
        let next = candidates
            .iter()
            .copied()
            .max_by(|&a, &b| score(a).total_cmp(&score(b)))
            .filter(|&p| distance(p) > 1e-12)
            .ok_or("没有足够的分区位置")?;
        sites.push(next);
    }
    Ok(sites
        .iter()
        .map(|&site| (site, domain.intersection(&voronoi(site, &sites))))
        .collect())
}

fn voronoi(site: P2, sites: &[P2]) -> Polygon<f64> {
    let mut ring = vec![[-50., -45.], [143., -45.], [143., 132.], [-50., 132.]];
    for &other in sites.iter().filter(|q| **q != site) {
        let mid = [(site[0] + other[0]) * 0.5, (site[1] + other[1]) * 0.5];
        let normal = [0.85 * (other[0] - site[0]), other[1] - site[1]];
        let side = |p: P2| (p[0] - mid[0]) * normal[0] + (p[1] - mid[1]) * normal[1];
        let mut trimmed = Vec::new();
        for i in 0..ring.len() {
            let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
            let (a, b) = (side(p), side(q));
            if a <= 0.0 {
                trimmed.push(p);
            }
            if (a <= 0.0) != (b <= 0.0) {
                let t = a / (a - b);
                trimmed.push([p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]);
            }
        }
        ring = trimmed;
    }
    polygon(ring)
}

pub struct FacePiece {
    pub face: FaceId,
    pub owner: String,
    pub province: String,
    pub country: String,
}

pub struct Mesh {
    pub topo: Topology,
    pub faces: Vec<FacePiece>,
    pub districts: Vec<districts::Unit>,
}

pub fn redraw(regions: &[Region], allies: &[Ally]) -> Result<Mesh, String> {
    let mut land = MultiPolygon::new(vec![coast(CONTINENT, 0.80)]);
    let mut national = Vec::<(&str, Shape)>::new();
    let mut provincial = BTreeMap::<&str, Shape>::new();
    for &(code, controls) in BORDER_MASKS {
        let mask = coast(controls, 0.20);
        let member = land.intersection(&mask);
        land = land.difference(&mask);
        national.push((code, member));
    }
    let shape = |parts| MultiPolygon::new(parts);
    for (code, specs) in [
        (
            "XJ",
            &[
                [1.0, 99.0, 1.8, 0.75],
                [-2.0, 100.5, 0.8, 0.45],
                [3.3, 101.5, 0.7, 0.5],
            ][..],
        ),
        (
            "DS",
            &[
                [122.0, 85.4, 1.5, 0.85],
                [124.1, 87.7, 0.76, 0.56],
                [121.3, 90.6, 0.60, 0.39],
                [119.4, 89.0, 0.45, 0.33],
                [117.9, 91.3, 0.37, 0.28],
                [114.9, 90.1, 0.47, 0.42],
            ][..],
        ),
        (
            "ML",
            &[
                [1.6, -14.3, 1.45, 0.77],
                [0.5, -16.7, 0.65, 0.50],
                [-1.0, -14.4, 0.47, 0.30],
                [1.1, -10.8, 0.7, 0.38],
            ][..],
        ),
        (
            "DDa",
            &[
                [88.0, 94.1, 4.0, 3.3],
                [92.5, 97.0, 0.80, 0.58],
                [85.0, 99.2, 0.53, 0.33],
            ][..],
        ),
    ] {
        let parts = shape(
            specs
                .iter()
                .map(|[x, y, w, h]| island([*x, *y], *w, *h, -0.25))
                .collect(),
        );
        if let Some((_, area)) = national.iter_mut().find(|(key, _)| *key == code) {
            let parts = parts.difference(&land);
            *area = area.union(&parts);
        } else {
            provincial.insert(code, parts.difference(&land));
        }
    }

    // Small municipalities have explicit envelopes. Their locations stay
    // separate from the broad province sites and remain small in the atlas.
    for (code, requested, rx, ry) in [
        ("SM", [39., 44.], 2.05, 1.6),
        ("XC", [48., 50.], 1.75, 1.45),
        ("GH", [10.5, 37.5], 2.45, 1.9),
        ("LY", [83.0, 53.0], 2.05, 1.65),
    ] {
        let center = seats(&land, &[(code, requested)])?[0];
        let cut = island(center, rx, ry, 0.2);
        let member = land.intersection(&cut);
        land = land.difference(&cut);
        provincial.insert(code, member);
    }

    let samples: Vec<_> = regions
        .iter()
        .filter(|r| !["SM", "XC", "GH", "LY", "DDa"].contains(&r.code.as_str()))
        .map(|r| (r.code.as_str(), r.xy.map(f64::from)))
        .collect();
    let sites = seats(&land, &samples)?;
    for ((code, _), site) in samples.into_iter().zip(&sites) {
        provincial.insert(code, land.intersection(&voronoi(*site, &sites)));
    }

    let provinces = regions
        .iter()
        .map(|r| {
            provincial
                .remove(r.code.as_str())
                .map(|shape| (r, shape))
                .ok_or_else(|| format!("缺少一级行政区 {}", r.code))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (mut areas, districts) = districts::build(provinces)?;
    for (country, domain) in national {
        let requested: Vec<_> = allies
            .iter()
            .filter(|r| r.country == country)
            .map(|r| (r.code.as_str(), r.xy.map(f64::from)))
            .collect();
        let sites = seats(&domain, &requested)?;
        for ((code, _), site) in requested.into_iter().zip(&sites) {
            areas.push(AreaPiece {
                country: country.into(),
                province: code.into(),
                owner: code.into(),
                shape: domain.intersection(&voronoi(*site, &sites)),
            });
        }
    }

    let ring = |line: &LineString<f64>| {
        line.coords()
            .map(|Coord { x, y }| location(*x, *y))
            .collect()
    };
    let mut polygons = Vec::new();
    let mut feature_of = Vec::new();
    for (i, area) in areas.iter().enumerate() {
        if area.shape.unsigned_area() < 0.010 {
            return Err(format!("缺少行政区 {} 的有效轮廓", area.owner));
        }
        for part in &area.shape.0 {
            if part.unsigned_area() <= 0.001 {
                continue;
            }
            polygons.push(SourcePolygon {
                rings: std::iter::once(part.exterior())
                    .chain(part.interiors())
                    .map(ring)
                    .collect(),
            });
            feature_of.push(i);
        }
    }
    let result = repair::repair(
        &polygons,
        &feature_of,
        &RepairOptions {
            snap_m: 24.0,
            radius_km: 9650.0,
            ..Default::default()
        },
    )
    .map_err(|r| format!("无法拼合轮廓：{:?} / {:?}", r.problems, r.build.problems))?;
    let mut topo = result.topo;
    let mut pieces = Vec::new();
    let mut code_of = BTreeMap::new();
    for (i, area) in areas.iter().enumerate() {
        let faces = result
            .faces
            .get(&i)
            .ok_or_else(|| format!("拓扑缺少 {}", area.owner))?;
        for &face in faces {
            pieces.push(FacePiece {
                face,
                owner: area.owner.clone(),
                province: area.province.clone(),
                country: area.country.clone(),
            });
            code_of.insert(face, i);
        }
    }
    let coast_segments = topo
        .edges
        .values()
        .filter(|e| e.left == BACKGROUND || e.right == BACKGROUND)
        .count();
    if coast_segments == 0 {
        return Err("重绘没有有效海岸".into());
    }
    // The repair pass joins both banks first. Bow only shared internal
    // straight segments, and leave their common endpoints fixed.
    let edges: Vec<_> = topo.edges.keys().copied().collect();
    for id in edges {
        let edge = &topo.edges[&id];
        let (Some(&left), Some(&right)) = (code_of.get(&edge.left), code_of.get(&edge.right))
        else {
            continue;
        };
        let a = &areas[left];
        let b = &areas[right];
        let strength = if a.country != b.country {
            0.16
        } else if a.province != b.province {
            0.146
        } else {
            0.125
        };
        let line = topo.edge_full(edge);
        let mut points = Vec::new();
        for (index, pair) in line.windows(2).enumerate() {
            if index > 0 {
                points.push(pair[0]);
            }
            let [a, b] = [pair[0], pair[1]];
            let length = geom::geo_dist_deg(a, b);
            if length < 0.13 {
                continue;
            }
            let n = (length / 0.072).ceil().min(106.0) as usize;
            let (dx, dy) = (b.lon - a.lon, b.lat - a.lat);
            let normal = dx.hypot(dy);
            let phase = id.0 as f64 * 2.123 + index as f64;
            for j in 1..n {
                let t = j as f64 / n as f64;
                let bend = (length * strength).min(0.37)
                    * (t * std::f64::consts::PI).sin().powi(2)
                    * (0.86 * (t * 5.7 + phase).sin() + 0.19 * (t * 14.5 - phase).cos());
                points.push(LonLat::new(
                    a.lon + t * dx - dy / normal * bend,
                    a.lat + t * dy + dx / normal * bend,
                ));
            }
        }
        if points.len() > edge.pts.len() {
            let original = edge.pts.clone();
            if topo.replace_edge_points(id, points).is_err() {
                topo.replace_edge_points(id, original)?;
            }
        }
    }
    topo.validate()?;
    Ok(Mesh {
        topo,
        faces: pieces,
        districts,
    })
}
