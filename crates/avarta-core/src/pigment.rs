// ===========================================================================
// Layer 3 — pigmentation (reaction–diffusion at the growing lip)
// ===========================================================================
//
// Biology (Meinhardt, *The Algorithmic Beauty of Sea Shells*): shell pigment is
// secreted by the mantle edge at the growing aperture. Pigment cells sit along
// that edge (a closed loop = φ) and switch on/off based on their neighbours —
// a 1-D reaction–diffusion line. As the shell grows (θ advances) the line's
// state is frozen into the shell, so the 2-D surface pattern is the *space–time
// record* of the line: φ across, growth-time θ along. That is exactly the mesh
// sweep, so pigment and geometry share one growth process and the mesh's
// (u=θ, v=φ) UVs map the pattern with no distortion.
//
// The core is a Gray–Scott reaction–diffusion line (robust across regimes; its
// (F,K) sit in Pearson's pattern space). Different `PigRegime`s steer it: a
// drift term advects the pattern around the lip (oblique / chevron leans), and
// a commarginal growth-rhythm oscillation (a temporal band, biologically the
// periodic deposition along a growth line) supplies axial stripes and combines
// with the RD stripes for spots (×) and reticulation (max). Coefficients are
// empirically tuned approximations — tweak against the species harness.

use crate::noise::{lerp, noise1, rand01, smoothstep};
use crate::params::ShellParams;
use std::f32::consts::PI;

/// Pigment texture resolution around the lip (φ), independent of the mesh's
/// `cols`. High enough that hard-edged patterns don't pixellate when the texture
/// is magnified on a large body whorl (mip/anisotropic filtering smooths the
/// rest); the reaction–diffusion line runs coarser and is resampled up to this.
const PIG_PHI: usize = 512;
/// Growth-time (θ) samples per whorl, bounded so long augers stay affordable.
const PIG_THETA_PER_WHORL: f32 = 256.0;
const PIG_THETA_MIN: usize = 96;
const PIG_THETA_MAX: usize = 4096;
/// RD steps to settle the line before recording the apex column.
const PIG_BURN_IN: usize = 400;
/// Decorrelates the pigment RNG from the geometry jitter salts.
const S_PIGMENT: u32 = 0x5F1C_3A2B;
/// Salt for the axial-stripe phase wobble (irregularity along the coil).
const S_AXIAL_PHASE: u32 = 0x1111_2222;
/// Salt gating whether a mid-sweep random re-nucleation fires this column.
const S_RESEED_GATE: u32 = 0x0000_ABCD;
/// Salt picking which lip cell that re-nucleation seeds.
const S_RESEED_POS: u32 = 0x0000_1234;

/// Pigment pattern families (the `pig_regime` index). Each selects a Gray–Scott
/// configuration / combination of the shared RD line; the continuous `pig_*`
/// knobs modulate within a family.
#[derive(Clone, Copy, PartialEq)]
enum PigRegime {
    Solid,
    SpiralBands,
    AxialStripes,
    ObliqueLines,
    Chevrons,
    Spots,
    Reticulated,
}

impl PigRegime {
    fn from_index(i: f32) -> Self {
        match i.round() as i32 {
            1 => Self::SpiralBands,
            2 => Self::AxialStripes,
            3 => Self::ObliqueLines,
            4 => Self::Chevrons,
            5 => Self::Spots,
            6 => Self::Reticulated,
            _ => Self::Solid,
        }
    }
}

/// A periodic 1-D Gray–Scott reaction–diffusion line with double-buffered state.
/// `u` ≈ substrate (1 = full), `v` ≈ activator → pigment. The diffusion/feed/kill
/// coefficients are fixed for the line's lifetime; `step` advances it in place and
/// the caller reads [`GrayScott::activator`] after each step to record the pattern.
///
/// This replaces a free `gs_step(u, v, un, vn, du, dv, f, k, drift)` whose nine
/// arguments tripped `clippy::too_many_arguments`: bundling the four buffers and
/// four coefficients into the type leaves only the per-step `drift`, and hides the
/// next-state swap that every caller had to remember to do.
struct GrayScott {
    // Diffusion rates and feed/kill coefficients — fixed for the whole sweep.
    du: f32,
    dv: f32,
    f: f32,
    k: f32,
    // Current state, plus the scratch buffers swapped in after each step.
    u: Vec<f32>,
    v: Vec<f32>,
    u_next: Vec<f32>,
    v_next: Vec<f32>,
}

impl GrayScott {
    /// A line of `len` cells in the resting state: substrate full (`u` = 1), no
    /// activator (`v` = 0). Nucleate cells with [`GrayScott::nucleate`] to seed it.
    fn new(len: usize, du: f32, dv: f32, f: f32, k: f32) -> Self {
        Self {
            du,
            dv,
            f,
            k,
            u: vec![1.0; len],
            v: vec![0.0; len],
            u_next: vec![0.0; len],
            v_next: vec![0.0; len],
        }
    }

    /// The activator field (→ pigment) on the current state.
    fn activator(&self) -> &[f32] {
        &self.v
    }

    /// Seed cell `j` to nucleate a stripe.
    fn nucleate(&mut self, j: usize) {
        self.v[j] = 0.5;
        self.u[j] = 0.25;
    }

    /// One explicit-Euler step on the periodic line, then swap in the new state.
    /// `drift` advects `v` around the lip (upwind) so the pattern can travel.
    fn step(&mut self, drift: f32) {
        let n = self.u.len();
        for j in 0..n {
            let jm = if j == 0 { n - 1 } else { j - 1 };
            let jp = if j == n - 1 { 0 } else { j + 1 };
            let lap_u = self.u[jm] + self.u[jp] - 2.0 * self.u[j];
            let lap_v = self.v[jm] + self.v[jp] - 2.0 * self.v[j];
            let uvv = self.u[j] * self.v[j] * self.v[j];
            let du_dt = self.du * lap_u - uvv + self.f * (1.0 - self.u[j]);
            let dv_dt = self.dv * lap_v + uvv
                - (self.f + self.k) * self.v[j]
                - drift * (self.v[j] - self.v[jm]);
            // dt = 1 (the chosen diffusion/feed coefficients keep this stable).
            self.u_next[j] = (self.u[j] + du_dt).clamp(0.0, 1.5);
            self.v_next[j] = (self.v[j] + dv_dt).clamp(0.0, 1.5);
        }
        std::mem::swap(&mut self.u, &mut self.u_next);
        std::mem::swap(&mut self.v, &mut self.v_next);
    }
}

/// Generate the Layer-3 pigment field for `p`. Returns `(field, width=θ,
/// height=φ)`, row-major over φ. Deterministic from `p.seed`. The geometry is
/// untouched — this only reads the params that describe the pattern.
pub(crate) fn pigment_field(p: &ShellParams) -> (Vec<u8>, u32, u32) {
    let regime = PigRegime::from_index(p.pig_regime);
    let nx = ((p.n * PIG_THETA_PER_WHORL).round() as usize).clamp(PIG_THETA_MIN, PIG_THETA_MAX);
    let ny = PIG_PHI;
    let mut out = vec![0u8; nx * ny];

    // Solid → a uniform field; the base colour fills the whole shell.
    if regime == PigRegime::Solid {
        return (out, nx as u32, ny as u32);
    }

    let scale = p.pig_scale.clamp(0.0, 1.0);
    let contrast = p.pig_contrast.clamp(0.0, 1.0);
    let density = p.pig_density.clamp(0.0, 1.0);
    let angle = p.pig_angle.clamp(0.0, 1.0);
    let irreg = p.pig_irregularity.clamp(0.0, 1.0);
    let seed = (p.seed.max(0.0) as u32) ^ S_PIGMENT;

    // Contrast → smoothstep window around the mid-tone: crisp edge (high) vs.
    // soft gradient (low, which the viewer palette renders as accent mid-tones).
    // The floor keeps even the crispest edge a couple of texels wide so it
    // anti-aliases under texture filtering instead of stair-stepping.
    let half = lerp(0.35, 0.05, contrast);
    let edge = |x: f32| smoothstep(0.5 - half, 0.5 + half, x);

    // Commarginal growth-rhythm oscillation: a band in growth-time (θ), uniform
    // around the lip. Used directly for axial stripes and as the transverse
    // factor for spots / reticulation.
    let bands_per_whorl = lerp(1.5, 10.0, density);
    let axial_at = |i: usize| -> f32 {
        let whorls = (i as f32 / nx as f32) * p.n;
        let mut ph = whorls * bands_per_whorl * (2.0 * PI);
        if irreg > 0.0 {
            ph += irreg * 1.5 * noise1(seed ^ S_AXIAL_PHASE, whorls * 3.0);
        }
        0.5 + 0.5 * ph.cos()
    };

    if regime == PigRegime::AxialStripes {
        for i in 0..nx {
            let b = (edge(axial_at(i)) * 255.0).round() as u8;
            for j in 0..ny {
                out[j * nx + i] = b;
            }
        }
        return (out, nx as u32, ny as u32);
    }

    // --- Gray–Scott line advanced over growth-time --------------------------
    // Feature scale sets the simulation's φ resolution: the GS stripe wavelength
    // is fixed (so the pattern is always reliable), so fewer sim cells spread
    // the same wavelength over fewer features around the lip (coarse), more
    // cells over more features (fine). The ny_sim line is resampled to the
    // PIG_PHI output rows, so the texture stays smooth either way.
    let (f, k) = (0.035, 0.060); // Pearson stripe/maze regime → periodic peaks
    let du = 0.16;
    let dv = 0.08;
    // Snap to a multiple of the seed spacing so nucleation wraps evenly around
    // the periodic lip (no seam where the last gap differs from the rest).
    const SEED_SPACING: usize = 16;
    let ny_sim = (((lerp(384.0, 48.0, scale) / SEED_SPACING as f32).round() as usize)
        * SEED_SPACING)
        .max(2 * SEED_SPACING); // large→fine, small→coarse
    let substeps = lerp(2.0, 12.0, density).round().max(1.0) as usize;
    // Obliqueness advects the pattern around the lip as the shell grows → a
    // diagonal lean, on *every* stripe regime (so the control always does
    // something). Oblique/chevron carry a baseline lean — that is their
    // character — and angle adds to it. Chevrons additionally mirror the drifted
    // field across the lip midline so the two leans meet in V-tents (below).
    // Keep the per-step drift gentle — it accumulates over the long coil into a
    // strong lean, and too large a per-step advection breaks the stripes into
    // dashes instead of leaning them.
    let drift_base = match regime {
        PigRegime::ObliqueLines => 0.08,
        PigRegime::Chevrons => 0.12,
        _ => 0.0,
    };
    let drift = drift_base + 0.06 * angle;

    let mut gs = GrayScott::new(ny_sim, du, dv, f, k);
    // Nucleate at the constant ~intrinsic stripe spacing (≈ the GS wavelength at
    // this du/dv), so the stripes are stable (neither merge nor split) and the
    // feature count is ny_sim / spacing — i.e. driven by `scale` via ny_sim.
    // irregularity sprinkles extra seeds for natural variation.
    for j in (0..ny_sim).step_by(SEED_SPACING) {
        gs.nucleate(j);
    }
    if irreg > 0.0 {
        for j in 0..ny_sim {
            if rand01(seed, j as i32) < irreg * 0.15 {
                gs.nucleate(j);
            }
        }
    }

    for _ in 0..PIG_BURN_IN {
        gs.step(0.0);
    }

    // Raw activator field on the sim grid; normalised after the sweep so the
    // contrast mapping is robust regardless of the regime's concentration range.
    let mut raw = vec![0.0f32; nx * ny_sim];
    for i in 0..nx {
        for _ in 0..substeps {
            gs.step(drift);
        }
        if irreg > 0.0 && rand01(seed ^ S_RESEED_GATE, i as i32) < irreg * 0.05 {
            let j = (rand01(seed ^ S_RESEED_POS, i as i32) * ny_sim as f32) as usize % ny_sim;
            gs.nucleate(j);
        }
        let v = gs.activator();
        for j in 0..ny_sim {
            raw[j * nx + i] = v[j];
        }
    }

    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for &x in &raw {
        lo = lo.min(x);
        hi = hi.max(x);
    }
    let span = (hi - lo).max(1e-6);
    // Bilinearly sample the normalised sim line at a fractional φ position
    // (wrapping the periodic lip). Interpolating *before* the edge threshold is
    // what removes the φ stair-stepping when ny_sim < ny (the output rows).
    let sample_phi = |i: usize, fy: f32| -> f32 {
        let f = fy.rem_euclid(ny_sim as f32);
        let a = f.floor() as usize % ny_sim;
        let b = (a + 1) % ny_sim;
        let fr = f - f.floor();
        ((raw[a * nx + i] - lo) * (1.0 - fr) + (raw[b * nx + i] - lo) * fr) / span
    };
    // Resample the ny_sim line onto the PIG_PHI (`ny`) output rows.
    for i in 0..nx {
        let ax = axial_at(i);
        for jo in 0..ny {
            let fy = jo as f32 * ny_sim as f32 / ny as f32;
            let gs = sample_phi(i, fy);
            // Spots = RD stripes ∧ transverse rhythm (dots at intersections);
            // reticulated = RD stripes ∨ rhythm (net); chevrons = the drifted
            // diagonal ∨ its φ-mirror (the two leans meet in V-tents); others use
            // the RD field directly.
            let combined = match regime {
                PigRegime::Spots => gs * ax,
                PigRegime::Reticulated => gs.max(ax),
                PigRegime::Chevrons => {
                    let mirror = sample_phi(i, (ny_sim as f32 - 1.0) - fy);
                    gs.max(mirror)
                }
                _ => gs,
            };
            out[jo * nx + i] = (edge(combined) * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    (out, nx as u32, ny as u32)
}
