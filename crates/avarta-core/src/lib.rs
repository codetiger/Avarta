//! Pure mesh generation for spiral shells.
//!
//! Layer-1 coiling geometry only (see `parameters.md`): a single tube swept
//! along a logarithmic helico-spiral. Ornamentation / pigment / colour are not
//! handled here — they hook in later by modulating the aperture in `generate`.
//!
//! No JS/wasm dependencies, so this crate is unit-testable with plain `cargo test`.

mod geometry;
mod idcodec;
mod mesh;
mod noise;
mod params;
mod pigment;

pub use geometry::generate;
pub use mesh::Mesh;
pub use params::{
    decode_id, encode_id, IdError, ParamRange, ShellParams, PARAM_RANGES, PIGMENT_RANGES,
};

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
        // `Default` is derived from the tables, so every shape field must equal
        // its table default — which also confirms each `field` arm points at the
        // right struct field.
        let d = ShellParams::default();
        for r in PARAM_RANGES {
            assert_eq!(
                d.field(r.key),
                Some(r.default),
                "{} default mismatch",
                r.key
            );
        }
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
            let v = p
                .field(r.key)
                .expect("every PARAM_RANGES key resolves via field");
            assert!(
                v >= r.min && v <= r.max,
                "{} out of range after clamp: {v}",
                r.key
            );
        }
    }

    #[test]
    fn share_id_round_trips_within_one_step() {
        // Encoding quantises each param to its slider step, so decoding returns a
        // value within half a step of the original — and re-encoding a decoded id
        // is byte-identical (the id is stable / idempotent).
        let samples = [
            ShellParams::default(),
            ShellParams {
                w: 3.27,
                d: 0.4,
                t: 6.1,
                n: 12.3,
                aspect: 2.5,
                rib_ax_count: 18.0,
                rib_ax_amp: 0.33,
                rib_sp_count: 25.0,
                rib_sp_amp: 0.21,
                rib_sharp: 0.7,
                proj_count: 11.0,
                proj_rows: 3.0,
                proj_pos: 2.4,
                proj_size: 0.55,
                proj_sharp: 0.8,
                varix_count: 4.0,
                varix_amp: 0.28,
                seed: 4242.0,
                jitter: 0.65,
                pig_regime: 5.0,
                pig_scale: 0.3,
                pig_contrast: 0.9,
                pig_density: 0.15,
                pig_angle: 0.6,
                pig_irregularity: 0.45,
                ..ShellParams::default()
            },
        ];
        for p in &samples {
            let id = encode_id(p);
            assert!(
                id.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
                "id must be URL-safe: {id}"
            );
            let q = decode_id(&id).expect("a freshly encoded id must decode");
            let base = p.clamped();
            for r in PARAM_RANGES.iter().chain(PIGMENT_RANGES.iter()) {
                let a = base.field(r.key).unwrap();
                let b = q.field(r.key).unwrap();
                assert!(
                    (a - b).abs() <= r.step * 0.5 + 1e-4,
                    "{}: {a} vs decoded {b} exceeds half a step ({})",
                    r.key,
                    r.step
                );
            }
            assert_eq!(encode_id(&q), id, "re-encoding a decoded id must be stable");
        }
    }

    #[test]
    fn share_id_preserves_integer_and_extreme_params_exactly() {
        // Integer-valued params and table extremes are step-aligned, so they must
        // survive a round trip with no drift at all.
        let p = ShellParams {
            rib_ax_count: 40.0,
            rib_sp_count: 60.0,
            proj_count: 30.0,
            proj_rows: 5.0,
            varix_count: 6.0,
            seed: 255.0,
            pig_regime: 6.0,
            ..ShellParams::default()
        };
        let q = decode_id(&encode_id(&p)).unwrap();
        for key in [
            "rib_ax_count",
            "rib_sp_count",
            "proj_count",
            "proj_rows",
            "varix_count",
            "seed",
            "pig_regime",
        ] {
            assert_eq!(p.field(key), q.field(key), "{key} integer param drifted");
        }
    }

    #[test]
    fn decode_id_rejects_garbage_without_panicking() {
        use crate::idcodec::{base64url_decode, base64url_encode};
        assert!(matches!(decode_id(""), Err(IdError::BadLength)));
        assert!(matches!(decode_id("!!!!"), Err(IdError::BadChar)));
        // Valid chars but a byte 0 (version 0) this build doesn't understand:
        assert!(decode_id("AAAA").is_err());
        let good = encode_id(&ShellParams::default());
        // Truncation (a common copy/paste corruption) → a length error, never a panic.
        assert!(decode_id(&good[..good.len() - 2]).is_err());
        // Correct length but an unknown version byte → BadVersion.
        let mut bytes = base64url_decode(&good).unwrap();
        bytes[0] = 0xFF;
        assert!(matches!(
            decode_id(&base64url_encode(&bytes)),
            Err(IdError::BadVersion)
        ));
    }

    #[test]
    fn share_id_is_compact_for_default_heavy_shells() {
        // The sparse format pays only for non-default params, so a plain shell
        // encodes far shorter than a fully ornamented one, and an all-default
        // shell round-trips through a tiny id.
        let plain = encode_id(&ShellParams::default());
        let loaded = encode_id(&ShellParams {
            rib_ax_count: 14.0,
            rib_ax_amp: 0.2,
            rib_sp_count: 20.0,
            rib_sp_amp: 0.15,
            rib_sharp: 0.5,
            proj_count: 10.0,
            proj_rows: 2.0,
            proj_pos: 1.0,
            proj_size: 0.4,
            proj_sharp: 0.6,
            varix_count: 3.0,
            varix_amp: 0.3,
            seed: 1234.0,
            jitter: 0.5,
            pig_regime: 6.0,
            pig_angle: 0.4,
            pig_irregularity: 0.3,
            ..ShellParams::default()
        });
        assert!(
            plain.len() < loaded.len(),
            "a sparse id should grow with ornament ({} vs {})",
            plain.len(),
            loaded.len()
        );
        // The all-default id still round-trips back to the defaults.
        assert_eq!(decode_id(&plain).map(|p| encode_id(&p)).unwrap(), plain);
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
        let smooth = generate(&ShellParams::default());
        // A single faint rib needs fewer samples than the base resolution, so
        // auto-refine leaves tessellation at the base and topology matches the
        // smooth shell — letting us confirm element-wise that ribs displace the
        // surface. (Amplitude-aware sampling now makes a *realistic* ribbed shell
        // refine to a different topology, so the strong feature is checked below.)
        let one_rib = generate(&ShellParams {
            rib_ax_count: 1.0,
            rib_ax_amp: 0.1,
            rib_sharp: 0.0,
            ..ShellParams::default()
        });
        assert_eq!(one_rib.positions.len(), smooth.positions.len());
        let moved = smooth
            .positions
            .iter()
            .zip(&one_rib.positions)
            .any(|(a, b)| (a - b).abs() > 1e-4);
        assert!(moved, "a rib should displace the surface");
        // A realistic rib+cord shell (the combination that drives vertex count)
        // pushes auto-refine to a finer mesh and stays finite.
        let ribbed = generate(&ShellParams {
            rib_ax_count: 14.0,
            rib_ax_amp: 0.25,
            rib_sp_count: 10.0,
            rib_sp_amp: 0.15,
            rib_sharp: 0.5,
            ..ShellParams::default()
        });
        assert!(
            ribbed.positions.len() > smooth.positions.len(),
            "ribs + cords should refine the mesh ({} vs {})",
            ribbed.positions.len(),
            smooth.positions.len(),
        );
        assert!(ribbed.positions.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn fainter_ribs_and_cords_use_fewer_segments() {
        // The amplitude-aware optimisation: identical counts/sharpness, only the
        // amplitude differs. A bold rib+cord shell must tessellate finer than a
        // faint one (chord error ∝ amp/N² ⇒ samples ∝ √amp), and both finer than
        // a fixed-amplitude design would otherwise force.
        let common = ShellParams {
            rib_ax_count: 20.0,
            rib_sp_count: 30.0,
            rib_sharp: 0.4,
            ..ShellParams::default()
        };
        let faint = generate(&ShellParams {
            rib_ax_amp: 0.05,
            rib_sp_amp: 0.05,
            ..common.clone()
        });
        let bold = generate(&ShellParams {
            rib_ax_amp: 0.6,
            rib_sp_amp: 0.6,
            ..common.clone()
        });
        assert!(
            faint.positions.len() < bold.positions.len(),
            "fainter ribs/cords should use fewer vertices ({} vs {})",
            faint.positions.len(),
            bold.positions.len(),
        );
    }

    #[test]
    fn subtle_beads_use_fewer_segments_than_bold_spikes() {
        // Cerithium nodulosum-style: small, blunt beads on a many-whorl spire must
        // not force the same dense mesh as a bold spike of identical count/sharp.
        let common = ShellParams {
            proj_count: 12.0,
            proj_rows: 2.0,
            proj_pos: 0.6,
            proj_sharp: 0.25,
            n: 9.0,
            w: 1.6,
            ..ShellParams::default()
        };
        let subtle = generate(&ShellParams {
            proj_size: 0.14,
            ..common.clone()
        });
        let bold = generate(&ShellParams {
            proj_size: 0.8,
            ..common.clone()
        });
        assert!(
            subtle.positions.len() < bold.positions.len(),
            "subtle beads should tessellate coarser than bold spikes ({} vs {})",
            subtle.positions.len(),
            bold.positions.len(),
        );
        assert!(subtle.positions.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn projections_cluster_rows_near_beads() {
        // Localised projection refinement: with near-cylindrical geometry (W≈1, so
        // growth-driven grading is negligible), a sharp-spike shell must place rows
        // NON-uniformly — dense near each spike, sparse between — rather than dense
        // across the whole whorl. A smooth shell at W≈1 stays near-uniform (see
        // `grading_degenerates_to_uniform_when_w_near_one`, ratio < 1.5), so a large
        // spacing ratio here is the localisation at work.
        let m = generate(&ShellParams {
            w: 1.05,
            n: 3.0,
            proj_count: 6.0,
            proj_rows: 1.0,
            proj_size: 0.6,
            proj_sharp: 0.9,
            ..ShellParams::default()
        });
        let us = ring_us(&m);
        let deltas: Vec<f32> = us.windows(2).map(|w| w[1] - w[0]).collect();
        let max = deltas.iter().cloned().fold(f32::MIN, f32::max);
        let min = deltas.iter().cloned().fold(f32::MAX, f32::min);
        assert!(
            max / min > 2.0,
            "sharp spikes should cluster rows near beads, got spacing ratio {}",
            max / min
        );
        assert!(m.positions.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn projections_and_varices_each_perturb_and_stay_finite() {
        // Each ornament drives auto-refine to a finer mesh than a smooth shell and
        // stays finite. Projection need is now size-dependent (see the ribs test),
        // so we compare vertex counts against smooth rather than element-wise.
        let smooth = generate(&ShellParams::default());
        let variants = [
            // single-row needle spine
            ShellParams {
                proj_count: 8.0,
                proj_rows: 1.0,
                proj_pos: 1.1,
                proj_size: 0.8,
                proj_sharp: 0.75,
                ..ShellParams::default()
            },
            // multi-row blunt nodules
            ShellParams {
                proj_count: 12.0,
                proj_rows: 2.0,
                proj_size: 0.12,
                proj_sharp: 0.15,
                ..ShellParams::default()
            },
            ShellParams {
                varix_count: 3.0,
                varix_amp: 0.3,
                ..ShellParams::default()
            },
        ];
        for (k, v) in variants.iter().enumerate() {
            let m = generate(v);
            assert!(
                m.positions.len() > smooth.positions.len(),
                "variant {k} should refine the mesh ({} vs {})",
                m.positions.len(),
                smooth.positions.len(),
            );
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

    /// Per-ring `u` values: `u` is constant within a ring (all `cols` vertices
    /// share it) and strictly increasing from ring to ring, so the distinct
    /// values in order are exactly the rows. Length == `theta_verts`.
    fn ring_us(m: &Mesh) -> Vec<f32> {
        let mut us = Vec::new();
        for k in (0..m.uvs.len()).step_by(2) {
            let u = m.uvs[k];
            if us.last().is_none_or(|l: &f32| (u - *l).abs() > 1e-7) {
                us.push(u);
            }
        }
        us
    }

    #[test]
    fn grading_reduces_vertex_count_for_high_w() {
        // No ornament, same whorl count and base resolution. A high-W shell's
        // inner whorls shrink fast toward the apex, so graded θ-tessellation
        // gives them far fewer rows than a low-W shell whose whorls stay large.
        let low = generate(&ShellParams {
            w: 1.3,
            n: 6.0,
            ..ShellParams::default()
        });
        let high = generate(&ShellParams {
            w: 6.0,
            n: 6.0,
            ..ShellParams::default()
        });
        assert!(
            high.positions.len() < low.positions.len(),
            "high-W shell should use fewer vertices ({} vs {})",
            high.positions.len(),
            low.positions.len(),
        );
    }

    #[test]
    fn grading_degenerates_to_uniform_when_w_near_one() {
        // When W≈1 the coil barely grows (g ~ constant), so row density is ~flat
        // and the rows should be near-uniform — the opposite of a strongly graded
        // W=2 shell, whose row-spacing ratio would be ~W^n (≈32).
        let m = generate(&ShellParams {
            w: 1.05,
            n: 5.0,
            ..ShellParams::default()
        });
        let us = ring_us(&m);
        assert!(us.len() > 4);
        let deltas: Vec<f32> = us.windows(2).map(|w| w[1] - w[0]).collect();
        let max = deltas.iter().cloned().fold(f32::MIN, f32::max);
        let min = deltas.iter().cloned().fold(f32::MAX, f32::min);
        assert!(
            max / min < 1.5,
            "W≈1 should give near-uniform rows, got spacing ratio {}",
            max / min
        );
    }

    #[test]
    fn inner_whorl_edge_length_matches_body_whorl() {
        // The whole point of grading: roughly constant arc-length per segment.
        // Arc length ≈ radius·Δθ; radius ∝ g and Δθ ∝ 1/g, so the world-space
        // edge between consecutive rings stays comparable from inner to body
        // whorl. A *uniform* mesh would make the inner edge ~g-times shorter
        // (≈9× for these rings), failing this bound.
        let m = generate(&ShellParams {
            w: 2.0,
            n: 5.0,
            ..ShellParams::default()
        });
        let theta_verts = ring_us(&m).len();
        let cols = (m.positions.len() / 3) / theta_verts;
        let edge = |i: usize| -> f32 {
            let a = i * cols * 3;
            let b = (i + 1) * cols * 3;
            let dx = m.positions[a] - m.positions[b];
            let dy = m.positions[a + 1] - m.positions[b + 1];
            let dz = m.positions[a + 2] - m.positions[b + 2];
            (dx * dx + dy * dy + dz * dz).sqrt()
        };
        let inner = edge(theta_verts * 4 / 10); // ~40% along the coil
        let body = edge(theta_verts - 2); // last segment (body whorl)
        let ratio = inner.max(body) / inner.min(body);
        assert!(
            ratio < 4.0,
            "inner and body edge lengths should be comparable, got ratio {ratio}"
        );
    }

    #[test]
    fn u_uv_is_theta_fraction_monotonic_ring_constant() {
        // The pigment-mapping guarantee: `u` spans [0,1] exactly, increases
        // monotonically, and is identical across every column of a ring (so the
        // pigment texture stays locked to growth-time θ under non-uniform rows).
        let m = generate(&ShellParams {
            w: 3.0,
            n: 4.0,
            ..ShellParams::default()
        });
        let us = ring_us(&m);
        let theta_verts = us.len();
        let cols = (m.positions.len() / 3) / theta_verts;
        assert!(us[0].abs() < 1e-6, "u must start at 0");
        assert!((us[theta_verts - 1] - 1.0).abs() < 1e-6, "u must end at 1");
        for w in us.windows(2) {
            assert!(w[1] > w[0], "u must be strictly increasing per ring");
        }
        for i in 0..theta_verts {
            let base_u = m.uvs[i * cols * 2];
            for j in 0..cols {
                assert!(
                    (m.uvs[(i * cols + j) * 2] - base_u).abs() < 1e-7,
                    "u must be constant across a ring"
                );
            }
        }
    }

    #[test]
    fn min_density_floor_prevents_degenerate_inner_whorls() {
        // Extreme growth: without a minimum density the apex would collapse to a
        // few huge triangles. The floor (MIN_DENSITY_PER_WHORL = 32) keeps the
        // innermost whorl (u < 1/n) angularly smooth — ~32 rows, not the handful
        // that made the apex read as a polygon — and the whole mesh stays finite.
        let m = generate(&ShellParams {
            w: 8.0,
            n: 10.0,
            ..ShellParams::default()
        });
        assert!(m.positions.iter().all(|x| x.is_finite()));
        let us = ring_us(&m);
        let inner_rows = us.iter().filter(|&&u| u < 1.0 / 10.0).count();
        assert!(
            inner_rows >= 24,
            "innermost whorl should stay angularly smooth (~32 rows), got {inner_rows}"
        );
    }

    #[test]
    fn phi_seam_carries_v_one_and_stays_closed() {
        // The φ wrap is a duplicated seam column: each ring ends with an extra
        // vertex coincident with col 0 but carrying v = 1.0, so the closing strip
        // interpolates v up to 1.0 (no texture-smear under wrapT=repeat) while the
        // tube stays geometrically closed and the normals match across the seam.
        let m = generate(&ShellParams {
            w: 2.5,
            n: 4.0,
            rib_sp_count: 7.0, // non-integer-friendly cord count exercises the seam
            rib_sp_amp: 0.2,
            ..ShellParams::default()
        });
        let theta_verts = ring_us(&m).len();
        let stride = (m.positions.len() / 3) / theta_verts;
        for i in 0..theta_verts {
            let first = i * stride; // col 0
            let seam = i * stride + (stride - 1); // duplicate seam column
            assert!(
                m.uvs[first * 2 + 1].abs() < 1e-6,
                "ring {i}: v must start at 0"
            );
            assert!(
                (m.uvs[seam * 2 + 1] - 1.0).abs() < 1e-6,
                "ring {i}: seam v must be exactly 1.0"
            );
            for k in 0..3 {
                assert!(
                    (m.positions[first * 3 + k] - m.positions[seam * 3 + k]).abs() < 1e-6,
                    "ring {i}: seam vertex must coincide with col 0 (closed tube)"
                );
                assert!(
                    (m.normals[first * 3 + k] - m.normals[seam * 3 + k]).abs() < 1e-5,
                    "ring {i}: seam normal must match col 0 (no lighting seam)"
                );
            }
        }
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
        // Defaults are sourced from the tables; confirm every pigment field
        // resolves to its table default through the shared accessor.
        let d = ShellParams::default();
        for r in PIGMENT_RANGES {
            assert_eq!(
                d.field(r.key),
                Some(r.default),
                "{} default mismatch",
                r.key
            );
        }
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
