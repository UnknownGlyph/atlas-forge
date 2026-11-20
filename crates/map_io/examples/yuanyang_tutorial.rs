//! Rebuild the tutorial images and project from the included fictional geography.
use map_core::{
    area::{AreaMark, Pattern},
    diplomacy::{Agreement, Kind},
    doc::{
        Annotation, Dash, EntityKind, HomeView, LabelPlace, LayerKind, LineFeature, LineId,
        RouteStyle, POLITICAL,
    },
    features::FeatureRef,
    geom,
    paper::Paper,
    projection::Projection,
    symbol::{MapSymbol, SymbolKind},
    theme,
    world_book::{Attachment, Link, Node, WorldBook, ROOT},
    Document, EntityId, FaceKind, LonLat,
};
use map_io::{
    geojson, png, project,
    scene::{build_scene, ExportOptions, PixelMap, SceneLine},
    svg,
};
use serde::Deserialize;
#[path = "yuanyang_geography/mod.rs"]
mod geography;
use geography::{districts::METROS, statistics::Fields};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
    sync::Arc,
};

#[derive(Deserialize)]
struct Region {
    name: String,
    code: String,
    seat: String,
    group: usize,
    area: f64,
    pop: u32,
    gdp: u32,
    xy: [u32; 2],
}
#[derive(Deserialize)]
struct Geography {
    regions: Vec<Region>,
}

#[derive(Deserialize)]
struct Ally {
    name: String,
    code: String,
    country: String,
    seat: String,
    area: f64,
    pop: u32,
    xy: [i32; 2],
    note: String,
}

const SOVEREIGN_DATA: [(&str, f64, u32, u32); 5] = [
    ("YY", 1150., 80_000, 600_000),
    ("PS", 310., 19_000, 158_000),
    ("XJ", 190., 14_000, 126_000),
    ("ML", 165., 9_500, 76_000),
    ("DS", 135., 12_500, 105_000),
];
const MAIN: &str = "远洋人民共和国";
const NEIGHBOURS: [(&str, &str, [u8; 4]); 4] = [
    ("PS", "皮斯共和国", [203, 225, 193, 255]),
    ("ML", "麦伦民主共和国", [202, 222, 237, 255]),
    ("XJ", "辛加尔王国", [245, 220, 184, 255]),
    ("DS", "东升国", [243, 205, 195, 255]),
];
const GROUPS: [&str; 7] = [
    "中枢平原与大江腹心区",
    "西海共和与日落古陆区",
    "西北大漠与北宁边疆区",
    "北方卡罗与东北林海区",
    "东海初耀与龙吟江海区",
    "南江暖海与岛礁明珠区",
    "西南云山与奇普高台区",
];
const GROUP_COLOURS: [[u8; 4]; 7] = [
    [230, 211, 156, 255],
    [225, 179, 153, 255],
    [237, 213, 167, 255],
    [168, 204, 180, 255],
    [176, 208, 220, 255],
    [181, 221, 212, 255],
    [204, 181, 168, 255],
];

fn place([x, y]: [u32; 2]) -> LonLat {
    geography::location(f64::from(x), f64::from(y))
}

fn output_options(title: &str) -> ExportOptions {
    ExportOptions {
        width: 1536,
        height: 1280,
        projection: Projection::Equirectangular,
        bounds: Some([
            102_f64.to_radians(),
            17_f64.to_radians(),
            162_f64.to_radians(),
            68_f64.to_radians(),
        ]),
        title: title.into(),
        legend: false,
        graticule: false,
        ..Default::default()
    }
}

fn overview(doc: &Document, depth: usize, keep_cities: bool) -> Result<Document, String> {
    let mut view = doc.clone();
    let faces: Vec<_> = doc.owner.iter().map(|(&face, &id)| (face, id)).collect();
    for (face, id) in faces {
        let lineage: Vec<_> = doc.ancestors(id).collect();
        let offset = lineage.len().saturating_sub(depth + 1);
        if offset > 0 {
            view.assign(face, Some(lineage[offset]))?;
        }
    }
    let visible_cities: std::collections::HashSet<_> = doc
        .entities
        .iter()
        .filter(|(&id, e)| {
            e.kind == EntityKind::Country
                && e.layer == POLITICAL
                && doc.ancestors(id).count() <= depth + 1
        })
        .filter_map(|(_, e)| e.capital)
        .collect();
    for (&id, entity) in &doc.entities {
        if doc.ancestors(id).count() > depth + 1 {
            view.entities.get_mut(&id).unwrap().label_hidden = true;
        }
        if entity.kind == EntityKind::City && (!keep_cities || !visible_cities.contains(&id)) {
            view.entities.remove(&id);
        }
    }
    Ok(view)
}

fn image(
    dir: &Path,
    filename: &str,
    doc: &Document,
    opts: &ExportOptions,
    links: bool,
) -> Result<(), String> {
    let font = png::find_system_font();
    let mut scene = build_scene(doc, &theme::evaluate(doc), opts);
    if links {
        let map = PixelMap::new(opts);
        let records = doc.diplomacy.as_ref().unwrap();
        let mut dot = |id| doc.label_path(id).map(|p| map.px(p.middle()));
        for record in records.records.values().filter(|r| r.visible(doc.year)) {
            let anchor = record.parties.iter().copied().find_map(&mut dot);
            if let Some(first) = anchor {
                let [r, g, b] = record.kind.color();
                for end in record
                    .parties
                    .iter()
                    .copied()
                    .filter_map(&mut dot)
                    .filter(|p| *p != first)
                {
                    scene.lines.push(SceneLine {
                        color: [r, g, b, 200],
                        width: 2.3,
                        pts: vec![first, end],
                        dash: Some([6.0, 4.0]),
                    });
                }
            }
        }
    }
    png::save(
        &scene,
        font.as_deref(),
        &dir.join(format!("{filename}.png")),
    )
}

fn text(
    doc: &mut Document,
    name: &str,
    location: LonLat,
    size: f32,
    color: [u8; 4],
    strong: bool,
) -> LineId {
    let key = LineId(doc.topo.alloc());
    let mut a = Annotation::new(name, location);
    a.size = size;
    a.color = color;
    a.bold = strong;
    a.italic = false;
    a.spacing = 0.0;
    doc.annotations.insert(key, a);
    key
}
fn route(doc: &mut Document, name: &str, points: Vec<LonLat>, style: Option<RouteStyle>) -> LineId {
    let id = LineId(doc.topo.alloc());
    doc.rivers.insert(
        id,
        LineFeature {
            name: name.into(),
            pts: points,
            route: style,
            legend: String::new(),
            show_name: false,
            hidden: false,
        },
    );
    id
}
fn water_rect(at: LonLat, x: f64, y: f64) -> Vec<LonLat> {
    (0..12)
        .map(|i| {
            let a = (i as f64) * std::f64::consts::TAU / 12.0;
            LonLat::new(at.lon + x * a.cos(), at.lat + y * a.sin())
        })
        .collect()
}

fn terrain(
    doc: &mut Document,
    country: EntityId,
    units: &HashMap<&str, EntityId>,
) -> Result<(), String> {
    let group = doc.add_group("三级地形与山脉");
    let domain = doc.dissolve_faces(&doc.faces_of_tree(country).into_iter().collect());
    for (name, coords, pattern, color) in [
        (
            "西南云梯高原",
            [[13, 81], [23, 76], [39, 81], [35, 94], [22, 94]],
            Pattern::Mountains,
            [147, 93, 61, 220],
        ),
        (
            "赤风大沙海",
            [[16, 13], [27, 13], [33, 20], [27, 26], [18, 25]],
            Pattern::Dots,
            [202, 145, 53, 220],
        ),
        (
            "东北林海",
            [[66, 7], [76, 7], [80, 15], [80, 27], [70, 23]],
            Pattern::Trees,
            [63, 116, 83, 210],
        ),
        (
            "中枢大平原",
            [[32, 32], [53, 32], [72, 43], [61, 55], [35, 59]],
            Pattern::Lines,
            [104, 154, 100, 155],
        ),
    ] {
        let mut rings = geography::landform(&coords, &domain).into_iter();
        let Some(outline) = rings.next() else {
            continue;
        };
        let mut mark = AreaMark::new(outline);
        mark.extra = rings.collect();
        mark.name = name.into();
        mark.color = color;
        mark.pattern = pattern;
        mark.tint = true;
        mark.dashed = false;
        mark.spacing = 14.0;
        let id = LineId(doc.topo.alloc());
        doc.areas.insert(id, mark);
        doc.set_group(&[FeatureRef::Area(id)], Some(group))?;
    }
    let peaks = [
        ("天阶圣峰 6842 m", "MQ", [17, 88]),
        ("九盘峰 4620 m", "PSh", [35, 88]),
        ("白垩顶 3420 m", "ABE", [17, 25]),
        ("北极星峰 3680 m", "NKL", [61, 6]),
        ("寒松顶 2540 m", "MC", [74, 25]),
        ("初耀角", "CY", [95, 42]),
        ("独岛天火峰 2980 m", "DDa", [88, 95]),
    ];
    for (name, code, xy) in peaks {
        let id = LineId(doc.topo.alloc());
        let mut symbol = MapSymbol::new(
            if xy == [88, 95] {
                SymbolKind::Volcano
            } else {
                SymbolKind::Peak
            },
            point_in_unit(doc, units[code], place(xy)),
        );
        symbol.name = name.into();
        doc.symbols.insert(id, symbol);
        doc.set_group(&[FeatureRef::Symbol(id)], Some(group))?;
    }
    let rivers: [(&str, &[[u32; 2]]); 7] = [
        (
            "沧澜江",
            &[
                [17, 88],
                [26, 75],
                [26, 62],
                [30, 50],
                [39, 50],
                [48, 50],
                [61, 50],
                [74, 50],
                [83, 50],
            ],
        ),
        (
            "北汉江",
            &[[35, 12], [39, 25], [43, 38], [43, 44], [48, 50]],
        ),
        ("青叶江", &[[61, 12], [52, 25], [61, 38], [61, 50]]),
        ("南江", &[[35, 88], [39, 75], [48, 69], [57, 75], [61, 88]]),
        ("西澜河", &[[26, 31], [17, 38], [4, 38]]),
        ("卡罗河", &[[43, 6], [57, 6], [70, 6], [78, 12]]),
        ("赤沙内流河", &[[17, 12], [26, 19], [30, 19]]),
    ];
    let water = doc.add_group("河湖与运河");
    for (name, coords) in rivers {
        let mut pts = geography::watercourse(coords);
        if ["沧澜江", "西澜河", "南江"].contains(&name) {
            let at_sea = |p| doc.topo.face_at(p) == map_core::BACKGROUND;
            if let Some((i, pair)) = pts
                .windows(2)
                .enumerate()
                .rev()
                .find(|(_, pair)| !at_sea(pair[0]) && at_sea(pair[1]))
            {
                let (mut land, mut sea) = (pair[0], pair[1]);
                for _ in 0..16 {
                    let mid = LonLat::new((land.lon + sea.lon) * 0.5, (land.lat + sea.lat) * 0.5);
                    if at_sea(mid) {
                        sea = mid;
                    } else {
                        land = mid;
                    }
                }
                pts.truncate(i + 1);
                pts.push(land);
            }
        }
        let id = route(doc, name, pts, None);
        doc.set_group(&[FeatureRef::River(id)], Some(water))?;
        if name == "沧澜江" || name == "卡罗河" || name == "南江" {
            let label = text(
                doc,
                name,
                place(coords[coords.len() / 2]),
                4.8,
                [26, 92, 143, 255],
                false,
            );
            doc.set_group(&[FeatureRef::Text(label)], Some(water))?;
        }
    }
    for (name, xy, size) in [
        ("澄明湖", [56, 56], 0.27),
        ("镜漠湖", [30, 19], 0.22),
        ("云镜天池", [20, 86], 0.14),
    ] {
        let center = place(xy);
        let shape = water_rect(center, size, size * 0.78);
        let lake = doc.topo.face_at(center);
        if doc.topo.faces[&lake].kind == FaceKind::Land {
            let (edited, _) =
                map_core::overlay::apply(doc, &shape, map_core::overlay::DrawIntent::CarveWater)?;
            *doc = edited;
        }
        let label = text(
            doc,
            name,
            LonLat::new(center.lon + 0.18, center.lat - 0.32),
            4.1,
            [26, 92, 143, 255],
            false,
        );
        doc.set_group(&[FeatureRef::Text(label)], Some(water))?;
    }
    let canal = route(
        doc,
        "共兴南北大运河",
        [[58, 18], [58, 34], [56, 50], [51, 65], [54, 77], [59, 90]]
            .into_iter()
            .map(place)
            .collect(),
        Some(RouteStyle {
            color: [28, 120, 122, 255],
            width: 1.45,
            dash: Dash::Dashed,
            arrow: false,
            ..Default::default()
        }),
    );
    doc.set_group(&[FeatureRef::Route(canal)], Some(water))?;
    Ok(())
}

fn position_in(doc: &Document, owner: EntityId, ideal: LonLat, descendants: bool) -> LonLat {
    let owns = |p| {
        doc.owner.get(&doc.topo.face_at(p)).is_some_and(|&id| {
            id == owner || (descendants && doc.ancestors(id).any(|x| x == owner))
        })
    };
    if owns(ideal) {
        return ideal;
    }
    for ring in 1..=24 {
        let distance = f64::from(ring) * 0.065;
        for spoke in 0..32 {
            let angle = f64::from(spoke) * std::f64::consts::TAU / 32.0;
            let p = LonLat::new(
                ideal.lon + distance * angle.cos(),
                ideal.lat + distance * angle.sin(),
            );
            if owns(p) {
                return p;
            }
        }
    }
    doc.faces_of(owner)
        .into_iter()
        .filter_map(|face| {
            let (outer, holes) = doc.topo.face_planar(face);
            let ([x, y], _) = geom::interior_point(&outer, &holes)?;
            Some(LonLat::new(x, y))
        })
        .min_by(|a, b| geom::geo_dist_deg(*a, ideal).total_cmp(&geom::geo_dist_deg(*b, ideal)))
        .unwrap_or(ideal)
}

fn point_in_unit(doc: &Document, owner: EntityId, ideal: LonLat) -> LonLat {
    position_in(doc, owner, ideal, true)
}

fn city_at(doc: &mut Document, owner: EntityId, name: &str, ideal: LonLat) -> EntityId {
    let point = position_in(doc, owner, ideal, false);
    doc.add_city(point, name)
}

fn main() -> Result<(), String> {
    let out = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "docs/tutorial".into());
    let dir = out.as_path();
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let config: Geography =
        serde_json::from_str(include_str!("../../../docs/tutorial/yuanyang-data.json"))
            .map_err(|e| e.to_string())?;
    let allies: Vec<Ally> =
        serde_json::from_str(include_str!("../../../docs/tutorial/allies-data.json"))
            .map_err(|e| e.to_string())?;
    let mut doc = Document::new("2897 · 远洋人民共和国与邻国", 9650.0);
    doc.home_view = Some(HomeView {
        projection: Projection::Equirectangular,
        central_meridian: 131.5,
        globe_latitude: 42.0,
        flat_extent: Some([61.0, 51.0]),
    });
    doc.calendar.on = true;
    doc.calendar.era = "伊瑟尔纪元".into();
    doc.style.ocean = [205, 228, 240, 255];
    doc.style.land = [237, 236, 213, 255];
    doc.style.border = [120, 68, 63, 255];
    doc.style.border_width = 1.65;
    doc.style.internal = [106, 98, 89, 220];
    doc.style.internal_width = 0.0;
    doc.style.hide_face_seams_in_export = true;
    doc.style.coast = [57, 103, 129, 255];
    doc.style.coast_width = 1.30;
    doc.style.river = [35, 109, 160, 255];
    let mut paper = Document::new_paper("龙吟港", Paper::default());
    let paper_opts = ExportOptions {
        width: 960,
        height: 720,
        title: "".into(),
        legend: false,
        ..Default::default()
    };
    image(
        dir,
        "01-blank",
        &paper,
        &ExportOptions {
            title: "平面画布 1600 × 1200 px".into(),
            ..paper_opts.clone()
        },
        false,
    )?;

    let geography::Mesh {
        topo,
        faces: pieces,
        districts: local_units,
    } = geography::redraw(&config.regions, &allies)?;
    eprintln!(
        "共享拓扑 · {} 个面 · {} 条边",
        topo.faces.len(),
        topo.edges.len()
    );
    doc.topo = topo;
    let ops = |name| output_options(name);
    image(dir, "02-coast", &doc, &ops("02  远洋洲海岸与外岛"), false)?;

    let mut sovereign = BTreeMap::<&str, EntityId>::new();
    let yy = doc.add_entity(EntityKind::Country, MAIN);
    sovereign.insert("YY", yy);
    doc.entities.get_mut(&yy).unwrap().color = [232, 212, 169, 255];
    for (code, name, color) in NEIGHBOURS {
        let id = doc.add_entity(EntityKind::Country, name);
        doc.entities.get_mut(&id).unwrap().color = color;
        sovereign.insert(code, id);
    }
    for (&code, &id) in &sovereign {
        doc.entities.get_mut(&id).unwrap().code = code.into();
    }
    for piece in &pieces {
        doc.assign(piece.face, Some(sovereign[piece.country.as_str()]))?;
    }
    let waters = doc.add_group("大洋与海湾名称");
    for (name, lon, lat, size) in [
        ("西沧海", 106.5, 42.3, 8.8),
        ("东大洋", 155.0, 47.0, 9.4),
        ("南暖海", 129.0, 19.1, 8.8),
        ("北冷海", 153.2, 56.3, 7.0),
        ("东波湾", 148.4, 39.5, 5.5),
    ] {
        let id = text(
            &mut doc,
            name,
            LonLat::new(lon, lat),
            size,
            [50, 101, 133, 255],
            false,
        );
        doc.set_group(&[FeatureRef::Text(id)], Some(waters))?;
    }
    image(dir, "03-countries", &doc, &ops("03  五国政区底图"), false)?;
    fs::write(
        dir.join("countries.geojson"),
        geojson::export_countries(&doc),
    )
    .map_err(|e| e.to_string())?;

    let mut units = BTreeMap::<&str, EntityId>::new();
    for region in &config.regions {
        let id = doc.add_subdivision(yy, &region.name)?;
        doc.entities.get_mut(&id).unwrap().code = format!("YY-{}", region.code);
        units.insert(&region.code, id);
    }
    for region in &allies {
        let id = doc.add_subdivision(sovereign[region.country.as_str()], &region.name)?;
        doc.entities.get_mut(&id).unwrap().code = region.code.clone();
        units.insert(&region.code, id);
    }
    for piece in &pieces {
        doc.assign(piece.face, Some(units[piece.province.as_str()]))?;
    }
    let fields = Fields::new(&mut doc)?;
    fields.seed(&mut doc, &sovereign, &units, &config.regions, &allies);
    doc.style.internal_width = 0.75;
    // Position the entity's own label so preview and export use one name.
    for &id in units.values() {
        let best = doc.faces_of(id).into_iter().max_by(|a, b| {
            doc.topo
                .face_area(*a, doc.radius())
                .total_cmp(&doc.topo.face_area(*b, doc.radius()))
        });
        let Some(f) = best else {
            continue;
        };
        let (outer, holes) = doc.topo.face_planar(f);
        if let Some(([x, y], _)) = geom::interior_point(&outer, &holes) {
            doc.entities.get_mut(&id).unwrap().label = Some(LabelPlace {
                at: LonLat::new(x, y),
                angle: 0.0,
                scale: 0.9,
            });
        }
    }
    image(
        dir,
        "04-provinces",
        &doc,
        &ops("04  五国 64 个一级行政区"),
        false,
    )?;
    fs::write(
        dir.join("provinces.geojson"),
        geojson::export_countries(&doc),
    )
    .map_err(|e| e.to_string())?;

    let districts = doc.add_layer_of("七大地理区", LayerKind::Exclusive);
    doc.set_active_layer(districts)?;
    let mut group_ids = Vec::new();
    for (i, name) in GROUPS.into_iter().enumerate() {
        let id = doc.add_entity(EntityKind::Country, name);
        doc.entities.get_mut(&id).unwrap().color = GROUP_COLOURS[i];
        doc.entities.get_mut(&id).unwrap().label_hidden = true;
        group_ids.push(id);
    }
    let region_group: HashMap<_, _> = config
        .regions
        .iter()
        .map(|r| (r.code.as_str(), r.group - 1))
        .collect();
    for piece in &pieces {
        if let Some(idx) = region_group.get(piece.province.as_str()) {
            doc.assign(piece.face, Some(group_ids[*idx]))?;
        }
    }
    let group_opts = ExportOptions {
        legend: true,
        ..ops("05  七大地理区 · 独占层")
    };
    image(dir, "05-regions", &doc, &group_opts, false)?;
    doc.set_active_layer(POLITICAL)?;
    let mut local_ids = units.clone();
    for unit in &local_units {
        let parent = local_ids[unit.parent.as_str()];
        let id = doc.add_subdivision(parent, &unit.name)?;
        doc.entities.get_mut(&id).unwrap().code = format!("YY-{}", unit.code);
        fields.text(&mut doc, id, fields.rank, unit.rank.name());
        local_ids.insert(&unit.code, id);
    }
    for piece in &pieces {
        doc.assign(piece.face, Some(local_ids[piece.owner.as_str()]))?;
    }
    let terrain_units: HashMap<_, _> = units.iter().map(|(&code, &id)| (code, id)).collect();
    terrain(&mut doc, yy, &terrain_units)?;
    doc.topo.validate()?;
    image(
        dir,
        "06-water",
        &overview(&doc, 1, false)?,
        &ops("06  山地、湖泊、河流与运河"),
        false,
    )?;
    eprintln!("行政区和地形已拼合");

    let mut cities = BTreeMap::new();
    for unit in &local_units {
        let id = local_ids[unit.code.as_str()];
        let city = city_at(&mut doc, id, &unit.seat, unit.at);
        if doc.entities[&city].parent != Some(id) {
            return Err(format!("{} 城市坐标不在自己的辖区", unit.seat));
        }
        doc.entities.get_mut(&city).unwrap().code = format!("YY-{}-T", unit.code);
        doc.entities.get_mut(&id).unwrap().capital = Some(city);
        fields.text(&mut doc, city, fields.rank, unit.rank.name());
        if unit.code.ends_with("-01") && unit.parent == unit.province {
            let province = units[unit.province.as_str()];
            doc.entities.get_mut(&province).unwrap().capital = Some(city);
            cities.insert(unit.province.as_str(), city);
        }
    }
    for region in config
        .regions
        .iter()
        .filter(|r| METROS.contains(&r.code.as_str()))
    {
        let id = units[region.code.as_str()];
        let city = city_at(&mut doc, id, &region.name, place(region.xy));
        doc.entities.get_mut(&city).unwrap().code = format!("YY-{}-T", region.code);
        doc.entities.get_mut(&id).unwrap().capital = Some(city);
        fields.text(&mut doc, city, fields.rank, "直辖市");
        cities.insert(region.code.as_str(), city);
    }
    doc.entities.get_mut(&yy).unwrap().capital = Some(cities["SM"]);
    let mut capitals = BTreeMap::from([
        ("皮斯城", sovereign["PS"]),
        ("麦伦市", sovereign["ML"]),
        ("海冠市", sovereign["XJ"]),
        ("东升市", sovereign["DS"]),
    ]);
    for region in &allies {
        let id = units[region.code.as_str()];
        let [x, y] = region.xy.map(f64::from);
        let city = city_at(&mut doc, id, &region.seat, geography::location(x, y));
        doc.entities.get_mut(&city).unwrap().code = format!("{}-T", region.code);
        doc.entities.get_mut(&id).unwrap().capital = Some(city);
        let capital = capitals.remove(region.seat.as_str());
        if let Some(sov) = capital {
            doc.entities.get_mut(&sov).unwrap().capital = Some(city);
        }
        fields.text(
            &mut doc,
            city,
            fields.rank,
            if capital.is_some() {
                "国家首都"
            } else {
                "一级行政区首府"
            },
        );
    }
    if !capitals.is_empty() {
        return Err("邻国首都未全部落图".into());
    }
    doc.set_year(Some(2897));
    let areas = doc.areas_by_entity().into_iter().collect();
    fields.local(&mut doc, &config.regions, &units, &areas);
    let tracks = ["GH", "SM", "XC", "LY"]
        .iter()
        .map(|code| doc.entities[&cities[code]].pos.unwrap())
        .collect();
    let transport = doc.add_group("城市与交通示意");
    let rail = route(
        &mut doc,
        "西沧—龙吟磁浮主轴",
        tracks,
        Some(RouteStyle {
            color: [161, 70, 88, 240],
            width: 2.1,
            dash: Dash::Solid,
            arrow: false,
            ..Default::default()
        }),
    );
    doc.set_group(&[FeatureRef::Route(rail)], Some(transport))?;
    let capital = LineId(doc.topo.alloc());
    let at_capital = doc.entities[&cities["SM"]].pos.unwrap();
    let mut star = MapSymbol::new(SymbolKind::Capital, at_capital);
    star.color = [176, 44, 57, 255];
    doc.symbols.insert(capital, star);
    doc.set_group(&[FeatureRef::Symbol(capital)], Some(transport))?;
    image(
        dir,
        "07-cities",
        &overview(&doc, 1, true)?,
        &ops("07  首都、一级行政区首府与交通轴线"),
        false,
    )?;

    let modern = units["SJM"];
    // The 2897 excerpt includes only the five selected countries. Move
    // the peninsula's assignments to the accession year without adding
    // another sovereign entity to the regional project.
    let shifts: Vec<_> = doc
        .faces_of_tree(modern)
        .into_iter()
        .filter_map(|face| doc.owner.get(&face).map(|&owner| (face, owner)))
        .collect();
    doc.set_year(None);
    for (face, _) in &shifts {
        doc.assign(*face, None)?;
    }
    doc.set_year(Some(2508));
    for (face, owner) in shifts {
        doc.assign(face, Some(owner))?;
    }
    doc.set_lifespan(modern, Some(2508), None)?;
    let annex = doc.add_event("圣基艾姆半岛公投并入", 2508, None);
    let pact = doc.add_event("远洋互助协约缔结", 2766, None);
    doc.add_event("远洋人民共和国建立", 2699, None);
    doc.add_event("五州改行同级行政制度", 2742, None);
    let mut historic_view = overview(&doc, 1, false)?;
    historic_view.set_year(Some(2507));
    for &id in units.values() {
        historic_view.entities.get_mut(&id).unwrap().label_hidden = true;
    }
    image(
        dir,
        "08-history",
        &historic_view,
        &ops("08  2507 年 · 圣基艾姆半岛并入前"),
        false,
    )?;
    doc.set_year(Some(2897));
    let record = Agreement {
        name: "远洋互助协约".into(),
        kind: Kind::Alliance,
        parties: ["YY", "PS", "XJ", "ML", "DS"]
            .into_iter()
            .map(|c| sovereign[c])
            .collect(),
        from: Some(2766),
        event: Some(pact),
        note: "远洋人民共和国、皮斯、辛加尔、麦伦和东升开展经济、能源及防务合作。".into(),
        ..Default::default()
    };
    doc.save_agreement(None, record)?;
    let mut diplomatic_view = overview(&doc, 0, false)?;
    for &id in units.values() {
        diplomatic_view.entities.get_mut(&id).unwrap().label_hidden = true;
    }
    diplomatic_view.style.show_cities = false;
    image(
        dir,
        "09-diplomacy",
        &diplomatic_view,
        &ops("09  远洋互助协约 · 2897 年"),
        true,
    )?;

    let ink = [37, 69, 87, 255];
    text(
        &mut paper,
        "龙吟港河口示意",
        LonLat::new(0.0, 32.0),
        18.0,
        ink,
        true,
    );
    let river = route(
        &mut paper,
        "沧澜江河口",
        vec![
            LonLat::new(-52.0, 1.0),
            LonLat::new(-16.0, 5.0),
            LonLat::new(20.0, 0.0),
            LonLat::new(52.0, -9.0),
        ],
        Some(RouteStyle {
            color: [75, 152, 193, 255],
            width: 15.0,
            dash: Dash::Solid,
            arrow: false,
            ..Default::default()
        }),
    );
    let glyph = text(
        &mut paper,
        "跨江大桥",
        LonLat::new(5.0, 17.0),
        11.0,
        ink,
        false,
    );
    let bridge = route(
        &mut paper,
        "大桥",
        vec![LonLat::new(5.0, -21.0), LonLat::new(5.0, 13.0)],
        Some(RouteStyle {
            color: [112, 78, 65, 255],
            width: 4.0,
            dash: Dash::Solid,
            arrow: false,
            ..Default::default()
        }),
    );
    let group = paper.add_group("港口示意");
    paper.set_group(
        &[
            FeatureRef::Route(river),
            FeatureRef::Route(bridge),
            FeatureRef::Text(glyph),
        ],
        Some(group),
    )?;
    text(
        &mut paper,
        "北岸 · 转运区",
        LonLat::new(-30.0, 21.0),
        11.0,
        ink,
        false,
    );
    text(
        &mut paper,
        "南岸 · 深水港区",
        LonLat::new(26.0, -23.0),
        11.0,
        ink,
        false,
    );
    image(dir, "10-board", &paper, &paper_opts, false)?;
    let book = WorldBook::edit(&mut doc);
    let board = book.add_map(paper);
    let article = book.add_article("龙吟港市与东波湾".into());
    let snapshot = fs::read(dir.join("10-board.png")).map_err(|e| e.to_string())?;
    let asset = book.add_asset(Attachment {
        name: "龙吟港河口示意.png".into(),
        bytes: map_core::underlay::Blob(Arc::new(snapshot)),
    });
    let port_article = Arc::make_mut(book.articles.get_mut(&article).unwrap());
    port_article.attachments.insert(asset);
    port_article.markdown = format!(
        "## 沧澜江入海口\n\n龙吟港市位于东波湾，沿江连接兴川市、燧明市与内陆大平原。\n\n![龙吟港河口示意](atlas-asset:{asset})\n\n[打开港口示意画板]({})",
        Node::Map(board).uri()
    );
    let port = Node::Entity(ROOT, units["LY"].0);
    book.add_link(Link::new(port, Node::Article(article), false))?;
    book.add_link(Link::new(port, Node::Map(board), true))?;
    let historic = book.add_article("圣基艾姆半岛 · 2508 年".into());
    Arc::make_mut(book.articles.get_mut(&historic).unwrap()).markdown =
        "圣基艾姆共和国经公投并入远洋联邦共和国。切换到 2507 年，可以对照并入前后五国图中的归属变化。".into();
    book.add_link(Link::new(
        Node::Event(ROOT, annex),
        Node::Article(historic),
        false,
    ))?;
    let final_opts = ExportOptions {
        scale_bar: true,
        north_arrow: true,
        ..ops("远洋人民共和国及邻国 · 伊瑟尔纪元 2897 年")
    };
    let summary = overview(&doc, 1, true)?;
    image(dir, "11-atlas", &summary, &final_opts, false)?;
    let regional_export = ExportOptions {
        width: 2800,
        height: 2220,
        ..final_opts
    };
    fs::write(
        dir.join("yuanyang.svg"),
        svg::render(&build_scene(
            &summary,
            &theme::evaluate(&summary),
            &regional_export,
        )),
    )
    .map_err(|e| e.to_string())?;
    let local_opts = ExportOptions {
        bounds: Some([
            120.5_f64.to_radians(),
            33.3_f64.to_radians(),
            136.0_f64.to_radians(),
            47.2_f64.to_radians(),
        ]),
        ..ops("12  远洋腹心区 · 地级与县级辖区")
    };
    let mut local_view = doc.clone();
    local_view.areas.clear();
    for id in sovereign.values() {
        local_view.entities.get_mut(id).unwrap().label_hidden = true;
    }
    let labels: Vec<_> = local_view.annotations.keys().copied().collect();
    for id in labels {
        local_view.annotations.get_mut(&id).unwrap().size *= 0.52;
    }
    image(dir, "12-districts", &local_view, &local_opts, false)?;
    fields.export(&doc, &dir.join("yuanyang-2897-data.csv"))?;
    doc.topo.validate()?;
    project::save(&dir.join("yuanyang.atlas"), &doc)?;
    let reloaded = project::load(&dir.join("yuanyang.atlas"))?;
    assert_eq!(reloaded.year, Some(2897));
    assert_eq!(
        reloaded.faces_of_tree(yy).len(),
        doc.faces_of_tree(yy).len()
    );
    assert_eq!(
        reloaded
            .entities
            .values()
            .filter(|e| e.parent == Some(yy) && e.kind == EntityKind::Country)
            .count(),
        42
    );
    let politics = || {
        reloaded
            .entities
            .values()
            .filter(|e| e.kind == EntityKind::Country && e.layer == POLITICAL)
    };
    assert_eq!(politics().filter(|e| e.parent.is_none()).count(), 5);
    assert_eq!(
        politics()
            .filter(|e| Some(2) == e.parent.map(|p| reloaded.ancestors(p).count() + 1))
            .count(),
        64
    );
    assert_eq!(
        reloaded
            .entities
            .values()
            .filter(|e| e.kind == EntityKind::City)
            .count(),
        350
    );
    assert_eq!(reloaded.diplomacy.as_ref().unwrap().records.len(), 1);
    eprintln!("原生工程重读成功 · 5 国 · 64 个一级行政区 · 350 个城市点");
    assert_eq!(reloaded.book.as_ref().unwrap().boards.len(), 2);
    Ok(())
}
