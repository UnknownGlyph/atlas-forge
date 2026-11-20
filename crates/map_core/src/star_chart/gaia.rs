//! A bright sample from Gaia DR3, recorded in Galactic coordinates.
use glam::Vec3;
use std::sync::OnceLock;

const SOURCE: &str = include_str!("../../../../assets/astronomy/gaia-dr3-bright.csv");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Center {
    #[default]
    Galaxy,
    SolarSystem,
}

pub fn solar_origin(radius_kly: f32, angle_degrees: f32) -> Vec3 {
    let (y, x) = angle_degrees.to_radians().sin_cos();
    Vec3::new(x, 0.0, y) * radius_kly
}

#[derive(Clone, Copy, Debug)]
pub struct ObservedStar {
    pub source_id: u64,
    pub galactic_longitude: f32,
    pub galactic_latitude: f32,
    pub distance_kly: f32,
    pub magnitude: f32,
    pub color_index: f32,
}

impl ObservedStar {
    pub fn position(self, radius_kly: f32, home_angle: f32) -> Vec3 {
        self.coordinates(Center::Galaxy, radius_kly, home_angle)
    }

    pub fn coordinates(self, center: Center, radius_kly: f32, home_angle: f32) -> Vec3 {
        let (along, inward) = self.galactic_longitude.to_radians().sin_cos();
        let (up, plane) = self.galactic_latitude.to_radians().sin_cos();
        let (y, x) = home_angle.to_radians().sin_cos();
        let radial = Vec3::new(x, 0.0, y);
        let tangent = Vec3::new(-y, 0.0, x);
        let relative = self.distance_kly * (plane * (along * tangent - inward * radial) + Vec3::Y * up);
        match center {
            Center::Galaxy => solar_origin(radius_kly, home_angle) + relative,
            Center::SolarSystem => relative,
        }
    }
}

pub fn stars() -> &'static [ObservedStar] {
    static CATALOG: OnceLock<Vec<ObservedStar>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        SOURCE
            .lines()
            .skip(1)
            .map(|line| {
                let mut fields = line.split(',');
                let id = fields.next().unwrap().parse().unwrap();
                let mut number = || fields.next().unwrap().parse().unwrap();
                ObservedStar {
                    source_id: id,
                    galactic_longitude: number(),
                    galactic_latitude: number(),
                    distance_kly: number(),
                    magnitude: number(),
                    color_index: number(),
                }
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_contains_eight_thousand_distinct_observations() {
        let stars = stars();
        assert_eq!(stars.len(), 8000);
        assert_eq!(
            stars
                .iter()
                .map(|s| s.source_id)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            8000
        );
        assert!(stars.iter().all(|s| (0.0..6.524).contains(&s.distance_kly)
            && (-90.0..=90.0).contains(&s.galactic_latitude)
            && s.position(26.0, 210.0).is_finite()));
    }

    #[test]
    fn a_star_keeps_its_measured_solar_distance_in_both_frames() {
        let star = stars()[0];
        let solar = star.coordinates(Center::SolarSystem, 26.0, 210.0);
        let galaxy = star.coordinates(Center::Galaxy, 26.0, 210.0);
        assert!((solar.length() - star.distance_kly).abs() < 1e-5);
        assert!((galaxy - solar - solar_origin(26.0, 210.0)).length() < 1e-5);
    }
}
