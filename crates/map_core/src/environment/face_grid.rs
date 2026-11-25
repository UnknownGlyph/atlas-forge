//! Sample the same face as `Topology::face_at` at every centre of a regular grid.
use crate::{geom, topo::Topology, FaceId, LonLat, BACKGROUND};

pub(super) fn sample(
    topo: &Topology,
    width: usize,
    height: usize,
    center: impl Fn(usize) -> LonLat,
) -> Vec<FaceId> {
    // A local grid may cross the date line. Keep its original column indices.
    let mut columns: Vec<_> = (0..width).map(|x| (center(x).lon, x)).collect();
    columns.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let rows: Vec<_> = (0..height).map(|y| center(y * width).lat).collect();
    let mut hits = vec![BACKGROUND; width * height];
    let mut cuts = vec![Vec::new(); height];

    // Point picking takes the lowest qualifying face id, including on shared
    // boundaries. A hole can remove only the claim by its own face.
    for (&id, face) in &topo.faces {
        if id == BACKGROUND || face.bbox[1] > rows[0] || face.bbox[3] < rows[height - 1] {
            continue;
        }
        let outer = topo.ring_planar(&face.outer);
        scan_ring(&outer, &face.bbox, &rows, &columns, &mut cuts, |i, lon| {
            if hits[i] == BACKGROUND
                && [-360., 0., 360.].iter().any(|k| {
                    let x = lon + k;
                    face.bbox[0] <= x && x <= face.bbox[2]
                })
            {
                hits[i] = id;
            }
        });
        for hole in &face.holes {
            let mut path = topo.ring_coords(hole);
            path.reverse();
            let polygon = geom::ring_to_planar(&path);
            scan_ring(
                &polygon,
                &geom::bbox(&polygon),
                &rows,
                &columns,
                &mut cuts,
                |i, _| {
                    if hits[i] == id {
                        hits[i] = BACKGROUND;
                    }
                },
            );
        }
    }
    hits
}

fn scan_ring(
    ring: &[geom::P2],
    bbox: &[f64; 4],
    rows: &[f64],
    columns: &[(f64, usize)],
    cuts: &mut [Vec<f64>],
    mut visit: impl FnMut(usize, f64),
) {
    let Some(mut previous) = ring.last().copied() else {
        return;
    };
    let start = rows.partition_point(|&y| y > bbox[3]);
    let end = rows.partition_point(|&y| y >= bbox[1]);
    for &point in ring {
        let high = previous[1].max(point[1]);
        let low = previous[1].min(point[1]);
        let first = rows.partition_point(|&y| y >= high).max(start);
        let last = rows.partition_point(|&y| y >= low).min(end);
        for y in first..last {
            // Match point_in_poly's directed edge expression and strict ray
            // comparison, including grid centres exactly on an edge.
            cuts[y].push(
                (previous[0] - point[0]) * (rows[y] - point[1]) / (previous[1] - point[1])
                    + point[0],
            );
        }
        previous = point;
    }
    for y in start..end {
        let crossings = &mut cuts[y];
        crossings.sort_unstable_by(f64::total_cmp);
        for pair in crossings.chunks_exact(2) {
            for shift in [0., 360., -360., 720., -720.] {
                let left = columns.partition_point(|&(x, _)| x + shift < pair[0].max(bbox[0]));
                let right =
                    columns.partition_point(|&(x, _)| x + shift < pair[1] && x + shift <= bbox[2]);
                if left < right {
                    for &(lon, col) in &columns[left..right] {
                        visit(y * columns.len() + col, lon);
                    }
                }
            }
        }
        crossings.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        build::{build, SourcePolygon},
        overlay::{apply, DrawIntent},
        Document,
    };

    fn compare(topo: &Topology, xs: &[f64], ys: &[f64]) {
        let center = |i: usize| LonLat::new(xs[i % xs.len()], ys[i / xs.len()]);
        let sampled = sample(topo, xs.len(), ys.len(), center);
        for (i, id) in sampled.into_iter().enumerate() {
            assert_eq!(id, topo.face_at(center(i)), "at {:?}", center(i));
        }
    }

    #[test]
    fn raster_picking_matches_points_on_edges_holes_and_wrapped_polar_faces() {
        let ring = |a, b, c, d| {
            [(a, b), (c, b), (c, d), (a, d)]
                .map(|(x, y)| LonLat::new(x, y))
                .to_vec()
        };
        let (topo, _, _) = build(&[
            SourcePolygon {
                rings: vec![ring(-20., 0., 0., 20.)],
            },
            SourcePolygon {
                rings: vec![ring(0., 0., 20., 20.), ring(5., 5., 15., 15.)],
            },
        ])
        .unwrap();
        compare(
            &topo,
            &[-20., -10., 0., 5., 10., 15., 20.],
            &[21., 20., 15., 10., 5., 0., -1.],
        );

        let cap = (0..36)
            .map(|i| LonLat::new(-179. + 10. * i as f64, 70. + (i % 2) as f64))
            .collect::<Vec<_>>();
        let (doc, _) = apply(&Document::default(), &cap, DrawIntent::AddLand).unwrap();
        let (doc, _) = apply(&doc, &ring(170., 76., -170., 82.), DrawIntent::CarveWater).unwrap();
        compare(
            &doc.topo,
            &[165., 170., 179., -180., -179., -170., -165., 0.],
            &[90., 86., 82., 80., 76., 75., 71., 70., 60.],
        );
    }
}
