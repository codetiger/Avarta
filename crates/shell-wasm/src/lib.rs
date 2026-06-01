//! Thin wasm-bindgen boundary over `shell-core`.
//!
//! JS calls `generate({ w, d, t, n, aspect, seg_theta, seg_phi })` and gets back
//! a `JsMesh` whose getters return typed arrays ready for Three.js BufferAttributes.

use shell_core::{generate as core_generate, ShellParams};
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
    })
}

/// Mesh buffers. The getters hand JS `Float32Array` / `Uint32Array` copies; call
/// `.free()` once you've read them to release the wasm-side allocation.
#[wasm_bindgen]
pub struct JsMesh {
    positions: Vec<f32>,
    normals: Vec<f32>,
    uvs: Vec<f32>,
    indices: Vec<u32>,
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
}
