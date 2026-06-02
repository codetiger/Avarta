//! Layer-1 coiling geometry and Layer-2 ornamentation: the `generate` sweep.
//!
//! Sweeps an elliptical aperture along a logarithmic helico-spiral, modulating
//! it with ribs / cords / projections / varices and seeded jitter, then
//! normalises, orients, and attaches the Layer-3 pigment field.

use crate::mesh::Mesh;
use crate::noise::{lobe, noise1, rand_signed, ribbed};
use crate::params::ShellParams;
use crate::pigment::pigment_field;
use std::f32::consts::PI;

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
    let mut seg_theta = (base_theta as f32)
        .max(theta_need)
        .ceil()
        .min(MAX_THETA as f32) as u32;
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
    // Distinct salts so the different effects are decorrelated. Each θ-periodic
    // feature gets its own *position* salt (irregular spacing) and *amplitude*
    // salt (irregular height).
    const S_AXPOS: u32 = 0x9E37_79B1; // rib axial θ-phase (spacing)
    const S_VXPOS: u32 = 0x85EB_CA77; // varix θ-phase (spacing)
    const S_PRPOS: u32 = 0xC2B2_AE3D; // projection θ-phase (spacing)
    const S_AXAMP: u32 = 0x27D4_EB2F;
    const S_VXAMP: u32 = 0x1656_67B1;
    const S_PRAMP: u32 = 0xD3A2_646C;
    const S_SPAMP: u32 = 0xFD70_46C5;
    const S_PHIWARP: u32 = 0x7F4A_7C15;
    const S_COIL: u32 = 0xB55A_4F09;

    // Position jitter is applied in *argument space* (`count·θ + phase`), so a
    // fixed phase amplitude shifts every feature by the same fraction of its own
    // spacing regardless of count — coarse varices and fine ribs alike get
    // irregular *spacing*, not just height. `PH_POS / 2π ≈ 0.24` ⇒ ±24 % of a
    // feature's gap at jitter 1. The amplitude/frequency are bounded so
    // `count·θ + phase` stays monotonic (no mesh folds) down to one feature per
    // whorl: worst-case dphase/dθ ≈ jit·PH_POS·3·freq/2π < 1 ≤ count.
    const PH_POS: f32 = 1.5;

    for i in 0..theta_verts {
        let theta = total_theta * (i as f32) / (theta_steps as f32);
        let g = (k * theta).exp();
        let ap_r = aspect * g; // aperture radial semi-axis
        let ap_z = g; // aperture axial semi-axis
        let ct = theta.cos();
        let st = theta.sin();

        // Per-θ seeded jitter: an independent θ-phase per feature (so spacing
        // goes irregular and features drift across whorls instead of locking to
        // the same angle), per-instance amplitude wobble, a φ-shift (cords
        // meander), and a subtle coil radius wobble.
        let mut radius_wob = 1.0;
        let mut ax_ampj = 1.0;
        let mut varix_ampj = 1.0;
        let mut proj_ampj = 1.0;
        let mut phi_shift = 0.0;
        let (mut ax_ph, mut varix_ph, mut proj_ph) = (0.0f32, 0.0f32, 0.0f32);
        if jittered {
            let whorl = theta / two_pi;
            // Smooth, bounded θ-phase (argument-space) → irregular feature spacing.
            let phase = |salt: u32, freq: f32| jit * PH_POS * noise1(seed ^ salt, whorl * freq);
            ax_ph = phase(S_AXPOS, 1.0);
            varix_ph = phase(S_VXPOS, 0.85);
            proj_ph = phase(S_PRPOS, 0.95);
            radius_wob = 1.0 + jit * 0.04 * noise1(seed ^ S_COIL, whorl * 1.1 + 11.0);
            ax_ampj = 1.0
                + jit * 0.5 * rand_signed(seed ^ S_AXAMP, (p.rib_ax_count * whorl).round() as i32);
            varix_ampj = 1.0
                + jit * 0.7 * rand_signed(seed ^ S_VXAMP, (p.varix_count * whorl).round() as i32);
            proj_ampj = 1.0
                + jit * 0.6 * rand_signed(seed ^ S_PRAMP, (p.proj_count * whorl).round() as i32);
            phi_shift = jit * 0.12 * noise1(seed ^ S_PHIWARP, theta * 0.4);
        }

        let radius = (ap_r / (1.0 - d)) * radius_wob; // axis → centre, with coil wobble
        let cz = p.t * radius; // centre height: ∝ radius gives the conical spire

        // θ-only ornament terms (per-feature phase → irregular spacing, and
        // features don't lock to the same angle every whorl).
        let axial = p.rib_ax_amp * ax_ampj * ribbed(p.rib_ax_count * theta + ax_ph, sharp);
        let varix = p.varix_amp * varix_ampj * lobe(p.varix_count * theta + varix_ph, VARIX_POWER);
        let proj_theta = lobe(p.proj_count * theta + proj_ph, proj_power);

        let u = i as f32 / theta_steps as f32; // 0..1 along the coil

        for col in 0..cols {
            let phi = two_pi * (col as f32) / (cols as f32);
            // warped φ for ornament placement
            let phi_o = phi + phi_shift;
            // Spiral cords: continuous along the coil → longitudinal cords.
            let cord_ampj = if jittered {
                1.0 + jit
                    * 0.45
                    * rand_signed(
                        seed ^ S_SPAMP,
                        (p.rib_sp_count * phi / two_pi).round() as i32,
                    )
            } else {
                1.0
            };
            let spiral = p.rib_sp_amp * cord_ampj * ribbed(p.rib_sp_count * phi_o, sharp);
            // Projections: blunt beads (rows≥2, low sharp) → needle spines
            // (rows=1, high sharp), localised on a θ×φ grid offset by proj_pos.
            let proj = if proj_active {
                p.proj_size
                    * proj_ampj
                    * proj_theta
                    * lobe(p.proj_rows * (phi_o - p.proj_pos), proj_power)
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
        let pa = [
            positions[ia * 3],
            positions[ia * 3 + 1],
            positions[ia * 3 + 2],
        ];
        let pb = [
            positions[ib * 3],
            positions[ib * 3 + 1],
            positions[ib * 3 + 2],
        ];
        let pc = [
            positions[ic * 3],
            positions[ic * 3 + 1],
            positions[ic * 3 + 2],
        ];
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

    // Layer 3: pigment laid down by the same growth process (shares `seed` and
    // the coil extent); independent grid resolution, mapped via the UVs above.
    let (pigment, pig_w, pig_h) = pigment_field(p);

    Mesh {
        positions,
        normals,
        uvs,
        indices,
        pigment,
        pig_w,
        pig_h,
    }
}
