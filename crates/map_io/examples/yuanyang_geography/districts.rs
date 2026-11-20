use super::{location, partition, Shape};
use crate::Region;
use geo::Area;
use map_core::{
    geom::{LonLat, P2},
    names::{self, NameKind, NameStyle},
};
use std::collections::HashSet;

pub const METROS: [&str; 4] = ["SM", "GH", "XC", "LY"];
pub const PREFECTURES: usize = 90;
const DISTRICTS: usize = 20;
pub const COUNTIES: usize = 234;

#[derive(Clone, Copy)]
pub enum Rank {
    City,
    District,
    County,
}

impl Rank {
    pub fn name(self) -> &'static str {
        match self {
            Self::City => "地级市",
            Self::District => "地区",
            Self::County => "县级市",
        }
    }
}

pub struct Unit {
    pub code: String,
    pub parent: String,
    pub province: String,
    pub name: String,
    pub seat: String,
    pub at: LonLat,
    pub rank: Rank,
}

pub struct AreaPiece {
    pub country: String,
    pub province: String,
    pub owner: String,
    pub shape: Shape,
}

// Largest-remainder allocation keeps the requested count and the reference
// totals even when province sizes differ substantially.
pub fn share(total: usize, weights: &[f64]) -> Vec<usize> {
    let sum: f64 = weights.iter().sum();
    let amounts: Vec<_> = weights.iter().map(|w| w * total as f64 / sum).collect();
    let mut base: Vec<_> = amounts.iter().map(|a| a.floor() as usize).collect();
    let remaining = total - base.iter().sum::<usize>();
    let mut remainders: Vec<_> = (0..weights.len()).collect();
    remainders.sort_by(|&a, &b| amounts[b].fract().total_cmp(&amounts[a].fract()));
    for i in remainders.into_iter().take(remaining) {
        base[i] += 1;
    }
    base
}

pub fn build(provinces: Vec<(&Region, Shape)>) -> Result<(Vec<AreaPiece>, Vec<Unit>), String> {
    let editable: Vec<_> = provinces
        .iter()
        .filter(|(r, _)| !METROS.contains(&r.code.as_str()))
        .map(|(r, shape)| (shape.unsigned_area().sqrt() * (r.pop as f64).powf(0.25)).max(1.0))
        .collect();
    let prefecture_counts = share(PREFECTURES - editable.len(), &editable);
    let mut prefecture_counts = prefecture_counts.into_iter();
    let reserved: HashSet<_> = provinces
        .iter()
        .flat_map(|(r, _)| [r.seat.clone(), r.name.clone()])
        .collect();
    let mut names = names::unique(
        NameStyle::Huaxia,
        NameKind::Region,
        (PREFECTURES + COUNTIES) * 3,
        2897_0522,
        &reserved,
    )
    .into_iter();
    let mut areas = Vec::new();
    let mut prefectures = Vec::new();
    let mut used = reserved;
    let mut district_count = 0;
    for (r, shape) in provinces {
        if METROS.contains(&r.code.as_str()) {
            areas.push(piece(&r.code, &r.code, shape));
            continue;
        }
        let n = 1 + prefecture_counts.next().ok_or("地级区数量不匹配")?;
        let partitions =
            partition(&shape, r.xy.map(f64::from), n).map_err(|e| format!("{} {e}", r.name))?;
        for (i, (site, cell)) in partitions.into_iter().enumerate() {
            let code = format!("{}-{:02}", r.code, i + 1);
            let stem = if i == 0 {
                r.seat.trim_end_matches('市').to_owned()
            } else {
                next_name(&mut names, &mut used)?
            };
            let rank = if i > 0 && district_count < DISTRICTS && (r.group != 1 || i > 1) {
                district_count += 1;
                Rank::District
            } else {
                Rank::City
            };
            let name = match rank {
                Rank::District => format!("{stem}地区"),
                _ => format!("{stem}市"),
            };
            let seat = if i == 0 {
                r.seat.clone()
            } else {
                format!("{stem}市")
            };
            prefectures.push((
                unit(code, &r.code, &r.code, name, seat, site, rank),
                site,
                cell,
            ));
        }
    }
    if district_count != DISTRICTS {
        return Err(format!("地区数量 {district_count} 与规划不符"));
    }
    let weights: Vec<_> = prefectures
        .iter()
        .map(|(_, _, shape)| shape.unsigned_area().sqrt())
        .collect();
    let counts = share(COUNTIES - prefectures.len(), &weights);
    let mut units = Vec::new();
    for ((prefecture, site, shape), extra) in prefectures.into_iter().zip(counts) {
        let partitions =
            partition(&shape, site, 2 + extra).map_err(|e| format!("{} {e}", prefecture.name))?;
        let province = prefecture.province.clone();
        let code = prefecture.code.clone();
        let mut partitions = partitions.into_iter();
        let (_, core) = partitions.next().ok_or("地级区缺少中心")?;
        areas.push(piece(&province, &code, core));
        units.push(prefecture);
        for (i, (site, cell)) in partitions.enumerate() {
            let county = format!("{code}-{:02}", i + 1);
            let name = format!("{}市", next_name(&mut names, &mut used)?);
            areas.push(piece(&province, &county, cell));
            units.push(unit(
                county,
                &code,
                &province,
                name.clone(),
                name,
                site,
                Rank::County,
            ));
        }
    }
    Ok((areas, units))
}

fn piece(province: &str, owner: &str, shape: Shape) -> AreaPiece {
    AreaPiece {
        country: "YY".into(),
        province: province.into(),
        owner: owner.into(),
        shape,
    }
}

fn next_name(
    names: &mut impl Iterator<Item = String>,
    used: &mut HashSet<String>,
) -> Result<String, String> {
    for name in names {
        let stem = name
            .trim_end_matches("地区")
            .trim_end_matches(['郡', '州', '领', '省', '县', '道']);
        if used.insert(format!("{stem}市")) {
            return Ok(stem.to_owned());
        }
    }
    Err("城市名称池不足".into())
}

fn unit(
    code: String,
    parent: &str,
    province: &str,
    name: String,
    seat: String,
    [x, y]: P2,
    rank: Rank,
) -> Unit {
    Unit {
        code,
        parent: parent.into(),
        province: province.into(),
        name,
        seat,
        at: location(x, y),
        rank,
    }
}
