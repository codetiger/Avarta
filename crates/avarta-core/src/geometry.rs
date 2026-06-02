//! Layer-1 coiling geometry and Layer-2 ornamentation: the `generate` sweep.
//!
//! Sweeps an elliptical aperture along a logarithmic helico-spiral, modulating
//! it with ribs / cords / projections / varices and seeded jitter, then
//! normalises, orients, and attaches the Layer-3 pigment field.
//!
//! `generate` is deliberately a short recipe: it computes the feature profile
//! and coil constants, then delegates to one helper per phase —
//! [`plan_tessellation`] (how many rows/columns and where), [`sweep_surface`]
//! (the aperture sweep → positions/uvs), [`build_indices`], [`smooth_normals`],
//! [`normalize_to_unit_sphere`], and [`orient_for_display`]. Each helper is
//! self-contained and individually testable.

use crate::mesh::Mesh;
use crate::noise::{lobe, noise1, rand_signed, ribbed};
use crate::params::ShellParams;
use crate::pigment::pigment_field;
use std::f32::consts::PI;

/// Varix profile exponent — rounded raised ridges. Shared by tessellation
/// planning (how narrow the varix is) and the sweep (the `lobe` it draws).
const VARIX_POWER: f32 = 3.0;

/// Trapezoid-integrate the per-radian row density `rho` over `[0, total]` and
/// return the number of θ segments, `round(∫ρ dθ).max(1)`. This is how many
/// rows the graded sweep needs: where the coil is small (small `g`) `rho` is
/// low and contributes few rows, so the integral is far below `seg_theta·n` for
/// high-`W` shells, yet recovers `≈ seg_theta·n` when `rho` is ~constant (W≈1).
/// `min_grid` forces a finer integration grid when `rho` has high-frequency
/// structure (e.g. the per-bead projection window), so the integral is accurate.
fn estimate_n_seg(total: f32, rho: impl Fn(f32) -> f32, min_grid: usize) -> usize {
    let m = 1024usize.max(min_grid);
    let dx = total / m as f32;
    let mut acc = 0.0f32;
    let mut prev = rho(0.0);
    for s in 1..=m {
        let cur = rho(s as f32 * dx);
        acc += 0.5 * (prev + cur) * dx;
        prev = cur;
    }
    (acc.round() as usize).max(1)
}

/// Place `theta_verts` θ values in `[0, total]` whose local spacing follows the
/// density `rho` (more rows where `rho` is high), via inverse-transform
/// sampling: integrate `rho` to a cumulative-density array on a fine grid, then
/// for each output row find the θ where the CDF crosses `(i/n_seg)·total`.
/// Endpoints are pinned exactly to `0` and `total`, so the apex seam and the
/// aperture mouth are preserved. The returned list is monotone non-decreasing.
fn graded_thetas(
    total: f32,
    theta_verts: usize,
    rho: impl Fn(f32) -> f32,
    min_grid: usize,
) -> Vec<f32> {
    let n_seg = theta_verts.saturating_sub(1).max(1);
    // Fine integration grid (finer than the output rows) for the CDF.
    let m = (8 * n_seg).max(2048).max(min_grid);
    let dx = total / m as f32;
    let mut cdf = vec![0.0f32; m + 1];
    let mut prev = rho(0.0);
    for s in 1..=m {
        let cur = rho(s as f32 * dx);
        cdf[s] = cdf[s - 1] + 0.5 * (prev + cur) * dx;
        prev = cur;
    }
    let cap = cdf[m].max(1e-6);
    let mut out = Vec::with_capacity(theta_verts);
    out.push(0.0);
    let mut s = 0usize; // walking index into cdf
    for i in 1..n_seg {
        let target = (i as f32 / n_seg as f32) * cap;
        while s < m && cdf[s + 1] < target {
            s += 1;
        }
        // Linear interpolation of θ within the crossing bin [s, s+1].
        let (lo, hi) = (cdf[s], cdf[s + 1]);
        let frac = if hi > lo {
            (target - lo) / (hi - lo)
        } else {
            0.0
        };
        out.push((s as f32 + frac) * dx);
    }
    out.push(total); // endpoint pinned exactly
    out
}

/// Feature profile exponents (how narrow each ornament's peak is) plus the
/// projection on/off flag — shared by tessellation planning and the sweep so
/// the two agree on how sharp every feature is.
struct Profiles {
    /// Rib/cord raised-cosine exponent — matches `ribbed`.
    rib_power: f32,
    /// Projection lobe exponent — matches the `lobe` use for beads/spines.
    proj_power: f32,
    /// Whether projections contribute at all (size, count and rows all non-trivial).
    proj_active: bool,
}

impl Profiles {
    fn new(p: &ShellParams) -> Self {
        Self {
            rib_power: 1.0 + p.rib_sharp.clamp(0.0, 1.0) * 8.0, // matches `ribbed`
            proj_power: 2.0 + p.proj_sharp.clamp(0.0, 1.0).powi(2) * 40.0, // matches `lobe` use
            proj_active: p.proj_size.abs() > 1e-6 && p.proj_count > 0.5 && p.proj_rows > 0.5,
        }
    }
}

/// The resolved tessellation: where every row sits along the coil and the
/// cross-section topology (columns + the duplicated φ-seam column).
struct Tessellation {
    /// θ value of each row (non-uniform, graded). Length == `theta_verts`.
    theta_list: Vec<f32>,
    /// Number of rows along the coil (`theta_steps + 1`).
    theta_verts: usize,
    /// Number of θ segments between rows.
    theta_steps: usize,
    /// Columns around the aperture (the real φ samples).
    cols: usize,
    /// Per-ring vertex stride: `cols + 1` (the extra column is the φ-seam
    /// duplicate of col 0 that carries v = 1.0 — see [`sweep_surface`]).
    stride: usize,
}

/// Decide how finely to tessellate and where to place each row.
///
/// Auto-refines the θ/φ resolution from the *impacting* ornament parameters,
/// then grades the row placement so tiny inner whorls get few rows and the body
/// whorl gets many (constant arc-length per segment). Pure function of `p` and
/// the feature `prof`.
fn plan_tessellation(p: &ShellParams, prof: &Profiles) -> Tessellation {
    let n = p.n;
    let two_pi = 2.0 * PI;
    let total_theta = n * two_pi;
    let k = p.w.ln() / two_pi; // growth rate: g = exp(k·theta) grows by W each turn

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

    // Every feature is a radial displacement, and the chord (faceting) error of
    // sampling one at N points per cycle is ∝ amplitude / N² — so the samples
    // needed for a fixed visual error scale with √amplitude, not amplitude alone.
    // A faint cord (or a subtle bead) therefore needs far fewer segments than a
    // bold one. We calibrate against `AMP_REF`: at/above it a feature gets `spc`
    // exactly (no quality change), and fainter features cost proportionally less.
    // Because amplitude ≤ AMP_REF the factor is ≤ 1 — this only ever *reduces* the
    // mesh.
    const MIN_SPC: f32 = 6.0;
    const AMP_REF: f32 = 0.6; // amplitude at/above which a feature keeps full sampling
                              // Ribs/cords are continuous two-sided waves: a flat floor still captures the
                              // oscillation even where it binds.
    let spc_amp =
        |amp: f32, power: f32| (spc(power) * (amp.min(AMP_REF) / AMP_REF).sqrt()).max(MIN_SPC);
    // Projections are *localised* beads/spikes, so the floor scales with √power:
    // even a faint but sharp spine keeps enough samples to resolve its narrow peak
    // instead of vanishing between segments (a flat floor could drop the spike).
    let spc_proj = |size: f32, power: f32| {
        (spc(power) * (size.min(AMP_REF) / AMP_REF).sqrt()).max(MIN_SPC * power.sqrt())
    };

    // `theta_need` is the *uniform* θ density — features that run continuously
    // along the coil (rib waves, varices) need it on every whorl. Projections are
    // different: they are isolated beads, so their density is needed only *near
    // each bead*, not across the whole whorl (see the localised window below).
    let mut theta_need = 0.0f32; // uniform along the coil (ribs, varices)
    let mut phi_need = 0.0f32; // features periodic around the aperture
    if p.rib_ax_amp.abs() > 1e-6 {
        theta_need = theta_need.max(p.rib_ax_count * spc_amp(p.rib_ax_amp, prof.rib_power));
    }
    if p.rib_sp_amp.abs() > 1e-6 {
        phi_need = phi_need.max(p.rib_sp_count * spc_amp(p.rib_sp_amp, prof.rib_power));
    }
    if p.varix_amp.abs() > 1e-6 {
        theta_need = theta_need.max(p.varix_count * spc(VARIX_POWER));
    }
    // Projection peak θ density (applied only inside each bead's window) and the
    // window half-width: the bead occupies ±`proj_bead_w` (argument radians, the
    // lobe's 10%-height span) of its cycle, widened by the jitter range so a
    // jittered bead stays covered. Between beads the coil reverts to the uniform
    // baseline.
    let mut proj_peak_need = 0.0f32;
    let mut proj_bead_w = 0.0f32;
    if prof.proj_active {
        let s = spc_proj(p.proj_size, prof.proj_power);
        proj_peak_need = p.proj_count * s;
        phi_need = phi_need.max(p.proj_rows.max(1.0) * s);
        let half = (0.1f32.powf(1.0 / prof.proj_power)).acos(); // lobe drops to 10% here
        let jit_range = 1.5 * p.jitter.clamp(0.0, 1.0); // max bead θ-phase shift
        proj_bead_w = (half + jit_range).min(PI);
    }
    let theta_need_peak = theta_need.max(proj_peak_need); // densest θ anywhere

    let base_theta = p.seg_theta.max(3);
    let base_phi = p.seg_phi.max(3);
    // seg_theta is sized for the *peak* (so MAX_THETA / the vertex budget bound the
    // densest case), while seg_theta_geom — used for the smooth-tube baseline —
    // excludes the projection peak, so the coil between beads is not over-sampled.
    let mut seg_theta = (base_theta as f32)
        .max(theta_need_peak)
        .ceil()
        .min(MAX_THETA as f32) as u32;
    let mut seg_theta_geom = (base_theta as f32).max(theta_need).min(MAX_THETA as f32);
    let mut seg_phi = (base_phi as f32).max(phi_need).ceil().min(MAX_PHI as f32) as u32;
    // Scale both down together if the total vertex count would blow the budget.
    let est = seg_theta as f32 * n * seg_phi as f32;
    if est > VERT_BUDGET {
        let s = (VERT_BUDGET / est).sqrt();
        seg_theta = ((seg_theta as f32 * s) as u32).max(base_theta);
        seg_phi = ((seg_phi as f32 * s) as u32).max(base_phi);
    }
    seg_theta_geom = seg_theta_geom.min(seg_theta as f32); // never exceed the peak cap
    let cols = seg_phi as usize;
    // One extra column per ring duplicates col 0 (the φ = 0 ≡ 2π seam) so the
    // closed tube can carry v = 1.0 at the wrap instead of folding the UV back to
    // 0 across the final triangle strip — see the seam handling in the sweep below.
    let stride = cols + 1;

    // Graded θ-tessellation: place rows by a per-radian density `rho(θ)` instead
    // of uniformly, so the tiny inner whorls get few rows and the body whorl gets
    // many. Arc length per radian along the coil scales with the local radius
    // (∝ g = e^{kθ}), so for constant arc-length-per-segment the geometry density
    // is `c_geom·g`, normalised so the body whorl (g = g_max) keeps the smooth-
    // tube + rib density (`seg_theta_geom`). A θ-constant floor — the uniform
    // feature need and a minimum density — keeps continuous ornament sampled and
    // stops inner whorls degenerating. When W≈1, g is ~flat → uniform spacing.
    //
    // The floor is an *angular*-smoothness bound, not a size one: faceting is
    // scale-independent (an N-gon's relative chord error is `1 − cos(π/N)`, so a
    // tiny inner whorl looks just as polygonal as a big one at the same row count).
    // 32 rows/whorl ⇒ ~0.5 % chord error (visually round); 8 gave ~7.6 % — the
    // octagonal/"hexagonal" apex the grading optimisation introduced.
    const MIN_DENSITY_PER_WHORL: f32 = 32.0;
    let g_max = (k * total_theta).exp(); // = W^n
    let c_geom = seg_theta_geom / (g_max * two_pi);
    // Baseline floor capped at `seg_theta_geom` (so it never inflates the smooth-
    // tube density), and the localised projection peak capped at `seg_theta`. The
    // window is 1 within ±`proj_bead_w` of each bead centre and ramps to 0 just
    // outside, so the projection's dense sampling lands *only on the beads*; the
    // rest of the coil keeps the cheap baseline. Both peaks ≤ `seg_theta`/2π, so
    // `rho ≤ seg_theta/2π` everywhere ⇒ `N ≤ seg_theta·n` (the budget bound holds).
    let floor_uniform = seg_theta_geom.min(MIN_DENSITY_PER_WHORL.max(theta_need)) / two_pi;
    let proj_peak_density = (seg_theta as f32).min(proj_peak_need) / two_pi;
    let proj_count = p.proj_count;
    let proj_active = prof.proj_active;
    const PROJ_BAND: f32 = 0.3; // smooth window edge (argument radians)
    let rho = |theta: f32| {
        let mut d = (c_geom * (k * theta).exp()).max(floor_uniform);
        if proj_active {
            // distance (argument radians) from the nearest bead centre
            let cyc = proj_count * theta / two_pi;
            let dist = (cyc - cyc.round()).abs() * two_pi;
            let w = (1.0 - (dist - proj_bead_w) / PROJ_BAND).clamp(0.0, 1.0);
            d = d.max(proj_peak_density * w);
        }
        d
    };

    // The projection window oscillates `proj_count·n` times along the coil; the
    // integration grid must resolve it (≳24 samples per bead) for an accurate CDF.
    let min_grid = if proj_active {
        (24.0 * proj_count * n).ceil() as usize
    } else {
        0
    };
    // Total segments along the coil ≈ ∫ρ dθ; rows are then placed by inverse-CDF.
    let theta_steps = estimate_n_seg(total_theta, rho, min_grid);
    let theta_verts = theta_steps + 1;
    let theta_list = graded_thetas(total_theta, theta_verts, rho, min_grid);

    Tessellation {
        theta_list,
        theta_verts,
        theta_steps,
        cols,
        stride,
    }
}

/// Sweep the elliptical aperture along the coil into `(positions, uvs)`,
/// modulating it with ribs / cords / projections / varices and seeded jitter.
///
/// `theta` runs along the coil (0 .. 2π·n); `phi` runs around the aperture.
/// The aperture and its distance from the axis both scale by `g = W^(theta/2π)`,
/// which keeps the form self-similar (why shells are logarithmic spirals).
fn sweep_surface(p: &ShellParams, prof: &Profiles, tess: &Tessellation) -> (Vec<f32>, Vec<f32>) {
    let d = p.d;
    let aspect = p.aspect;
    let two_pi = 2.0 * PI;
    let total_theta = p.n * two_pi;
    let k = p.w.ln() / two_pi;
    let proj_active = prof.proj_active;
    let proj_power = prof.proj_power;

    let theta_verts = tess.theta_verts;
    let stride = tess.stride;
    let cols = tess.cols;
    let theta_list = &tess.theta_list;

    let mut positions = Vec::with_capacity(theta_verts * stride * 3);
    let mut uvs = Vec::with_capacity(theta_verts * stride * 2);

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

    for &theta in theta_list.iter() {
        // graded (non-uniform) row positions
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

        // u tracks the *actual* growth-time fraction (not the row index), so the
        // pigment field — generated on its own uniform-in-θ grid and sampled with
        // wrapS=clamp — stays locked to the geometry under non-uniform rows.
        let u = theta / total_theta; // 0..1 along the coil

        // `col == cols` is the seam duplicate: it reuses col 0's φ (so its
        // position and ornament coincide exactly with col 0, keeping the tube
        // closed) but carries v = 1.0, so the final triangle strip interpolates v
        // up to 1.0 instead of folding it back to 0 — which, under wrapT=repeat,
        // smeared the whole pigment pattern into that one seam strip.
        for col in 0..=cols {
            let cw = col % cols; // seam column wraps to col 0
            let phi = two_pi * (cw as f32) / (cols as f32);
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
            uvs.push(col as f32 / cols as f32); // v around the aperture: 0 .. 1 (seam = 1.0)
        }
    }

    (positions, uvs)
}

/// Build the triangle index buffer for the swept grid. φ closes via the seam
/// duplicate column (`stride = cols + 1`), so `j+1` walks straight into it — no
/// modulo wrap, and the closing strip connects col `cols-1` to the v = 1.0 seam
/// vertex.
fn build_indices(tess: &Tessellation) -> Vec<u32> {
    let Tessellation {
        theta_steps,
        cols,
        stride,
        ..
    } = *tess;
    let mut indices = Vec::with_capacity(theta_steps * cols * 6);
    let vid = |i: usize, j: usize| -> u32 { (i * stride + j) as u32 };
    for i in 0..theta_steps {
        for j in 0..cols {
            let a = vid(i, j);
            let b = vid(i + 1, j);
            let c = vid(i + 1, j + 1);
            let e = vid(i, j + 1);
            indices.extend_from_slice(&[a, b, c, a, c, e]);
        }
    }
    indices
}

/// Smooth per-vertex normals: accumulate face normals onto their vertices, fuse
/// the φ-seam duplicate with col 0 so the wrap shows no lighting seam, then
/// normalise. Returns a buffer parallel to `positions`.
fn smooth_normals(positions: &[f32], indices: &[u32], tess: &Tessellation) -> Vec<f32> {
    let Tessellation {
        theta_verts,
        cols,
        stride,
        ..
    } = *tess;
    let mut normals = vec![0.0f32; theta_verts * stride * 3];

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
    // The seam duplicate (col == cols) and col 0 are the same physical vertex, so
    // each only gathered the faces on its own side of the φ wrap. Sum them so both
    // carry the full normal — otherwise the wrap would show a lighting seam.
    for i in 0..theta_verts {
        let (a, b) = ((i * stride) * 3, (i * stride + cols) * 3);
        for k in 0..3 {
            let s = normals[a + k] + normals[b + k];
            normals[a + k] = s;
            normals[b + k] = s;
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
    normals
}

/// Translate to the centroid and uniformly scale `positions` to the unit sphere.
///
/// Raw coords span a huge range (g = e^(kθ) can reach thousands), which wrecks
/// downstream float precision (BVH / ray tracing). Translate + uniform scale
/// leave the already-unit normals unchanged, so normals are not touched here.
fn normalize_to_unit_sphere(positions: &mut [f32]) {
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
}

/// Rotate the centred mesh into the canonical display pose: coil axis vertical,
/// cone pointing down, body whorl facing the camera (+Z). Rotations preserve the
/// unit normalisation and the (already unit) normals, so both buffers are
/// rotated in place.
///
/// The coil is built around +Z with the apex (smallest whorl) at the low-z end
/// and the body whorl / aperture at the high-z end. Two rotations about the now-
/// centred origin pose it: 1) spin about the coil axis (Z) so the body whorl's
/// azimuth points to (0,-1); 2) tip upright with Rx(-90°): (x,y,z) -> (x, z, -y),
/// which sends +Z (coil axis) -> +Y (up), the low-z apex -> -Y (bottom), and the
/// body whorl -> +Z (front).
fn orient_for_display(positions: &mut [f32], normals: &mut [f32], tess: &Tessellation) {
    let Tessellation {
        theta_verts,
        cols,
        stride,
        ..
    } = *tess;
    let last = (theta_verts - 1) * stride;
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
}

/// Generate a shell surface by sweeping an elliptical aperture along a
/// logarithmic helico-spiral, then attaching the Layer-3 pigment field.
///
/// The pipeline: clamp → plan tessellation → sweep the aperture → build indices
/// → smooth normals → unit-normalise → orient for display → attach pigment. Each
/// step is a self-contained helper above.
pub fn generate(p: &ShellParams) -> Mesh {
    // Single, total clamp: every shape field is now guaranteed within its
    // `PARAM_RANGES` bound, so the rest of the pipeline (tessellation / mesh math)
    // never sees an out-of-range value.
    let p = p.clamped();
    let p = &p;

    let prof = Profiles::new(p);
    let tess = plan_tessellation(p, &prof);

    let (mut positions, uvs) = sweep_surface(p, &prof, &tess);
    let indices = build_indices(&tess);
    let mut normals = smooth_normals(&positions, &indices, &tess);
    normalize_to_unit_sphere(&mut positions);
    orient_for_display(&mut positions, &mut normals, &tess);

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
