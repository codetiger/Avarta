//! The GPU-ready mesh output type produced by [`crate::generate`].

use serde::{Deserialize, Serialize};

/// A triangle mesh as flat buffers, ready to hand to a GPU / Three.js.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mesh {
    /// xyz position triples.
    pub positions: Vec<f32>,
    /// xyz normal triples (per vertex, parallel to `positions`).
    pub normals: Vec<f32>,
    /// uv pairs (per vertex): u along the coil, v around the aperture.
    pub uvs: Vec<f32>,
    /// Triangle indices into the vertex arrays.
    pub indices: Vec<u32>,
    /// Layer-3 pigment field, row-major `pig_h` rows × `pig_w` columns, one byte
    /// per texel (0 = unpigmented base, 255 = full pigment). Produced by a 1-D
    /// reaction–diffusion line run along the aperture lip (φ → texture height /
    /// `v`) and stepped once per growth ring (θ → texture width / `u`) — the same
    /// growth sweep that builds the geometry, so it maps onto the mesh's existing
    /// UVs with no distortion (sample with `wrapT`/v = repeat for the closed lip,
    /// `wrapS`/u = clamp along the coil). Solid regime → a uniform field.
    pub pigment: Vec<u8>,
    /// Pigment texture width — samples along the coil (θ / `u`).
    pub pig_w: u32,
    /// Pigment texture height — samples around the aperture lip (φ / `v`).
    pub pig_h: u32,
}
