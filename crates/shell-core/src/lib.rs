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
            seg_theta: 96,
            seg_phi: 48,
        }
    }
}

/// A triangle mesh as flat buffers, ready to hand to a GPU / Three.js.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mesh {
    /// xyz position triples.
    pub positions: Vec<f32>,
    /// xyz normal triples (per vertex, parallel to `positions`).
    pub normals: Vec<f32>,
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

/// Generate a shell surface by sweeping an elliptical aperture along a
/// logarithmic helico-spiral.
///
/// `theta` runs along the coil (0 .. 2π·n); `phi` runs around the aperture.
/// The aperture and its distance from the axis both scale by `g = W^(theta/2π)`,
/// which keeps the form self-similar (why shells are logarithmic spirals).
pub fn generate(p: &ShellParams) -> Mesh {
    let n = p.n.max(0.01);
    let d = p.d.clamp(0.0, 0.95);
    let aspect = p.aspect.max(0.05);
    let w = p.w.max(1.0001);

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

    let two_pi = 2.0 * PI;
    let sharp = p.rib_sharp;

    for i in 0..theta_verts {
        let theta = total_theta * (i as f32) / (theta_steps as f32);
        let g = (k * theta).exp();
        let ap_r = aspect * g; // aperture radial semi-axis
        let ap_z = g; // aperture axial semi-axis
        let radius = ap_r / (1.0 - d); // axis → aperture centre (D controls openness)
        let ct = theta.cos();
        let st = theta.sin();
        let cz = p.t * radius; // centre height: ∝ radius gives the conical spire

        // θ-only ornament terms (constant around the aperture).
        // Axial ribs/waves (a non-integer count makes them drift across whorls).
        let axial = p.rib_ax_amp * ribbed(p.rib_ax_count * theta, sharp);
        // Varices: a few prominent raised transverse ridges per whorl.
        let varix = p.varix_amp * lobe(p.varix_count * theta, VARIX_POWER);
        // θ-window for the localised projections.
        let proj_theta = lobe(p.proj_count * theta, proj_power);

        for j in 0..cols {
            let phi = two_pi * (j as f32) / (cols as f32);
            // Spiral cords: continuous along the coil → longitudinal cords.
            let spiral = p.rib_sp_amp * ribbed(p.rib_sp_count * phi, sharp);
            // Projections: blunt beads (rows≥2, low sharp) → needle spines
            // (rows=1, high sharp), localised on a θ×φ grid offset by proj_pos.
            let proj = if proj_active {
                p.proj_size * proj_theta * lobe(p.proj_rows * (phi - p.proj_pos), proj_power)
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

    Mesh {
        positions,
        normals,
        indices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_is_wellformed() {
        let m = generate(&ShellParams::default());
        assert!(!m.positions.is_empty());
        assert_eq!(m.positions.len() % 3, 0);
        assert_eq!(m.normals.len(), m.positions.len());
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
        // T = 0 → centre height is 0; surface z only from the aperture (±g).
        let p = ShellParams {
            t: 0.0,
            ..ShellParams::default()
        };
        let m = generate(&p);
        // Max |z| should stay bounded by the largest aperture half-height.
        let g_max = p.w.powf(p.n);
        let max_z = m.positions.iter().skip(2).step_by(3).fold(0.0f32, |a, &z| a.max(z.abs()));
        assert!(max_z <= g_max * 1.01, "planispiral too tall: {max_z} vs {g_max}");
    }
}
