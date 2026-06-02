//! Shared math and seeded-noise primitives.
//!
//! Crate-internal helpers used by both the geometry sweep ([`crate::geometry`])
//! and the pigmentation engine ([`crate::pigment`]): rib/lobe profiles, a fast
//! integer hash, reproducible value noise, and small interpolation utilities.

/// Rib / wave profile in `[-1, 1]`. `sharp` morphs a smooth cosine **wave** (0)
/// into a narrow knife-edge **ridge** (1) by peaking a raised cosine.
#[inline]
pub(crate) fn ribbed(x: f32, sharp: f32) -> f32 {
    let c = 0.5 * (x.cos() + 1.0); // raised cosine, 0..1
    let p = 1.0 + sharp.clamp(0.0, 1.0) * 8.0; // exponent narrows the peak
    2.0 * c.powf(p) - 1.0
}

/// Positive periodic lobe in `[0, 1]`, peaking at multiples of 2π. `power`
/// narrows it: low = broad bump (nodule / varix), high = narrow spike (needle).
#[inline]
pub(crate) fn lobe(x: f32, power: f32) -> f32 {
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
pub(crate) fn rand01(seed: u32, i: i32) -> f32 {
    let h = hash_u32(seed ^ hash_u32(i as u32));
    (h >> 8) as f32 / ((1u32 << 24) as f32)
}

/// Hash `(seed, i)` → f32 in `[-1, 1)`.
#[inline]
pub(crate) fn rand_signed(seed: u32, i: i32) -> f32 {
    rand01(seed, i) * 2.0 - 1.0
}

/// Smooth 1-D value noise in `[-1, 1]` (lattice values, smoothstep-interpolated).
#[inline]
pub(crate) fn noise1(seed: u32, x: f32) -> f32 {
    let i = x.floor();
    let f = x - i;
    let ii = i as i32;
    let a = rand01(seed, ii);
    let b = rand01(seed, ii + 1);
    let u = f * f * (3.0 - 2.0 * f);
    (a + (b - a) * u) * 2.0 - 1.0
}

/// Linear interpolation with a clamped parameter.
#[inline]
pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// Hermite smoothstep in `[0, 1]`.
#[inline]
pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
