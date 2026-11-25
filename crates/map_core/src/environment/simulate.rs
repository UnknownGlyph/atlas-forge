use super::{compile, Cell, Edit, Field, Generation, Grid, Layer, Project, Settings};
use crate::{
    climate::{self, Field as Climate, Ground, Period, Surface},
    worldgen, Document, FaceKind, LonLat, BACKGROUND,
};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap, HashSet},
};

pub fn generate(doc: &Document, project: &Project) -> Result<Generation, String> {
    project.settings.validate()?;
    let settings = project.settings.resolve(doc)?;
    let grid = Grid::new(settings.region, settings.resolution)?;
    let climate_settings = doc
        .climate
        .as_ref()
        .map(|p| p.settings.clone())
        .unwrap_or_default();
    let existing = doc.climate.as_ref();
    let base = climate::sample(
        doc,
        existing
            .map(|p| &p.surface_edits)
            .unwrap_or(&Default::default()),
        existing.map_or(Surface::Unknown, |p| p.background),
    );
    let seas = climate::ocean_faces(doc);
    let edits = RasterEdits::new(project, grid);
    let coarse_faces = super::face_grid::sample(&doc.topo, climate::W, climate::H, climate::center);
    let mut global_ground: Vec<_> = base
        .iter()
        .copied()
        .enumerate()
        .map(|(i, source)| {
            let point = climate::center(i);
            let sample = terrain(
                doc,
                &settings,
                &seas,
                coarse_faces[i],
                source,
                point,
                existing.and_then(|x| x.surface_edits.get(&i)),
            );
            Ground {
                height: sample.height.max(0.0),
                ..sample.ground
            }
        })
        .collect();
    // An authored elevation is also a source for annual climate in the
    // global model, even when it lies inside a high-resolution local run.
    for (&i, edit) in &edits.standard[Field::Elevation as usize] {
        let at = climate::index(grid.center(i));
        global_ground[at].height = edit.value.max(0.0);
    }
    let atmosphere = climate::Scenario::new(
        climate_settings,
        doc.year,
        climate::Region::default(),
        &global_ground,
    )?;

    let annual: Vec<_> = (0..climate::W * climate::H)
        .map(|i| {
            [
                atmosphere.value(i, Climate::Temperature, Period::Annual),
                atmosphere.value(i, Climate::Rain, Period::Annual),
                global_ground[i].height,
            ]
        })
        .collect();
    let mut surface = Vec::with_capacity(grid.len());
    let mut cells = Vec::with_capacity(grid.len());
    let faces = super::face_grid::sample(&doc.topo, grid.width.into(), grid.height.into(), |i| grid.center(i));
    for (i, face) in faces.into_iter().enumerate() {
        let at = grid.center(i);
        let global = climate::index(at);
        let sample = terrain(
            doc,
            &settings,
            &seas,
            face,
            base[global],
            at,
            existing.and_then(|p| p.surface_edits.get(&global)),
        );
        let height = edits.value(Field::Elevation, i, sample.height);
        let [air_temp, rainfall, coarse_height] = local_weather(&annual, at);
        let adjusted_temp = air_temp - (height.max(0.0) - coarse_height) * 0.0060;
        let rainfall = (rainfall as f64 * settings.rainfall)
            * (1.0 + (height.max(0.0) - coarse_height) as f64 / 7000.0).clamp(0.45, 2.2);
        let mut cell = [0.0; Field::ALL.len()];
        cell[Field::Elevation as usize] = height;
        cell[Field::Temperature as usize] = edits.value(Field::Temperature, i, adjusted_temp);
        cell[Field::Rain as usize] = edits.value(Field::Rain, i, rainfall as f32);
        surface.push(sample);
        cells.push(cell);
    }

    let (order, downstream) = drainage(grid, &surface, &cells);
    let mut contributing = vec![0.0; grid.len()];
    let mut discharge = vec![0.0; grid.len()];
    let radius = doc.radius().max(1.0);
    let width = usize::from(grid.width);
    let row_area: Vec<_> = (0..usize::from(grid.height)).map(|y| grid.cell_area(y * width, radius)).collect();
    let g = &atmosphere.settings.planet;
    let year_seconds = g[climate::Param::Period] * 86_400.;
    for &i in order.iter().rev() {
        let precipitation = f64::from(cells[i][Field::Rain as usize]);
        let temperature = cells[i][Field::Temperature as usize];
        let land = surface[i].land();
        let area = if land { row_area[i / width] } else { 0.0 };
        let evap = f64::from((18.0 * temperature + 390.0).max(55.0));
        let runoff_fraction = (precipitation / (precipitation + evap)).clamp(0.0, 1.0);
        let catchment = edits.value(Field::Drainage, i, (contributing[i] + area) as f32);
        let runoff = edits.value(
            Field::Runoff,
            i,
            (discharge[i] + area * precipitation * runoff_fraction * 1000.0 / year_seconds) as f32,
        );
        cells[i][Field::Drainage as usize] = catchment;
        cells[i][Field::Runoff as usize] = runoff;
        if let Some(next) = downstream[i] {
            contributing[next as usize] += f64::from(catchment);
            discharge[next as usize] += f64::from(runoff);
        }
    }
    for (i, tile) in surface.iter().enumerate() {
        let center = grid.center(i);
        let gradient = neighbors(grid, i).fold(0.0_f64, |steepest, next| {
            let distance = distance_m(center, grid.center(next), radius);
            let gradient = (cells[i][Field::Elevation as usize]
                - cells[next][Field::Elevation as usize])
                .abs();
            steepest.max(f64::from(gradient) / distance.max(1.0))
        });
        let slope = gradient.atan().to_degrees() as f32;
        let cell = &mut cells[i];
        cell[Field::Slope as usize] = edits.value(Field::Slope, i, slope);
        let (height, temp, rain, runoff) = (
            cell[Field::Elevation as usize],
            cell[Field::Temperature as usize],
            cell[Field::Rain as usize],
            cell[Field::Runoff as usize],
        );
        let slope = cell[Field::Slope as usize];
        let rocky = if slope < 12.0 {
            1.0
        } else {
            (1.0 - (slope - 12.0) / 120.0).max(0.35)
        };
        let wet = if tile.land() {
            (100.0 * rain / (rain + settings.soil_capacity as f32) + 3.3 * runoff.ln_1p())
                .clamp(0.0, 100.0)
                * rocky
        } else {
            100.0
        };
        let wetness = edits.value(Field::Wetness, i, wet);
        cell[Field::Wetness as usize] = wetness;
        let frozen = temp < -6.0 || tile.ground.surface == Surface::Ice;
        let lake = tile.ground.surface == Surface::Lake;
        let landform = if !tile.land() {
            if frozen {
                7.0
            } else if lake {
                1.0
            } else {
                0.0
            }
        } else if frozen {
            7.0
        } else if slope > 22.0 {
            6.0
        } else if height > 2350.0 {
            if slope < 5.0 {
                4.0
            } else {
                5.0
            }
        } else if height > 520.0 || slope > 6.0 {
            3.0
        } else {
            2.0
        };
        cell[Field::Landform as usize] = edits.value(Field::Landform, i, landform);
        let biome = if frozen {
            2.0
        } else if !tile.land() {
            if lake {
                1.0
            } else {
                0.0
            }
        } else if wetness < 20.0 {
            3.0
        } else if wetness < 42.0 {
            4.0
        } else if temp < 8.0 {
            5.0
        } else if temp < 19.0 {
            6.0
        } else {
            7.0
        };
        let biome = edits.value(Field::Biome, i, biome);
        cell[Field::Biome as usize] = biome;
        let thermal = (1.0 - ((temp - 19.0) / 36.0).powi(2)).clamp(0.0, 1.0);
        let stress = (1.0 - settings.habitat_pressure as f32).clamp(0.0, 1.0);
        let vegetation = if tile.land() {
            wetness * thermal * stress * rocky + if tile.ground.forest { 9.0 } else { 0.0 }
        } else if frozen {
            2.0
        } else {
            (22.0 + thermal * 15.0) * stress
        };
        let vegetation = edits.value(Field::Vegetation, i, vegetation);
        cell[Field::Vegetation as usize] = vegetation;
        let diversity = if frozen {
            3.0
        } else {
            vegetation * (0.61 + thermal * 0.39) * stress
                + (1.0 - (tile.noise - 0.5).abs() * 2.0) * wetness.min(60.0) * 0.15
        };
        cell[Field::Diversity as usize] = edits.value(Field::Diversity, i, diversity);
    }
    let mut suitability = BTreeMap::new();
    for rule in &settings.rules {
        let expression = compile(rule)?;
        let layer = Layer::Suitability(rule.name.clone());
        let overrides = edits.custom.get(&rule.name);
        let samples = cells
            .iter()
            .zip(&surface)
            .enumerate()
            .map(|(i, (cell, tile))| {
                let value = crate::expr::eval(&expression, &|key| {
                    Some(match key {
                        "land" => {
                            if tile.land() {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        "latitude" => grid.center(i).lat,
                        "noise" => f64::from(tile.noise),
                        _ => f64::from(cell[*Field::ALL.iter().find(|f| f.key() == key)? as usize]),
                    })
                })
                .unwrap_or(0.0) as f32;
                let value = overrides
                    .and_then(|map| map.get(&i))
                    .map_or(value, |edit| edit.value);
                layer.normalize(value)
            })
            .collect();
        suitability.insert(rule.name.clone(), samples);
    }
    Ok(Generation {
        grid,
        year: doc.year,
        cells,
        downstream,
        suitability,
        settings,
    })
}

#[derive(Clone, Copy)]
struct TerrainSample {
    ground: Ground,
    height: f32,
    noise: f32,
}

impl TerrainSample {
    fn land(self) -> bool {
        matches!(
            self.ground.surface,
            Surface::Land | Surface::Forest | Surface::Ice
        )
    }
}

fn terrain(
    doc: &Document,
    settings: &Settings,
    seas: &HashSet<crate::topo::FaceId>,
    id: crate::FaceId,
    source: Ground,
    p: LonLat,
    edit: Option<&climate::SurfaceEdit>,
) -> TerrainSample {
    let xyz = worldgen::unit(p.lon, p.lat);
    let plate = worldgen::fbm(xyz.map(|x| x * 3.35), 4, settings.seed);
    let detail = worldgen::fbm(xyz.map(|x| x * 17.0), 4, settings.seed ^ 0x71E2_6304);
    let mut ground = source;
    let explicit = doc.terrain.as_ref().map(|terrain| terrain.at(p.lon, p.lat));
    ground.surface = if id != BACKGROUND {
        if doc.face_kind_at(id, doc.year) == Some(FaceKind::Land) {
            Surface::Land
        } else if seas.contains(&id) {
            Surface::Water
        } else {
            Surface::Lake
        }
    } else if doc.background_kind() == FaceKind::Land {
        Surface::Land
    } else if let Some(height) = explicit {
        if height > 0.0 {
            Surface::Land
        } else {
            Surface::Water
        }
    } else if let Some(climate) = &doc.climate {
        climate.background
    } else {
        Surface::Unknown
    };
    if ground.surface == Surface::Unknown
        && (source.forest || source.river || source.height >= 1800.0)
    {
        ground.surface = Surface::Land;
    }
    if ground.surface == Surface::Unknown && settings.generate_unknown {
        ground.surface = if plate > settings.sea_level {
            Surface::Land
        } else {
            Surface::Water
        }
    }
    let relief =
        settings.relief * (plate.abs() * 0.82 + (0.4 + detail * 0.6).max(0.0).powi(3) * 0.38);
    let height = match ground.surface {
        Surface::Water => explicit.unwrap_or_else(|| -120.0 - relief).min(0.0),
        Surface::Lake => explicit.unwrap_or(0.0).max(0.0),
        Surface::Unknown => 0.0,
        _ => explicit.map_or_else(
            || (28.0 + relief + source.height as f64 * 0.65).max(5.0),
            |h| h.max(0.0).max(source.height as f64),
        ),
    };
    TerrainSample {
        ground,
        noise: (0.5 + 0.5 * detail) as f32,
        height: edit.and_then(|e| e.height).unwrap_or(height as f32),
    }
}

fn local_weather(annual: &[[f32; 3]], point: LonLat) -> [f32; 3] {
    let w = climate::W as f64;
    let h = climate::H as f64;
    let x = (point.lon + 180.0) / 360.0 * w - 0.5;
    let y = (90.0 - point.lat) / 180.0 * h - 0.5;
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let [tx, ty] = [(x - x.floor()) as f32, (y - y.floor()) as f32];
    let sample = |field, dx: i32, dy: i32| {
        let x = (x0 + dx).rem_euclid(climate::W as i32) as usize;
        let y = (y0 + dy).clamp(0, climate::H as i32 - 1) as usize;
        annual[y * climate::W + x][field]
    };
    let bilinear = |field| {
        let north = sample(field, 0, 0) * (1.0 - tx) + sample(field, 1, 0) * tx;
        let south = sample(field, 0, 1) * (1.0 - tx) + sample(field, 1, 1) * tx;
        north * (1.0 - ty) + south * ty
    };
    std::array::from_fn(bilinear)
}

fn distance_m(a: LonLat, b: LonLat, radius_km: f64) -> f64 {
    let dlat = (b.lat - a.lat).to_radians();
    let dlon = crate::geom::lon_delta(a.lon, b.lon).to_radians();
    let hav = (dlat / 2.0).sin().powi(2)
        + a.lat.to_radians().cos() * b.lat.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    2000.0 * radius_km * hav.clamp(0.0, 1.0).sqrt().asin()
}

fn neighbors(grid: Grid, i: usize) -> impl Iterator<Item = usize> {
    let w = i32::from(grid.width);
    let h = i32::from(grid.height);
    let repeat = grid.whole_longitude();
    let (x, y) = (i as i32 % w, i as i32 / w);
    [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ]
    .into_iter()
    .filter_map(move |(dx, dy)| {
        let (x, y) = (x + dx, y + dy);
        let x = if repeat { x.rem_euclid(w) } else { x };
        ((0..w).contains(&x) && (0..h).contains(&y)).then_some((y * w + x) as usize)
    })
}

fn drainage(
    grid: Grid,
    surface: &[TerrainSample],
    cells: &[Cell],
) -> (Vec<usize>, Vec<Option<u32>>) {
    let w = grid.width as usize;
    let h = grid.height as usize;
    let mut queue = BinaryHeap::new();
    let mut visited = vec![false; grid.len()];
    let mut downstream = vec![None; grid.len()];
    let mut order = Vec::with_capacity(grid.len());
    let level = |i: usize| (cells[i][Field::Elevation as usize] * 100.0).round() as i32;
    for (i, ground) in surface.iter().enumerate() {
        let boundary = !grid.whole_longitude() && (i % w == 0 || i % w == w - 1)
            || (grid.region.north < 90.0 && i / w == 0)
            || (grid.region.south > -90.0 && i / w == h - 1);
        if boundary || matches!(ground.ground.surface, Surface::Water | Surface::Unknown) {
            visited[i] = true;
            queue.push(Reverse((level(i), i)));
        }
    }
    if queue.is_empty() {
        let sink = (0..grid.len()).min_by_key(|&i| level(i)).unwrap();
        visited[sink] = true;
        queue.push(Reverse((level(sink), sink)));
    }
    // Cells at the same spill elevation drain in order of discovery.
    // Every receiver is processed before its upstream cells.
    while let Some(Reverse((z, i))) = queue.pop() {
        order.push(i);
        for next in neighbors(grid, i) {
            if visited[next] {
                continue;
            }
            visited[next] = true;
            downstream[next] = Some(i as u32);
            queue.push(Reverse((z.max(level(next)), next)));
        }
    }
    (order, downstream)
}

struct RasterEdits<'a> {
    standard: [BTreeMap<usize, &'a Edit>; Field::ALL.len()],
    custom: BTreeMap<String, BTreeMap<usize, &'a Edit>>,
}

impl<'a> RasterEdits<'a> {
    fn new(project: &'a Project, grid: Grid) -> Self {
        let mut result = Self {
            standard: Default::default(),
            custom: BTreeMap::new(),
        };
        for edit in project.edits.values() {
            let Some(i) = grid.index(edit.at) else {
                continue;
            };
            let entries = match &edit.layer {
                Layer::Standard(field) => &mut result.standard[*field as usize],
                Layer::Suitability(name) => result.custom.entry(name.clone()).or_default(),
            };
            if entries.get(&i).is_none_or(|old| !old.pinned || edit.pinned) {
                entries.insert(i, edit);
            }
        }
        result
    }

    fn value(&self, field: Field, i: usize, generated: f32) -> f32 {
        field.normalize(
            self.standard[field as usize]
                .get(&i)
                .map_or(generated, |entry| entry.value),
        )
    }
}
