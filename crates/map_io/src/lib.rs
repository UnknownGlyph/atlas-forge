//! map_io — project files and interchange formats.
//!
//! * [`project`]: native `.atlas` project format (versioned, compressed, atomic save)
//! * [`recovery`]: autosave snapshots and crash detection via OS file locks
//! * [`geojson`]: GeoJSON export (per face / per country) and simple import
//! * [`table`]: CSV attribute import/export joined by stable entity codes
//! * [`scene`]: projection-aware vector scene shared by [`svg`] and [`png`] export

pub mod atlas;
pub mod batch;
pub mod chart_csv;
pub mod checkpoints;
pub mod cloud;
pub mod dossier;
pub mod clip;
pub mod fonts;
pub mod geojson;
pub mod gaia_csv;
pub mod model;
pub mod pdf;
pub mod png;
pub mod prefs;
pub mod project;
pub mod recovery;
pub mod scene;
pub mod shapefile;
pub mod svg;
pub mod table;
pub mod underlay;
