//! Thin wasm-bindgen boundary over `avarta-core`.
//!
//! JS calls `generate({ w, d, t, n, aspect, seg_theta, seg_phi })` and gets back
//! a `JsMesh` whose getters return typed arrays ready for Three.js BufferAttributes.

use avarta_core::{generate as core_generate, ShellParams, PARAM_RANGES, PIGMENT_RANGES};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn start() {
    // Readable panic messages in the browser console.
    console_error_panic_hook::set_once();
}

/// Generate a shell mesh from a plain JS params object.
#[wasm_bindgen]
pub fn generate(params: JsValue) -> Result<JsMesh, JsValue> {
    let p: ShellParams = serde_wasm_bindgen::from_value(params)
        .map_err(|e| JsValue::from_str(&format!("invalid params: {e}")))?;
    let m = core_generate(&p);
    Ok(JsMesh {
        positions: m.positions,
        normals: m.normals,
        uvs: m.uvs,
        indices: m.indices,
        pigment: m.pigment,
        pig_w: m.pig_w,
        pig_h: m.pig_h,
    })
}

/// The parameter range table — the single source of truth for every shape
/// parameter's `min`/`max`/`step`/`default`/`integer`. Returns a JS array of
/// `{ key, label, min, max, step, default, integer }` so the UI can configure
/// its sliders from Rust instead of hardcoding ranges.
#[wasm_bindgen]
pub fn param_ranges() -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(PARAM_RANGES)
        .map_err(|e| JsValue::from_str(&format!("param_ranges: {e}")))
}

/// The Layer-3 pigmentation range table — the source of truth for the
/// `pig_*` controls' `min`/`max`/`step`/`default`/`integer`, so the Appearance
/// UI configures its pigmentation sliders from Rust (mirrors `param_ranges`).
#[wasm_bindgen]
pub fn pigment_ranges() -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(PIGMENT_RANGES)
        .map_err(|e| JsValue::from_str(&format!("pigment_ranges: {e}")))
}

/// Mesh buffers. The getters hand JS `Float32Array` / `Uint32Array` /
/// `Uint8Array` copies; call `.free()` once you've read them to release the
/// wasm-side allocation.
#[wasm_bindgen]
pub struct JsMesh {
    positions: Vec<f32>,
    normals: Vec<f32>,
    uvs: Vec<f32>,
    indices: Vec<u32>,
    pigment: Vec<u8>,
    pig_w: u32,
    pig_h: u32,
}

#[wasm_bindgen]
impl JsMesh {
    #[wasm_bindgen(getter)]
    pub fn positions(&self) -> Vec<f32> {
        self.positions.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn normals(&self) -> Vec<f32> {
        self.normals.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn uvs(&self) -> Vec<f32> {
        self.uvs.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn indices(&self) -> Vec<u32> {
        self.indices.clone()
    }
    /// Layer-3 pigment field, row-major `pig_h`×`pig_w`, one byte per texel
    /// (0 = base, 255 = full pigment). Map it through the palette via the UVs.
    #[wasm_bindgen(getter)]
    pub fn pigment(&self) -> Vec<u8> {
        self.pigment.clone()
    }
    /// Pigment texture width — samples along the coil (θ / u).
    #[wasm_bindgen(getter)]
    pub fn pig_w(&self) -> u32 {
        self.pig_w
    }
    /// Pigment texture height — samples around the aperture lip (φ / v).
    #[wasm_bindgen(getter)]
    pub fn pig_h(&self) -> u32 {
        self.pig_h
    }
}
