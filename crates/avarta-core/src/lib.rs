//! Pure mesh generation for spiral shells.
//!
//! Layer-1 coiling geometry only (see `parameters.md`): a single tube swept
//! along a logarithmic helico-spiral. Ornamentation / pigment / colour are not
//! handled here — they hook in later by modulating the aperture in `generate`.
//!
//! No JS/wasm dependencies, so this crate is unit-testable with plain `cargo test`.

mod geometry;
mod mesh;
mod noise;
mod params;
mod pigment;

pub use geometry::generate;
pub use mesh::Mesh;
pub use params::{ParamRange, ShellParams, PARAM_RANGES, PIGMENT_RANGES};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::ribbed;
    use std::f32::consts::PI;

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
                assert_eq!(
                    r.default.fract(),
                    0.0,
                    "{}: integer default not whole",
                    r.key
                );
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
            w: 100.0,         // above max 8.0
            d: 5.0,           // above max 0.95
            t: -3.0,          // below min 0.0
            n: 999.0,         // above max 20.0
            aspect: 0.01,     // below min 0.3
            proj_count: -7.0, // below min, integer
            varix_count: 4.7, // integer rounding
            jitter: 2.0,      // above max 1.0
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
                "w" => p.w,
                "d" => p.d,
                "t" => p.t,
                "n" => p.n,
                "aspect" => p.aspect,
                "rib_ax_count" => p.rib_ax_count,
                "rib_ax_amp" => p.rib_ax_amp,
                "rib_sp_count" => p.rib_sp_count,
                "rib_sp_amp" => p.rib_sp_amp,
                "rib_sharp" => p.rib_sharp,
                "proj_count" => p.proj_count,
                "proj_rows" => p.proj_rows,
                "proj_pos" => p.proj_pos,
                "proj_size" => p.proj_size,
                "proj_sharp" => p.proj_sharp,
                "varix_count" => p.varix_count,
                "varix_amp" => p.varix_amp,
                "seed" => p.seed,
                "jitter" => p.jitter,
                other => panic!("untested key {other}"),
            };
            assert!(
                v >= r.min && v <= r.max,
                "{} out of range after clamp: {v}",
                r.key
            );
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
            ShellParams {
                proj_count: 8.0,
                proj_rows: 1.0,
                proj_pos: 1.1,
                proj_size: 0.8,
                proj_sharp: 0.75,
                ..base.clone()
            },
            // multi-row blunt nodules
            ShellParams {
                proj_count: 12.0,
                proj_rows: 2.0,
                proj_size: 0.12,
                proj_sharp: 0.15,
                ..base.clone()
            },
            ShellParams {
                varix_count: 3.0,
                varix_amp: 0.3,
                ..base.clone()
            },
        ];
        for (k, v) in variants.iter().enumerate() {
            let m = generate(v);
            assert_eq!(
                m.positions.len(),
                smooth.positions.len(),
                "variant {k} topology"
            );
            let moved = smooth
                .positions
                .iter()
                .zip(&m.positions)
                .any(|(a, b)| (a - b).abs() > 1e-4);
            assert!(moved, "variant {k} should change the surface");
            assert!(
                m.positions.iter().all(|x| x.is_finite()),
                "variant {k} finite"
            );
        }
    }

    #[test]
    fn jitter_zero_is_identical_regardless_of_seed() {
        let a = generate(&ShellParams::default());
        let b = generate(&ShellParams {
            seed: 9999.0,
            jitter: 0.0,
            ..ShellParams::default()
        });
        assert_eq!(
            a.positions, b.positions,
            "jitter=0 must ignore the seed exactly"
        );
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
        let b = generate(&ShellParams {
            seed: 2.0,
            ..p1.clone()
        });
        let uniform = generate(&ShellParams {
            jitter: 0.0,
            ..p1.clone()
        });

        assert_eq!(
            a.positions, a2.positions,
            "same seed+params must reproduce exactly"
        );
        assert_eq!(a.positions.len(), b.positions.len());
        assert!(
            a.positions
                .iter()
                .zip(&b.positions)
                .any(|(x, y)| (x - y).abs() > 1e-5),
            "different seeds should produce different shapes"
        );
        assert!(
            a.positions
                .iter()
                .zip(&uniform.positions)
                .any(|(x, y)| (x - y).abs() > 1e-5),
            "jitter should perturb the surface vs the uniform shape"
        );
    }

    #[test]
    fn seed_visibly_changes_spacing_not_just_height() {
        // Regression: at jitter=1 two seeds must differ *substantially* (irregular
        // spacing, not a cosmetic global rotation). Measured ~0.08 max displacement
        // with ~89% of vertices moving; assert well below that so it can't silently
        // regress to the old amplitude-only level. Topology is seed-independent, so
        // the vertex arrays line up element-wise.
        let base = ShellParams {
            varix_count: 4.0,
            varix_amp: 0.3,
            jitter: 1.0,
            ..ShellParams::default()
        };
        let a = generate(&ShellParams {
            seed: 1.0,
            ..base.clone()
        });
        let b = generate(&ShellParams {
            seed: 2.0,
            ..base.clone()
        });
        assert_eq!(
            a.positions.len(),
            b.positions.len(),
            "seed must not change topology"
        );
        let n = a.positions.len() / 3;
        let disp = |i: usize| -> f32 {
            ((a.positions[i * 3] - b.positions[i * 3]).powi(2)
                + (a.positions[i * 3 + 1] - b.positions[i * 3 + 1]).powi(2)
                + (a.positions[i * 3 + 2] - b.positions[i * 3 + 2]).powi(2))
            .sqrt()
        };
        let max_disp = (0..n).map(disp).fold(0.0f32, f32::max);
        let moved = (0..n).filter(|&i| disp(i) > 0.01).count();
        assert!(
            max_disp > 0.03,
            "seeds barely differ (max disp {max_disp}) — jitter too weak"
        );
        assert!(
            moved * 100 / n >= 40,
            "only {}% of vertices moved between seeds — spacing not randomized",
            moved * 100 / n
        );
        assert!(b.positions.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn low_count_varices_do_not_fold_under_max_jitter() {
        // The argument-space phase bound (PH_POS) must keep `count·θ + phase`
        // monotonic even at one feature per whorl, so the swept tube never folds
        // back on itself (which would make a self-intersecting, NaN-free but
        // broken mesh). Smoke test the tightest case stays finite and bounded.
        for &count in &[1.0, 2.0, 3.0] {
            let m = generate(&ShellParams {
                varix_count: count,
                varix_amp: 0.5,
                jitter: 1.0,
                seed: 7.0,
                ..ShellParams::default()
            });
            assert!(
                m.positions
                    .iter()
                    .all(|x| x.is_finite() && x.abs() <= 1.001),
                "count {count} unbounded/NaN"
            );
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
        let blunt = generate(&ShellParams {
            proj_sharp: 0.1,
            ..common.clone()
        });
        let needle = generate(&ShellParams {
            proj_sharp: 1.0,
            ..common.clone()
        });
        assert!(
            needle.positions.len() > blunt.positions.len(),
            "a sharp needle should refine denser than a blunt bead ({} vs {})",
            needle.positions.len(),
            blunt.positions.len()
        );
    }

    #[test]
    fn tessellation_refines_for_high_frequency_cords() {
        let plain = generate(&ShellParams {
            seg_phi: 48,
            ..ShellParams::default()
        });
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
        let max_y = m
            .positions
            .iter()
            .skip(1)
            .step_by(3)
            .fold(0.0f32, |a, &y| a.max(y.abs()));
        assert!(
            max_y <= g_max * 1.01,
            "planispiral too tall: {max_y} vs {g_max}"
        );
    }

    // --- Layer 3: pigmentation -------------------------------------------------

    #[test]
    fn pigment_table_is_wellformed() {
        let len = PIGMENT_RANGES.len();
        let mut keys: Vec<_> = PIGMENT_RANGES.iter().map(|r| r.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), len, "duplicate or missing PIGMENT_RANGES key");
        for r in PIGMENT_RANGES {
            assert!(r.min <= r.max, "{}: min > max", r.key);
            assert!(r.step > 0.0, "{}: non-positive step", r.key);
            assert!(
                r.default >= r.min && r.default <= r.max,
                "{}: default out of range",
                r.key
            );
        }
        // Defaults must agree with the `Default` impl (single source of truth).
        let d = ShellParams::default();
        let by = |k| PIGMENT_RANGES.iter().find(|r| r.key == k).unwrap().default;
        assert_eq!(d.pig_regime, by("pig_regime"));
        assert_eq!(d.pig_scale, by("pig_scale"));
        assert_eq!(d.pig_contrast, by("pig_contrast"));
        assert_eq!(d.pig_density, by("pig_density"));
        assert_eq!(d.pig_angle, by("pig_angle"));
        assert_eq!(d.pig_irregularity, by("pig_irregularity"));
    }

    #[test]
    fn pigment_field_is_well_formed_and_solid_is_uniform() {
        let m = generate(&ShellParams {
            pig_regime: 0.0,
            ..ShellParams::default()
        });
        assert!(m.pig_w > 0 && m.pig_h > 0);
        assert_eq!(m.pigment.len(), (m.pig_w * m.pig_h) as usize);
        assert!(
            m.pigment.iter().all(|&b| b == m.pigment[0]),
            "solid regime must be uniform"
        );
    }

    #[test]
    fn patterned_regimes_are_nonuniform_and_deterministic() {
        for regime in 1..=6 {
            let p = ShellParams {
                pig_regime: regime as f32,
                pig_irregularity: 0.3,
                seed: 3.0,
                ..ShellParams::default()
            };
            let a = generate(&p);
            let b = generate(&p);
            assert_eq!(a.pigment, b.pigment, "regime {regime} not reproducible");
            let first = a.pigment[0];
            assert!(
                a.pigment.iter().any(|&x| x != first),
                "regime {regime} produced no pattern"
            );
        }
    }

    #[test]
    fn pigment_is_periodic_around_the_lip() {
        // φ wraps (closed lip): the first and last rows are adjacent cells, so on
        // average they must be continuous — no hard seam where the texture wraps.
        let m = generate(&ShellParams {
            pig_regime: 1.0,
            ..ShellParams::default()
        });
        let (w, h) = (m.pig_w as usize, m.pig_h as usize);
        let mut diff = 0u32;
        for i in 0..w {
            let top = m.pigment[i] as i32;
            let bot = m.pigment[(h - 1) * w + i] as i32;
            diff += (top - bot).unsigned_abs();
        }
        let avg = diff as f32 / w as f32;
        assert!(avg < 90.0, "φ seam discontinuity too large: {avg}");
    }

    #[test]
    fn pigment_params_do_not_change_geometry() {
        // Pigmentation is a second output of the growth sweep; it must never move
        // a vertex. (Shape params still drive both, but pig_* are pattern-only.)
        let a = generate(&ShellParams::default());
        let b = generate(&ShellParams {
            pig_regime: 4.0,
            pig_scale: 0.8,
            pig_density: 0.9,
            pig_angle: 0.7,
            pig_irregularity: 0.5,
            seed: 12.0,
            ..ShellParams::default()
        });
        assert_eq!(
            a.positions, b.positions,
            "pigment params must not affect geometry"
        );
    }
}
