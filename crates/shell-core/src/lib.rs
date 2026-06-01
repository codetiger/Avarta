//! Pure mesh generation for spiral shells.
//!
//! Layer-1 coiling geometry only (see `parameters.md`): a single tube swept
//! along a logarithmic helico-spiral. Ornamentation / pigment / colour are not
//! handled here — they hook in later by modulating the aperture in `generate`.
//!
//! No JS/wasm dependencies, so this crate is unit-testable with plain `cargo test`.

use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

/// Layer-1 coiling parameters (Raup W/D/T + practical extras).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellParams {
    /// Whorl expansion rate per revolution (> 1). Snails ~1.1–3, Nautilus ~3.
    pub w: f32,
    /// Openness: 0 = whorls touch the axis (tight), → 1 = open coil / wide umbilicus.
    pub d: f32,
    /// Translation rate along the axis. 0 = planispiral (flat), higher = tall spire.
    pub t: f32,
    /// Number of whorls (revolutions).
    pub n: f32,
    /// Aperture aspect ratio (radial semi-axis / axial semi-axis). 1 = circular tube.
    #[serde(default = "default_aspect")]
    pub aspect: f32,

    // --- Layer 2: rib / wave ornamentation (see parameters.md) ---
    /// Axial ribs/waves: number across the whorl, per revolution.
    #[serde(default)]
    pub rib_ax_count: f32,
    /// Axial amplitude as a fraction of tube radius. High = structural wave.
    #[serde(default)]
    pub rib_ax_amp: f32,
    /// Spiral cords: number around the aperture cross-section.
    #[serde(default)]
    pub rib_sp_count: f32,
    /// Spiral amplitude as a fraction of tube radius.
    #[serde(default)]
    pub rib_sp_amp: f32,
    /// Profile: 0 = smooth sine wave, 1 = sharp knife-edge ridge.
    #[serde(default)]
    pub rib_sharp: f32,

    // --- projections: the nodule → spine continuum ---
    /// Projections per whorl (count along the coil). 0 = none.
    #[serde(default)]
    pub proj_count: f32,
    /// Rows of projections around the aperture. 1 = single row (spine-like),
    /// ≥2 = evenly spaced rows (nodulose).
    #[serde(default)]
    pub proj_rows: f32,
    /// Position of the first row around the aperture, radians (0..2π).
    #[serde(default)]
    pub proj_pos: f32,
    /// Projection size (height/length) as a fraction of tube radius.
    #[serde(default)]
    pub proj_size: f32,
    /// Sharpness: 0 = rounded blunt bead (nodule), 1 = narrow needle (spine).
    #[serde(default)]
    pub proj_sharp: f32,

    // --- varices ---
    /// Varices (prominent transverse ridges) per whorl. 0 = none.
    #[serde(default)]
    pub varix_count: f32,
    /// Varix prominence as a fraction of tube radius.
    #[serde(default)]
    pub varix_amp: f32,

    // --- randomness (seeded → reproducible) ---
    /// Random seed (integer-valued). Same seed + params → identical shape.
    #[serde(default)]
    pub seed: f32,
    /// Randomness amount, 0..1. 0 = perfectly uniform (seed has no effect).
    #[serde(default)]
    pub jitter: f32,

    /// Tessellation: segments per revolution along the coil.
    #[serde(default = "default_seg_theta")]
    pub seg_theta: u32,
    /// Tessellation: segments around the aperture cross-section.
    #[serde(default = "default_seg_phi")]
    pub seg_phi: u32,
}

fn default_aspect() -> f32 {
    1.0
}
fn default_seg_theta() -> u32 {
    96
}
fn default_seg_phi() -> u32 {
    48
}

impl Default for ShellParams {
    fn default() -> Self {
        Self {
            w: 2.0,
            d: 0.15,
            t: 1.5,
            n: 5.0,
            aspect: 1.0,
            rib_ax_count: 0.0,
            rib_ax_amp: 0.0,
            rib_sp_count: 0.0,
            rib_sp_amp: 0.0,
            rib_sharp: 0.0,
            proj_count: 0.0,
            proj_rows: 0.0,
            proj_pos: 0.0,
            proj_size: 0.0,
            proj_sharp: 0.0,
            varix_count: 0.0,
            varix_amp: 0.0,
            seed: 0.0,
            jitter: 0.0,
            seg_theta: 96,
            seg_phi: 48,
        }
    }
}

/// Metadata for one user-facing shape parameter — the single source of truth
/// for its valid range, slider configuration, and clamping behaviour.
///
/// The web UI reads this table (via the wasm `param_ranges()` export) to
/// configure its sliders, and `generate` clamps every input to it, so a value
/// can never be out of range regardless of where it came from.
#[derive(Debug, Clone, Serialize)]
pub struct ParamRange {
    /// `ShellParams` field name (e.g. `"rib_ax_count"`). MUST match exactly.
    pub key: &'static str,
    /// Human-readable label for the UI slider.
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub default: f32,
    /// Integer-valued: clamping rounds to the nearest integer; the UI shows no
    /// decimals.
    pub integer: bool,
}

/// The 19 user-facing shape parameters, in UI display order. This is the single
/// source of truth for every parameter's range.
///
/// Bounds are chosen to span the real diversity of **coiled** shells (Raup 1966
/// morphospace + malacology): augers/*Turritella* reach ~20 whorls, open coils
/// reach D≈0.95, spiny *Murex* and fine spiral cords push the ornament counts.
/// The degenerate high-W limpet/single-bivalve-valve case is intentionally out
/// of scope. `seg_theta`/`seg_phi` are absent — they are internal and
/// auto-derived from the ornament frequency, not user parameters.
pub const PARAM_RANGES: &[ParamRange] = &[
    // --- Layer 1: coiling geometry ---
    ParamRange { key: "w",            label: "Whorl expansion W",       min: 1.05, max: 8.0,    step: 0.01, default: 2.0,  integer: false },
    ParamRange { key: "d",            label: "Openness D",              min: 0.0,  max: 0.95,   step: 0.01, default: 0.15, integer: false },
    ParamRange { key: "t",            label: "Translation T",           min: 0.0,  max: 12.0,   step: 0.01, default: 1.5,  integer: false },
    ParamRange { key: "n",            label: "Whorls n",                min: 0.5,  max: 20.0,   step: 0.1,  default: 5.0,  integer: false },
    ParamRange { key: "aspect",       label: "Aperture aspect",         min: 0.3,  max: 4.0,    step: 0.01, default: 1.0,  integer: false },
    // --- Layer 2: ribs / cords / waves ---
    ParamRange { key: "rib_ax_count", label: "Axial ribs / waves",      min: 0.0,  max: 40.0,   step: 1.0,  default: 0.0,  integer: true  },
    ParamRange { key: "rib_ax_amp",   label: "Axial amplitude",         min: 0.0,  max: 0.6,    step: 0.01, default: 0.0,  integer: false },
    ParamRange { key: "rib_sp_count", label: "Spiral cords",            min: 0.0,  max: 60.0,   step: 1.0,  default: 0.0,  integer: true  },
    ParamRange { key: "rib_sp_amp",   label: "Spiral amplitude",        min: 0.0,  max: 0.6,    step: 0.01, default: 0.0,  integer: false },
    ParamRange { key: "rib_sharp",    label: "Profile (wave → ridge)",  min: 0.0,  max: 1.0,    step: 0.01, default: 0.0,  integer: false },
    // --- projections (nodules → spines) ---
    ParamRange { key: "proj_count",   label: "Projections / whorl",     min: 0.0,  max: 30.0,   step: 1.0,  default: 0.0,  integer: true  },
    ParamRange { key: "proj_rows",    label: "Rows around aperture",    min: 0.0,  max: 5.0,    step: 1.0,  default: 0.0,  integer: true  },
    ParamRange { key: "proj_pos",     label: "Row position (φ)",        min: 0.0,  max: 6.2832, step: 0.01, default: 0.0,  integer: false },
    ParamRange { key: "proj_size",    label: "Size",                    min: 0.0,  max: 1.2,    step: 0.01, default: 0.0,  integer: false },
    ParamRange { key: "proj_sharp",   label: "Sharpness (bead → needle)", min: 0.0, max: 1.0,   step: 0.01, default: 0.0,  integer: false },
    // --- varices ---
    ParamRange { key: "varix_count",  label: "Varices / whorl",         min: 0.0,  max: 6.0,    step: 1.0,  default: 0.0,  integer: true  },
    ParamRange { key: "varix_amp",    label: "Varix prominence",        min: 0.0,  max: 0.5,    step: 0.01, default: 0.0,  integer: false },
    // --- randomness ---
    ParamRange { key: "seed",         label: "Seed",                    min: 0.0,  max: 9999.0, step: 1.0,  default: 0.0,  integer: true  },
    ParamRange { key: "jitter",       label: "Jitter",                  min: 0.0,  max: 1.0,    step: 0.01, default: 0.0,  integer: false },
];

impl ShellParams {
    /// Clamp every user-facing field to its `PARAM_RANGES` bound, rounding the
    /// integer-valued fields. `seg_theta`/`seg_phi` are left untouched (internal,
    /// auto-derived). After this, every shape field is guaranteed in range.
    pub fn clamp_in_place(&mut self) {
        fn fix(v: f32, r: &ParamRange) -> f32 {
            let c = v.clamp(r.min, r.max);
            if r.integer {
                c.round()
            } else {
                c
            }
        }
        // Panics only if PARAM_RANGES is missing a key — caught by the unit test
        // `param_table_covers_every_field`, so it can never reach production.
        let g = |k| PARAM_RANGES.iter().find(|r| r.key == k).expect("missing PARAM_RANGES key");
        self.w = fix(self.w, g("w"));
        self.d = fix(self.d, g("d"));
        self.t = fix(self.t, g("t"));
        self.n = fix(self.n, g("n"));
        self.aspect = fix(self.aspect, g("aspect"));
        self.rib_ax_count = fix(self.rib_ax_count, g("rib_ax_count"));
        self.rib_ax_amp = fix(self.rib_ax_amp, g("rib_ax_amp"));
        self.rib_sp_count = fix(self.rib_sp_count, g("rib_sp_count"));
        self.rib_sp_amp = fix(self.rib_sp_amp, g("rib_sp_amp"));
        self.rib_sharp = fix(self.rib_sharp, g("rib_sharp"));
        self.proj_count = fix(self.proj_count, g("proj_count"));
        self.proj_rows = fix(self.proj_rows, g("proj_rows"));
        self.proj_pos = fix(self.proj_pos, g("proj_pos"));
        self.proj_size = fix(self.proj_size, g("proj_size"));
        self.proj_sharp = fix(self.proj_sharp, g("proj_sharp"));
        self.varix_count = fix(self.varix_count, g("varix_count"));
        self.varix_amp = fix(self.varix_amp, g("varix_amp"));
        self.seed = fix(self.seed, g("seed"));
        self.jitter = fix(self.jitter, g("jitter"));
    }

    /// A clamped copy — see [`ShellParams::clamp_in_place`].
    pub fn clamped(&self) -> ShellParams {
        let mut p = self.clone();
        p.clamp_in_place();
        p
    }
}

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
}

/// Rib / wave profile in `[-1, 1]`. `sharp` morphs a smooth cosine **wave** (0)
/// into a narrow knife-edge **ridge** (1) by peaking a raised cosine.
#[inline]
fn ribbed(x: f32, sharp: f32) -> f32 {
    let c = 0.5 * (x.cos() + 1.0); // raised cosine, 0..1
    let p = 1.0 + sharp.clamp(0.0, 1.0) * 8.0; // exponent narrows the peak
    2.0 * c.powf(p) - 1.0
}

/// Positive periodic lobe in `[0, 1]`, peaking at multiples of 2π. `power`
/// narrows it: low = broad bump (nodule / varix), high = narrow spike (needle).
#[inline]
fn lobe(x: f32, power: f32) -> f32 {
    x.cos().max(0.0).powf(power)
}

// --- seeded value noise (pure function of the seed → reproducible randomness) ---

#[inline]
fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// Hash `(seed, i)` → f32 in `[0, 1)`.
#[inline]
fn rand01(seed: u32, i: i32) -> f32 {
    let h = hash_u32(seed ^ hash_u32(i as u32));
    (h >> 8) as f32 / ((1u32 << 24) as f32)
}

/// Hash `(seed, i)` → f32 in `[-1, 1)`.
#[inline]
fn rand_signed(seed: u32, i: i32) -> f32 {
    rand01(seed, i) * 2.0 - 1.0
}

/// Smooth 1-D value noise in `[-1, 1]` (lattice values, smoothstep-interpolated).
#[inline]
fn noise1(seed: u32, x: f32) -> f32 {
    let i = x.floor();
    let f = x - i;
    let ii = i as i32;
    let a = rand01(seed, ii);
    let b = rand01(seed, ii + 1);
    let u = f * f * (3.0 - 2.0 * f);
    (a + (b - a) * u) * 2.0 - 1.0
}

/// Generate a shell surface by sweeping an elliptical aperture along a
/// logarithmic helico-spiral.
///
/// `theta` runs along the coil (0 .. 2π·n); `phi` runs around the aperture.
/// The aperture and its distance from the axis both scale by `g = W^(theta/2π)`,
/// which keeps the form self-similar (why shells are logarithmic spirals).
pub fn generate(p: &ShellParams) -> Mesh {
    // Single, total clamp: every shape field is now guaranteed within its
    // `PARAM_RANGES` bound, so the rest of the function (and the tessellation /
    // mesh math below) never sees an out-of-range value.
    let p = p.clamped();
    let p = &p;
    let n = p.n;
    let d = p.d;
    let aspect = p.aspect;
    let w = p.w;

    // Feature profile exponents — also drive how narrow each feature is.
    const VARIX_POWER: f32 = 3.0; // rounded raised ridges
    let rib_power = 1.0 + p.rib_sharp.clamp(0.0, 1.0) * 8.0; // matches `ribbed`
    let proj_power = 2.0 + p.proj_sharp.clamp(0.0, 1.0).powi(2) * 40.0; // matches `lobe` use
    let proj_active = p.proj_size.abs() > 1e-6 && p.proj_count > 0.5 && p.proj_rows > 0.5;

    // Auto-refine tessellation per direction from the *impacting* parameters:
    // a feature needs enough samples across each peak, and sharper peaks (higher
    // profile power) are narrower — so required resolution ≈ frequency · √power,
    // not a flat per-feature constant. This is why a sharp projection needs far
    // denser segments than a blunt one. Plain shells keep the cheap base res; a
    // global vertex budget bounds many-whorl × sharp combos.
    const SPC: f32 = 16.0; // samples per feature-cycle at power 1
    const MAX_THETA: u32 = 768;
    const MAX_PHI: u32 = 384;
    const VERT_BUDGET: f32 = 800_000.0;
    let spc = |power: f32| SPC * power.sqrt();

    let mut theta_need = 0.0f32; // features periodic along the coil
    let mut phi_need = 0.0f32; // features periodic around the aperture
    if p.rib_ax_amp.abs() > 1e-6 {
        theta_need = theta_need.max(p.rib_ax_count * spc(rib_power));
    }
    if p.rib_sp_amp.abs() > 1e-6 {
        phi_need = phi_need.max(p.rib_sp_count * spc(rib_power));
    }
    if p.varix_amp.abs() > 1e-6 {
        theta_need = theta_need.max(p.varix_count * spc(VARIX_POWER));
    }
    if proj_active {
        theta_need = theta_need.max(p.proj_count * spc(proj_power));
        phi_need = phi_need.max(p.proj_rows.max(1.0) * spc(proj_power));
    }

    let base_theta = p.seg_theta.max(3);
    let base_phi = p.seg_phi.max(3);
    let mut seg_theta = (base_theta as f32).max(theta_need).ceil().min(MAX_THETA as f32) as u32;
    let mut seg_phi = (base_phi as f32).max(phi_need).ceil().min(MAX_PHI as f32) as u32;
    // Scale both down together if the total vertex count would blow the budget.
    let est = seg_theta as f32 * n * seg_phi as f32;
    if est > VERT_BUDGET {
        let s = (VERT_BUDGET / est).sqrt();
        seg_theta = ((seg_theta as f32 * s) as u32).max(base_theta);
        seg_phi = ((seg_phi as f32 * s) as u32).max(base_phi);
    }
    let cols = seg_phi as usize;

    let total_theta = n * 2.0 * PI;
    let k = w.ln() / (2.0 * PI); // growth rate: g = exp(k·theta) grows by W each turn

    // Total segments along the coil (segments-per-revolution × revolutions).
    let theta_steps = ((seg_theta as f32) * n).ceil().max(1.0) as usize;
    let theta_verts = theta_steps + 1;

    let mut positions = Vec::with_capacity(theta_verts * cols * 3);
    let mut normals = vec![0.0f32; theta_verts * cols * 3];
    let mut uvs = Vec::with_capacity(theta_verts * cols * 2);

    let two_pi = 2.0 * PI;
    let sharp = p.rib_sharp;

    // --- seeded randomness setup (jitter = 0 → exact uniform output) ---
    let jittered = p.jitter > 1e-6;
    let jit = p.jitter.clamp(0.0, 1.0);
    let seed = p.seed.max(0.0) as u32;
    // Distinct salts so the different effects are decorrelated.
    const S_TWARP: u32 = 0x9E37_79B1;
    const S_TWARP2: u32 = 0x85EB_CA77;
    const S_PHIWARP: u32 = 0xC2B2_AE3D;
    const S_AXAMP: u32 = 0x27D4_EB2F;
    const S_VXAMP: u32 = 0x1656_67B1;
    const S_PRAMP: u32 = 0xD3A2_646C;
    const S_SPAMP: u32 = 0xFD70_46C5;
    const S_COIL: u32 = 0xB55A_4F09;

    for i in 0..theta_verts {
        let theta = total_theta * (i as f32) / (theta_steps as f32);
        let g = (k * theta).exp();
        let ap_r = aspect * g; // aperture radial semi-axis
        let ap_z = g; // aperture axial semi-axis
        let ct = theta.cos();
        let st = theta.sin();

        // Per-θ seeded jitter: a slow domain-warp on θ (so features drift across
        // whorls instead of stacking at the same angle), per-instance amplitude
        // wobble, a φ-shift (cords meander), and a subtle coil radius wobble.
        let mut theta_w = theta;
        let mut radius_wob = 1.0;
        let mut ax_ampj = 1.0;
        let mut varix_ampj = 1.0;
        let mut proj_ampj = 1.0;
        let mut phi_shift = 0.0;
        if jittered {
            let whorl = theta / two_pi;
            theta_w = theta
                + jit
                    * 0.13
                    * (noise1(seed ^ S_TWARP, whorl * 0.8) * 0.7
                        + noise1(seed ^ S_TWARP2, whorl * 2.3) * 0.3);
            radius_wob = 1.0 + jit * 0.025 * noise1(seed ^ S_COIL, whorl * 1.1 + 11.0);
            ax_ampj = 1.0 + jit * 0.35 * rand_signed(seed ^ S_AXAMP, (p.rib_ax_count * whorl).round() as i32);
            varix_ampj = 1.0 + jit * 0.5 * rand_signed(seed ^ S_VXAMP, (p.varix_count * whorl).round() as i32);
            proj_ampj = 1.0 + jit * 0.45 * rand_signed(seed ^ S_PRAMP, (p.proj_count * whorl).round() as i32);
            phi_shift = jit * 0.08 * noise1(seed ^ S_PHIWARP, theta * 0.4);
        }

        let radius = (ap_r / (1.0 - d)) * radius_wob; // axis → centre, with coil wobble
        let cz = p.t * radius; // centre height: ∝ radius gives the conical spire

        // θ-only ornament terms (warped angle → features don't lock per whorl).
        let axial = p.rib_ax_amp * ax_ampj * ribbed(p.rib_ax_count * theta_w, sharp);
        let varix = p.varix_amp * varix_ampj * lobe(p.varix_count * theta_w, VARIX_POWER);
        let proj_theta = lobe(p.proj_count * theta_w, proj_power);

        let u = i as f32 / theta_steps as f32; // 0..1 along the coil

        for col in 0..cols {
            let phi = two_pi * (col as f32) / (cols as f32);
            let phi_o = phi + phi_shift; // warped φ for ornament placement
            // Spiral cords: continuous along the coil → longitudinal cords.
            let cord_ampj = if jittered {
                1.0 + jit * 0.3 * rand_signed(seed ^ S_SPAMP, (p.rib_sp_count * phi / two_pi).round() as i32)
            } else {
                1.0
            };
            let spiral = p.rib_sp_amp * cord_ampj * ribbed(p.rib_sp_count * phi_o, sharp);
            // Projections: blunt beads (rows≥2, low sharp) → needle spines
            // (rows=1, high sharp), localised on a θ×φ grid offset by proj_pos.
            let proj = if proj_active {
                p.proj_size * proj_ampj * proj_theta * lobe(p.proj_rows * (phi_o - p.proj_pos), proj_power)
            } else {
                0.0
            };

            // Radial displacement along the aperture's outward direction, scaled
            // by g so ornament stays proportional along the whole shell.
            let disp = g * (axial + spiral + varix + proj);
            let rr = radius + (ap_r + disp) * phi.cos();
            positions.push(rr * ct); // x
            positions.push(rr * st); // y
            positions.push(cz + (ap_z + disp) * phi.sin()); // z
            uvs.push(u);
            uvs.push(col as f32 / cols as f32); // v around the aperture
        }
    }

    // Build quads between adjacent rings; phi wraps (closed tube), so j+1 is mod cols.
    let mut indices = Vec::with_capacity(theta_steps * cols * 6);
    let vid = |i: usize, j: usize| -> u32 { (i * cols + (j % cols)) as u32 };
    for i in 0..theta_steps {
        for j in 0..cols {
            let a = vid(i, j);
            let b = vid(i + 1, j);
            let c = vid(i + 1, j + 1);
            let e = vid(i, j + 1);
            indices.extend_from_slice(&[a, b, c, a, c, e]);
        }
    }

    // Smooth normals: accumulate face normals onto vertices, then normalise.
    for tri in indices.chunks_exact(3) {
        let (ia, ib, ic) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let pa = [positions[ia * 3], positions[ia * 3 + 1], positions[ia * 3 + 2]];
        let pb = [positions[ib * 3], positions[ib * 3 + 1], positions[ib * 3 + 2]];
        let pc = [positions[ic * 3], positions[ic * 3 + 1], positions[ic * 3 + 2]];
        let u = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let v = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let nrm = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        for &idx in &[ia, ib, ic] {
            normals[idx * 3] += nrm[0];
            normals[idx * 3 + 1] += nrm[1];
            normals[idx * 3 + 2] += nrm[2];
        }
    }
    for nrm in normals.chunks_exact_mut(3) {
        let len = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
        if len > 1e-8 {
            nrm[0] /= len;
            nrm[1] /= len;
            nrm[2] /= len;
        }
    }

    // Normalise to a unit sphere centred at the origin. Raw coords span a huge
    // range (g = e^(kθ) can reach thousands), which wrecks downstream float
    // precision (BVH / ray tracing). Translate + uniform scale leave the
    // already-unit normals unchanged.
    let vcount = (positions.len() / 3).max(1) as f32;
    let (mut cx, mut cy, mut cz) = (0.0f32, 0.0f32, 0.0f32);
    for v in positions.chunks_exact(3) {
        cx += v[0];
        cy += v[1];
        cz += v[2];
    }
    cx /= vcount;
    cy /= vcount;
    cz /= vcount;
    let mut max_r2 = 0.0f32;
    for v in positions.chunks_exact(3) {
        let (dx, dy, dz) = (v[0] - cx, v[1] - cy, v[2] - cz);
        max_r2 = max_r2.max(dx * dx + dy * dy + dz * dz);
    }
    let scale = 1.0 / max_r2.sqrt().max(1e-6);
    for v in positions.chunks_exact_mut(3) {
        v[0] = (v[0] - cx) * scale;
        v[1] = (v[1] - cy) * scale;
        v[2] = (v[2] - cz) * scale;
    }

    // --- Orient for display ---------------------------------------------------
    // The coil is built around +Z with the apex (smallest whorl) at the low-z end
    // and the body whorl / aperture at the high-z end. For viewing we want a
    // canonical pose: the coil axis vertical with the cone pointing *down*, and
    // the body whorl turned to face the camera (+Z). Two rotations about the now-
    // centred origin do it — rotations preserve the unit normalisation above and
    // keep the (already unit) normals valid.
    //
    // 1) Spin about the coil axis (Z) so the body whorl's azimuth points to
    //    (0,-1); 2) tip upright with Rx(-90°): (x,y,z) -> (x, z, -y), which sends
    //    +Z (coil axis) -> +Y (up), the low-z apex -> -Y (bottom), and the body
    //    whorl -> +Z (front).
    let last = (theta_verts - 1) * cols;
    let (mut ax, mut ay) = (0.0f32, 0.0f32);
    for c in 0..cols {
        let b = (last + c) * 3;
        ax += positions[b];
        ay += positions[b + 1];
    }
    ax /= cols as f32;
    ay /= cols as f32;
    let rho = -0.5 * PI - ay.atan2(ax);
    let (sr, cr) = rho.sin_cos();
    let orient = |p: &mut [f32]| {
        let (x, y, z) = (p[0], p[1], p[2]);
        p[0] = x * cr - y * sr; // x' = Rz(rho)·x
        p[1] = z; //               y' = z   (coil axis -> up)
        p[2] = -(x * sr + y * cr); // z' = -y'
    };
    for v in positions.chunks_exact_mut(3) {
        orient(v);
    }
    for nrm in normals.chunks_exact_mut(3) {
        orient(nrm);
    }

    Mesh {
        positions,
        normals,
        uvs,
        indices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_table_covers_every_field() {
        // 19 user-facing shape params, unique keys, defaults inside their range.
        assert_eq!(PARAM_RANGES.len(), 19);
        let mut keys: Vec<_> = PARAM_RANGES.iter().map(|r| r.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 19, "duplicate or missing PARAM_RANGES key");
        for r in PARAM_RANGES {
            assert!(r.min <= r.max, "{}: min > max", r.key);
            assert!(r.step > 0.0, "{}: non-positive step", r.key);
            assert!(
                r.default >= r.min && r.default <= r.max,
                "{}: default out of range",
                r.key
            );
            if r.integer {
                assert_eq!(r.default.fract(), 0.0, "{}: integer default not whole", r.key);
            }
        }
        // The `Default` impl must agree with the table's defaults.
        let d = ShellParams::default();
        let by = |k| PARAM_RANGES.iter().find(|r| r.key == k).unwrap().default;
        assert_eq!(d.w, by("w"));
        assert_eq!(d.d, by("d"));
        assert_eq!(d.t, by("t"));
        assert_eq!(d.n, by("n"));
        assert_eq!(d.aspect, by("aspect"));
    }

    #[test]
    fn clamp_pins_out_of_range_and_rounds_integers() {
        let p = ShellParams {
            w: 100.0,        // above max 8.0
            d: 5.0,          // above max 0.95
            t: -3.0,         // below min 0.0
            n: 999.0,        // above max 20.0
            aspect: 0.01,    // below min 0.3
            proj_count: -7.0, // below min, integer
            varix_count: 4.7, // integer rounding
            jitter: 2.0,     // above max 1.0
            ..ShellParams::default()
        }
        .clamped();
        assert_eq!(p.w, 8.0);
        assert_eq!(p.d, 0.95);
        assert_eq!(p.t, 0.0);
        assert_eq!(p.n, 20.0);
        assert_eq!(p.aspect, 0.3);
        assert_eq!(p.proj_count, 0.0);
        assert_eq!(p.varix_count, 5.0); // 4.7 rounds to nearest int
        assert_eq!(p.jitter, 1.0);
        // every field lands within its declared range
        for r in PARAM_RANGES {
            let v = match r.key {
                "w" => p.w, "d" => p.d, "t" => p.t, "n" => p.n, "aspect" => p.aspect,
                "rib_ax_count" => p.rib_ax_count, "rib_ax_amp" => p.rib_ax_amp,
                "rib_sp_count" => p.rib_sp_count, "rib_sp_amp" => p.rib_sp_amp,
                "rib_sharp" => p.rib_sharp, "proj_count" => p.proj_count,
                "proj_rows" => p.proj_rows, "proj_pos" => p.proj_pos,
                "proj_size" => p.proj_size, "proj_sharp" => p.proj_sharp,
                "varix_count" => p.varix_count, "varix_amp" => p.varix_amp,
                "seed" => p.seed, "jitter" => p.jitter,
                other => panic!("untested key {other}"),
            };
            assert!(v >= r.min && v <= r.max, "{} out of range after clamp: {v}", r.key);
        }
    }

    #[test]
    fn generate_is_safe_for_wild_input() {
        // Garbage in → a finite, well-formed mesh out (clamp guards the math).
        let wild = ShellParams {
            w: 1e6,
            d: 12.0,
            n: 5000.0,
            proj_count: 1e4,
            rib_sp_count: 1e4,
            varix_count: -50.0,
            ..ShellParams::default()
        };
        let m = generate(&wild);
        assert!(!m.positions.is_empty());
        assert!(m.positions.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn mesh_is_wellformed() {
        let m = generate(&ShellParams::default());
        assert!(!m.positions.is_empty());
        assert_eq!(m.positions.len() % 3, 0);
        assert_eq!(m.normals.len(), m.positions.len());
        assert_eq!(m.uvs.len(), m.positions.len() / 3 * 2);
        assert_eq!(m.indices.len() % 3, 0);
        assert!(m.positions.iter().all(|x| x.is_finite()));
        let vcount = (m.positions.len() / 3) as u32;
        assert!(m.indices.iter().all(|&i| i < vcount), "index out of range");
    }

    #[test]
    fn normals_are_unit_length() {
        let m = generate(&ShellParams::default());
        for nrm in m.normals.chunks_exact(3) {
            let len = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-3, "normal not unit: {len}");
        }
    }

    #[test]
    fn ribs_perturb_the_surface_but_stay_finite() {
        // Pin resolution at the caps on both so auto-refinement clamps equally,
        // keeping topology identical for an element-wise comparison.
        let base = ShellParams {
            seg_phi: 384,
            seg_theta: 768,
            ..ShellParams::default()
        };
        let smooth = generate(&base);
        let ribbed = generate(&ShellParams {
            rib_ax_count: 14.0,
            rib_ax_amp: 0.25,
            rib_sp_count: 10.0,
            rib_sp_amp: 0.15,
            rib_sharp: 0.5,
            ..base.clone()
        });
        assert_eq!(smooth.positions.len(), ribbed.positions.len());
        let moved = smooth
            .positions
            .iter()
            .zip(&ribbed.positions)
            .any(|(a, b)| (a - b).abs() > 1e-4);
        assert!(moved, "ornamentation should change vertex positions");
        assert!(ribbed.positions.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn projections_and_varices_each_perturb_and_stay_finite() {
        let base = ShellParams {
            seg_phi: 384,
            seg_theta: 768,
            ..ShellParams::default()
        };
        let smooth = generate(&base);
        let variants = [
            // single-row needle spine
            ShellParams { proj_count: 8.0, proj_rows: 1.0, proj_pos: 1.1, proj_size: 0.8, proj_sharp: 0.75, ..base.clone() },
            // multi-row blunt nodules
            ShellParams { proj_count: 12.0, proj_rows: 2.0, proj_size: 0.12, proj_sharp: 0.15, ..base.clone() },
            ShellParams { varix_count: 3.0, varix_amp: 0.3, ..base.clone() },
        ];
        for (k, v) in variants.iter().enumerate() {
            let m = generate(v);
            assert_eq!(m.positions.len(), smooth.positions.len(), "variant {k} topology");
            let moved = smooth
                .positions
                .iter()
                .zip(&m.positions)
                .any(|(a, b)| (a - b).abs() > 1e-4);
            assert!(moved, "variant {k} should change the surface");
            assert!(m.positions.iter().all(|x| x.is_finite()), "variant {k} finite");
        }
    }

    #[test]
    fn jitter_zero_is_identical_regardless_of_seed() {
        let a = generate(&ShellParams::default());
        let b = generate(&ShellParams { seed: 9999.0, jitter: 0.0, ..ShellParams::default() });
        assert_eq!(a.positions, b.positions, "jitter=0 must ignore the seed exactly");
    }

    #[test]
    fn jitter_is_deterministic_and_seed_dependent() {
        let p1 = ShellParams {
            varix_count: 3.0,
            varix_amp: 0.3,
            jitter: 0.6,
            seed: 1.0,
            ..ShellParams::default()
        };
        let a = generate(&p1);
        let a2 = generate(&p1);
        let b = generate(&ShellParams { seed: 2.0, ..p1.clone() });
        let uniform = generate(&ShellParams { jitter: 0.0, ..p1.clone() });

        assert_eq!(a.positions, a2.positions, "same seed+params must reproduce exactly");
        assert_eq!(a.positions.len(), b.positions.len());
        assert!(
            a.positions.iter().zip(&b.positions).any(|(x, y)| (x - y).abs() > 1e-5),
            "different seeds should produce different shapes"
        );
        assert!(
            a.positions.iter().zip(&uniform.positions).any(|(x, y)| (x - y).abs() > 1e-5),
            "jitter should perturb the surface vs the uniform shape"
        );
    }

    #[test]
    fn sharper_projections_get_denser_tessellation() {
        // Same count/size; only sharpness differs. The needle is narrower, so it
        // must auto-refine to a denser mesh than the blunt bead.
        let common = ShellParams {
            proj_count: 8.0,
            proj_rows: 1.0,
            proj_size: 0.4,
            ..ShellParams::default()
        };
        let blunt = generate(&ShellParams { proj_sharp: 0.1, ..common.clone() });
        let needle = generate(&ShellParams { proj_sharp: 1.0, ..common.clone() });
        assert!(
            needle.positions.len() > blunt.positions.len(),
            "a sharp needle should refine denser than a blunt bead ({} vs {})",
            needle.positions.len(),
            blunt.positions.len()
        );
    }

    #[test]
    fn tessellation_refines_for_high_frequency_cords() {
        let plain = generate(&ShellParams { seg_phi: 48, ..ShellParams::default() });
        let cords = generate(&ShellParams {
            seg_phi: 48,
            rib_sp_count: 16.0,
            rib_sp_amp: 0.1,
            ..ShellParams::default()
        });
        // Same θ resolution, but cords force a finer φ tessellation (16·12 > 48).
        assert!(
            cords.positions.len() > plain.positions.len(),
            "high-frequency cords should auto-refine the cross-section mesh"
        );
    }

    #[test]
    fn smooth_wave_is_symmetric_ridge_is_peaked() {
        // sharp=0 → mean ~0 (symmetric swell in/out); sharp=1 → mean <0 (narrow ridges).
        let n = 2000;
        let mean = |sharp: f32| {
            (0..n)
                .map(|i| ribbed(i as f32 / n as f32 * 2.0 * PI, sharp))
                .sum::<f32>()
                / n as f32
        };
        assert!(mean(0.0).abs() < 0.02, "smooth wave should be ~symmetric");
        assert!(mean(1.0) < -0.3, "sharp ridges should sit below baseline");
    }

    #[test]
    fn planispiral_centres_lie_in_a_plane() {
        // T = 0 → centre height is 0; the coil is flat. After the display
        // orientation the coil axis is the world Y, so a planispiral stays flat in
        // Y: max |y| is bounded by the largest aperture half-height (the only
        // out-of-plane extent), times the unit-normalisation scale (≤ 1).
        let p = ShellParams {
            t: 0.0,
            ..ShellParams::default()
        };
        let m = generate(&p);
        let g_max = p.w.powf(p.n);
        let max_y = m.positions.iter().skip(1).step_by(3).fold(0.0f32, |a, &y| a.max(y.abs()));
        assert!(max_y <= g_max * 1.01, "planispiral too tall: {max_y} vs {g_max}");
    }
}
