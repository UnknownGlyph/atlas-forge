//! map_render — turns a `map_core::Document` into GPU-ready geometry and draws it.
//!
//! * [`tess`]: pure CPU preparation (triangulation, subdivision, seam cutting),
//!   incremental per-face caching. Fully unit-tested, no GPU needed.
//! * [`camera`]: 2D and globe cameras, CPU picking that mirrors the shader.
//! * [`gpu`]: wgpu pipelines; one geometry drawn by any projection or the globe.

pub mod camera;
pub mod gpu;
pub mod tess;
pub mod underlay;
