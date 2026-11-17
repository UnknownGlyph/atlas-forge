//! Name styles given to units are saved with the project, and files
//! written before them load with none.

use map_core::names::NameStyle;

#[test]
fn name_styles_are_saved() {
    let mut d = map_core::sample::grid_world(4, 3, 1, 2);
    let c = d.countries()[0];
    d.set_name_style(c, Some(NameStyle::Desert)).unwrap();
    let back = map_io::project::from_bytes(&map_io::project::to_bytes(&d).unwrap()).unwrap();
    assert_eq!(back.name_styles, d.name_styles);
    assert_eq!(back.name_style_for(c), Some(NameStyle::Desert));
    // an older project (none given): loads with none
    let mut old = d.clone();
    old.name_styles.clear();
    let back = map_io::project::from_bytes(&map_io::project::to_bytes(&old).unwrap()).unwrap();
    assert!(back.name_styles.is_empty());
}
