# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**Avarta** is a mathematical generator for spiral
seashell shapes (snails, *Nautilus*, augers, …). Pure-Rust mesh math compiled to WebAssembly, rendered
by a Vite + Three.js web app (real-time PBR raster viewport plus an on-demand path-traced "Render" mode).

See `parameters.md` for the full parameter model and `scope.md` for which shell shapes are in/out of scope
(single-tube, single-aperture, spirally-grown shells only — no paired/segmented/radial body plans).

## Commands

```sh
# Test the pure math natively (fast, no wasm toolchain needed)
cargo test -p avarta-core
cargo test -p avarta-core <test_name>          # a single test, e.g. clamp_pins_out_of_range_and_rounds_integers

# Rebuild the wasm into web/pkg/ AFTER ANY Rust change (add --release for prod-sized output)
wasm-pack build crates/avarta-wasm --target web --out-dir ../../web/pkg

# Run the web dev server (installs deps on first run) → http://localhost:8090
cd web && npm install && npm run dev

# Production web build (outputs web/dist/, base "./" for the GitHub Pages subpath)
cd web && npm run build
```

Vite hot-reloads JS/HTML, but the `.wasm` is treated as a static asset — **a Rust change is invisible
until you re-run `wasm-pack`**. Prerequisites: Rust stable + `rustup target add wasm32-unknown-unknown`,
`wasm-pack`, Node 18+.

CI (`.github/workflows/deploy.yml`, on push to `main`): native tests → `wasm-pack build --release` →
`npm ci && npm run build` → publish `web/dist/` to GitHub Pages.

## Architecture

Three crates/dirs, each consuming the previous one's *built output*:

- **`crates/avarta-core`** — pure Rust, no JS/wasm deps, so it's unit-testable with plain `cargo test`.
  The whole generator is one function: `generate(&ShellParams) -> Mesh`. `Mesh` is flat GPU-ready buffers
  (positions/normals/uvs/indices) plus a `pigment` byte field (`pig_w`×`pig_h`).
- **`crates/avarta-wasm`** — thin `wasm-bindgen` adapter (`cdylib`). Exposes `generate(params)`,
  `param_ranges()`, `pigment_ranges()` to JS. Getters hand back typed-array copies; JS must call `.free()`.
- **`web/`** — Vite app. `<avarta-viewer>` (`avarta-viewer.js`) is a custom element rendering into shadow
  DOM; `index.html` builds the control panel. `three` + `three-gpu-pathtracer` are bundled from npm as
  **one deduped `three` instance** (required by the path tracer); the path tracer is lazily imported.

### The four layers (where each one lives)

The model is layered (see `parameters.md`), and the layer boundary is also a code boundary:

1. **Coiling geometry** (Raup W/D/T + n + aspect) and 2. **Ornamentation** (ribs, cords, projections,
   varices, seeded jitter) — computed in Rust `generate`, sweeping an elliptical aperture along a
   logarithmic helico-spiral.
3. **Pigmentation** — a 1-D Gray–Scott **reaction–diffusion** line run along the aperture lip (φ) and
   stepped once per growth ring (θ), i.e. the *same* growth sweep as the geometry. Output as the `pigment`
   field; because it shares the sweep, it maps onto the mesh's existing UVs (u=θ along coil, v=φ around lip)
   with no distortion. Lives in Rust (`pigment_field` in `avarta-core`).
4. **Palette + material finish** — applied **viewer-side in JS** (`buildPigmentTexture`, `_applyMaterial`).
   The pigment byte field is mapped through a 3-stop palette into a colour map; the base material colour
   stays white so the map carries all colour.

Consequence for editing: **shape/ornament/pigment-pattern changes require a Rust rebuild; recolouring and
material finish do not.** In the viewer, `setParams()` regenerates geometry + reruns the RD sim;
`setMaterial()` / `setPalette()` / `setRenderMode()` are look-only (cheap re-bake, no `generate`).

### Single source of truth: parameter ranges

`PARAM_RANGES` (19 shape params) and `PIGMENT_RANGES` (6 pigment params) in `avarta-core/src/lib.rs` are
the **only** place a parameter's `min`/`max`/`step`/`default`/`integer` is defined. Both `generate`
(via `clamp_in_place`) and the web UI (via the `param_ranges()` / `pigment_ranges()` wasm exports) read
this table, so they cannot drift, and any input is clamped into range before the mesh math runs. The
JS `FALLBACK_RANGES` / `FALLBACK_PIGMENT` in `index.html` are used *only* if the wasm fails to load.

**To add or change a user-facing parameter:** add the field to `ShellParams` (with a `#[serde(default)]`),
add its row to the relevant range table, and extend `clamp_in_place`. The `param_table_covers_every_field`
test enforces that the table and `Default` stay consistent — `clamp_in_place` panics on a missing key, so
the test is the guard. The UI sliders, clamping, and harness all follow automatically.

The mesh is unit-normalised, centred, and oriented (spire up, cone down, aperture facing +Z) **in Rust**,
so the viewer needs no mesh transform and the BVH / path tracer get precision-friendly coordinates.
Tessellation (`seg_theta`/`seg_phi`) is auto-refined from ornament frequency and bounded by a vertex
budget — it is internal, not a user parameter.

## Shape-comparison harness (`harness/`)

A Python + Node test rig that renders each species and pairs it with a real reference photo in
`report.html` for **human** shape-matching (it renders; you judge). It reaches the generator **through the
prebuilt `web/pkg` wasm, exactly like the web page** (`extract_mesh.mjs` calls the same `generate`) and
**never rebuilds or modifies the Rust crates**. `species.json` is the catalog (params + `coverage` rating
+ notes); `coverage` doubles as the feature backlog. See `harness/README.md`. (Note: that README still
refers to `shell_wasm.js`; the actual built module is `avarta_wasm.js`.)

Run: `cd harness && python3 -m venv .venv && . .venv/bin/activate && pip install -r requirements.txt`,
then `python fetch_references.py` (once) and `python render_catalog.py` (`--only <slug>` for a subset).
