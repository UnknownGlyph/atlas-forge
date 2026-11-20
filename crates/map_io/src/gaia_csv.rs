use map_core::star_chart::gaia::{self, Center};
use std::path::Path;

pub fn export(path: &Path, center: Center, radius_kly: f32, angle: f32) -> Result<(), String> {
    let mut sheet = csv::Writer::from_writer(Vec::new());
    sheet
        .write_record([
            "source_id",
            "origin",
            "x_kly",
            "y_kly",
            "z_kly",
            "distance_from_sun_kly",
            "galactic_longitude_deg",
            "galactic_latitude_deg",
            "g_magnitude",
        ])
        .map_err(|e| e.to_string())?;
    let origin = match center {
        Center::Galaxy => "galactic_center",
        Center::SolarSystem => "solar_system",
    };
    for star in gaia::stars() {
        let p = star.coordinates(center, radius_kly, angle);
        sheet
            .serialize((
                star.source_id,
                origin,
                p.x,
                p.y,
                p.z,
                star.distance_kly,
                star.galactic_longitude,
                star.galactic_latitude,
                star.magnitude,
            ))
            .map_err(|e| e.to_string())?;
    }
    let data = sheet.into_inner().map_err(|e| e.to_string())?;
    crate::recovery::write_atomic(path, &data).map_err(|e| e.to_string())
}

#[test]
fn the_same_observations_export_in_two_reference_frames() {
    use map_core::star_chart::gaia::{solar_origin, stars, Center};
    let path = std::env::temp_dir().join(format!("atlas-gaia-check-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    let g = path.join("galaxy.csv");
    let s = path.join("solar.csv");
    let (radius, angle) = (26.0, 210.0);
    export(&g, Center::Galaxy, radius, angle).unwrap();
    export(&s, Center::SolarSystem, radius, angle).unwrap();
    let mut galaxy = csv::Reader::from_path(&g).unwrap();
    let mut solar = csv::Reader::from_path(&s).unwrap();
    let expected = solar_origin(radius, angle).to_array();
    let mut count = 0;
    for (a, b) in galaxy.records().zip(solar.records()) {
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(&a[0], &b[0]);
        assert_eq!(&a[1], "galactic_center");
        assert_eq!(&b[1], "solar_system");
        for (axis, &value) in expected.iter().enumerate() {
            let g = a[axis + 2].parse::<f32>().unwrap();
            let s = b[axis + 2].parse::<f32>().unwrap();
            assert!((g - s - value).abs() < 0.000015);
        }
        assert_eq!(&a[5], &b[5]);
        count += 1;
    }
    assert_eq!(count, stars().len());
    std::fs::remove_file(g).unwrap();
    std::fs::remove_file(s).unwrap();
}
