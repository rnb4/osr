//! Seeded randomness that stays the same for each thing it decides.

use fastrand::Rng;

/// A random source for one kind of decision about one thing, the same on every run with the same seed.
///
/// `tag` keeps unrelated decisions about the same thing apart, and `key` identifies the thing, such as a hex's
/// [`Axial::key`](crate::hex::Axial::key). Seeding per thing means changing one part of a map does not reshuffle the
/// rest.
pub fn stable_rng(seed: u64, tag: u64, key: u64) -> Rng {
    Rng::with_seed(seed ^ tag.wrapping_mul(0xc2b2_ae3d_27d4_eb4f) ^ key.wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

#[cfg(test)]
mod tests {
    use super::stable_rng;

    #[test]
    fn same_inputs_same_numbers() {
        let draw = |seed, tag, key| stable_rng(seed, tag, key).u64(..);
        assert_eq!(draw(7, 1, 42), draw(7, 1, 42));
        assert_ne!(draw(7, 1, 42), draw(8, 1, 42));
        assert_ne!(draw(7, 1, 42), draw(7, 2, 42));
        assert_ne!(draw(7, 1, 42), draw(7, 1, 43));
    }
}
