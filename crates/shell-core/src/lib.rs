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
    /// Phase drift per whorl: 0 = ribs aligned up the spire, ~π = staggered.
    #[serde(default)]
    pub rib_phase: f32,

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
            rib_phase: 0.0,
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

    // Auto-refine tessellation to the ornament frequency: each rib/cord needs
    // enough samples to read as a smooth wave, otherwise high-frequency ornament
    // aliases into faceted "sharp" lines. Plain shells keep the cheap base
    // resolution; only ornate ones pay for refinement. Capped to bound mesh size.
    const MIN_SAMPLES_PER_FEATURE: f32 = 12.0;
    const MAX_SEG: u32 = 256;
    let mut seg_phi = p.seg_phi.max(3);
    if p.rib_sp_amp.abs() > 1e-6 && p.rib_sp_count > 0.5 {
        let need = (p.rib_sp_count * MIN_SAMPLES_PER_FEATURE).ceil() as u32;
        seg_phi = seg_phi.max(need).min(MAX_SEG);
    }
    let mut seg_theta = p.seg_theta.max(3);
    if p.rib_ax_amp.abs() > 1e-6 && p.rib_ax_count > 0.5 {
        let need = (p.rib_ax_count * MIN_SAMPLES_PER_FEATURE).ceil() as u32;
        seg_theta = seg_theta.max(need).min(MAX_SEG);
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

        // Axial ribs/waves: periodic along the coil; rib_phase drifts them per
        // whorl (0 = aligned up the spire, ~π = staggered). Same for all phi, so
        // the whole cross-section pulses → transverse ribs / corrugation.
        let axial = p.rib_ax_amp
            * ribbed(p.rib_ax_count * theta + p.rib_phase * (theta / two_pi), sharp);

        for j in 0..cols {
            let phi = two_pi * (j as f32) / (cols as f32);
            // Spiral cords: periodic around the aperture, continuous along the
            // coil (depends on phi only) → longitudinal cords spiralling the whorl.
            let spiral = p.rib_sp_amp * ribbed(p.rib_sp_count * phi, sharp);
            // Displace the aperture outward; scale by g so ornament stays
            // proportional (self-similar) along the whole shell.
            let disp = g * (axial + spiral);
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
        // Pin resolution at the cap on both so auto-refinement can't change the
        // topology, making the comparison element-wise.
        let base = ShellParams {
            seg_phi: 256,
            seg_theta: 256,
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
