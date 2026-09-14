//! Property tests for the arithmetic everything else stands on.

use dimetric_core::{Angle, Fx, FxWide, Rng, Vec2Fx};
use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence};

fn any_fx() -> impl Strategy<Value = Fx> {
    any::<i32>().prop_map(Fx::from_raw)
}

/// Values small enough that products stay inside `Fx`, for the laws that only
/// hold in the absence of saturation.
fn small_fx() -> impl Strategy<Value = Fx> {
    (-180i32..180).prop_flat_map(|i| {
        (0i32..65536).prop_map(move |f| Fx::from_raw(i.wrapping_mul(65536).wrapping_add(f)))
    })
}

proptest! {
    // Keep the seed that found a failure. Without somewhere to write it, a
    // property test that fails once goes green on the next run and the case is
    // gone — the default location is derived from a `src/` layout these tests
    // do not have, so proptest was silently discarding them.
    #![proptest_config(Config {
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
            "proptest-regressions",
        ))),
        ..Config::default()
    })]

    #[test]
    fn decimal_round_trip_is_exact(v in any_fx()) {
        let text = v.to_exact_string();
        prop_assert_eq!(Fx::parse_exact(&text).unwrap(), v);
    }

    #[test]
    fn wide_decimal_round_trip_is_exact(raw in any::<i64>()) {
        let v = FxWide::from_raw(raw);
        prop_assert_eq!(FxWide::parse_exact(&v.to_exact_string()).unwrap(), v);
    }

    #[test]
    fn addition_is_commutative(a in small_fx(), b in small_fx()) {
        prop_assert_eq!(a + b, b + a);
    }

    #[test]
    fn addition_is_associative_when_it_does_not_saturate(
        a in small_fx(), b in small_fx(), c in small_fx()
    ) {
        // Associativity is the law fixed-point keeps and floating point does
        // not: there is no rounding step between operations, so as long as no
        // intermediate leaves range the grouping cannot matter.
        prop_assert_eq!((a + b) + c, a + (b + c));
    }

    #[test]
    fn subtraction_inverts_addition(a in small_fx(), b in small_fx()) {
        prop_assert_eq!((a + b) - b, a);
    }

    #[test]
    fn multiplication_is_commutative(a in small_fx(), b in small_fx()) {
        prop_assert_eq!(a * b, b * a);
    }

    #[test]
    fn widening_never_loses_anything(v in any_fx()) {
        prop_assert_eq!(v.wide().narrow().unwrap(), v);
    }

    #[test]
    fn ordering_matches_the_decimal_text(a in small_fx(), b in small_fx()) {
        let by_value = a.cmp(&b);
        let by_number = a.to_f64().partial_cmp(&b.to_f64()).unwrap();
        prop_assert_eq!(by_value, by_number);
    }

    #[test]
    fn sqrt_brackets_the_true_root(raw in 0i64..(1i64 << 46)) {
        // Checked in exact integer arithmetic rather than by squaring the
        // result in fixed point. A fixed-point square truncates, and the
        // truncation can turn a product that genuinely exceeds the input into
        // one that merely equals it — the property would then look false for a
        // correct root. Raw `r` is `floor(sqrt(v) * 2^16)`, so the real claim
        // is about `r^2` against `v << 16`.
        let v = FxWide::from_raw(raw);
        let root = v.sqrt().to_raw() as i128;
        let scaled = (raw as i128) << 16;
        prop_assert!(root * root <= scaled, "{root}^2 > {scaled}");
        prop_assert!((root + 1) * (root + 1) > scaled, "({root} + 1)^2 <= {scaled}");
    }

    #[test]
    fn pythagoras_holds_for_the_trig_tables(bam in any::<u16>()) {
        let a = Angle::from_bam(bam);
        let (s, c) = a.sin_cos();
        let unit = s.wide() * s.wide() + c.wide() * c.wide();
        // One unit of 1/65536 either way, doubled for the two squarings, plus
        // the interpolation error between table entries.
        let err = (unit - FxWide::ONE).abs();
        prop_assert!(err < FxWide::from_raw(8), "sin^2+cos^2 off by {err} at {bam}");
    }

    #[test]
    fn a_direction_survives_a_trip_through_an_angle(bam in any::<u16>()) {
        let a = Angle::from_bam(bam);
        let back = a.to_unit_vector().to_angle();
        let drift = a.delta_to(back).abs();
        prop_assert!(drift <= 24, "angle drifted by {drift} units at {bam}");
    }

    #[test]
    fn normalizing_produces_a_unit_vector(x in -3000i32..3000, y in -3000i32..3000) {
        let v = Vec2Fx::from_ints(x, y);
        let n = v.normalized();
        if v.is_zero() {
            prop_assert!(n.is_zero());
        } else {
            let drift = (n.length() - Fx::ONE).abs();
            prop_assert!(drift < Fx::from_raw(64), "length off by {} for {:?}", drift, v);
        }
    }

    #[test]
    fn sliding_never_moves_into_the_surface(
        mx in -500i32..500, my in -500i32..500, bam in any::<u16>()
    ) {
        let motion = Vec2Fx::from_ints(mx, my);
        let normal = Angle::from_bam(bam).to_unit_vector();
        let slid = motion.slid_along(normal);
        // A small tolerance: the normal is itself only unit-length to within
        // the resolution of the trig tables.
        prop_assert!(slid.dot(normal) >= FxWide::from_raw(-(1 << 12)));
    }

    #[test]
    fn rng_range_stays_in_bounds(seed in any::<u64>(), lo in -1000i32..1000, span in 1i32..5000) {
        let mut rng = Rng::new(seed, 0);
        for _ in 0..64 {
            let v = rng.range_i32(lo, lo + span);
            prop_assert!(v >= lo && v < lo + span);
        }
    }

    #[test]
    fn rng_snapshot_restores_the_exact_sequence(seed in any::<u64>()) {
        let mut rng = Rng::new(seed, 7);
        for _ in 0..17 { rng.next_u32(); }
        let saved = rng.snapshot();
        let expected: Vec<u32> = (0..32).map(|_| rng.next_u32()).collect();
        let mut restored = Rng::restore(saved);
        let actual: Vec<u32> = (0..32).map(|_| restored.next_u32()).collect();
        prop_assert_eq!(expected, actual);
    }
}

#[test]
fn pcg32_matches_the_reference_implementation() {
    // Output vector from the canonical pcg32-demo with initstate=42,
    // initseq=54. If this ever changes, every recorded replay is invalid.
    let mut rng = Rng::new(42, 54);
    let expected = [
        0xa15c02b7u32,
        0x7b47f409,
        0xba1d3330,
        0x83d2f293,
        0xbfa4784b,
        0xcbed606e,
    ];
    for (i, want) in expected.iter().enumerate() {
        assert_eq!(rng.next_u32(), *want, "draw {i}");
    }
}

#[test]
fn known_trig_values() {
    assert_eq!(Angle::ZERO.sin(), Fx::ZERO);
    assert_eq!(Angle::ZERO.cos(), Fx::ONE);
    assert_eq!(Angle::QUARTER_TURN.sin(), Fx::ONE);
    assert_eq!(Angle::QUARTER_TURN.cos(), Fx::ZERO);
    assert_eq!(Angle::HALF_TURN.sin(), Fx::ZERO);
    assert_eq!(Angle::HALF_TURN.cos(), Fx::NEG_ONE);
    assert_eq!(Angle::from_bam(49152).sin(), Fx::NEG_ONE);
}

#[test]
fn degrees_round_trip_where_the_angle_is_representable() {
    for deg in ["0.0", "45.0", "90.0", "180.0", "270.0", "315.0"] {
        let a = Angle::from_degrees_str(deg).unwrap();
        assert_eq!(a.to_degrees_string(), deg, "{deg}");
    }
}

#[test]
fn degrees_that_are_not_representable_round_and_say_so_in_canonical_form() {
    // 30 degrees falls between two binary units. The value rounds to the
    // nearest, and canonical form writes back what is actually stored, so the
    // loss appears in a formatting diff instead of hiding in the file.
    let a = Angle::from_degrees_str("30.0").unwrap();
    assert_eq!(a.to_bam(), 5461);
    assert_eq!(a.to_degrees_string(), "29.9981689453125");
    assert_eq!(Angle::from_degrees_str("29.9981689453125").unwrap(), a);
}

#[test]
fn angles_wrap_without_special_cases() {
    let a = Angle::from_bam(60000);
    assert_eq!((a + Angle::from_bam(10000)).to_bam(), 4464);
    assert_eq!(Angle::ZERO.delta_to(Angle::from_bam(65000)), -536);
    assert_eq!(
        Angle::ZERO
            .rotate_toward(Angle::from_bam(65000), 100)
            .to_bam(),
        65436
    );
}

#[test]
fn shuffle_is_a_permutation_and_depends_only_on_the_seed() {
    let mut a: Vec<u32> = (0..64).collect();
    let mut b = a.clone();
    Rng::new(9, 1).shuffle(&mut a);
    Rng::new(9, 1).shuffle(&mut b);
    assert_eq!(a, b);
    let mut sorted = a.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..64).collect::<Vec<_>>());
    assert_ne!(
        a, sorted,
        "a seeded shuffle of 64 items should reorder them"
    );
}

#[test]
fn sqrt_of_a_value_whose_next_root_squares_to_itself_after_truncation() {
    // A case the property test found. `(r + 1)^2` really is greater than the
    // input, by about six tenths of a raw unit — which a fixed-point square
    // truncates away, so squaring in `Fx` reports equality. The root is right;
    // measuring it that way was not.
    let v = FxWide::from_raw(13_114_317_989_164);
    let root = v.sqrt().to_raw() as i128;
    let scaled = 13_114_317_989_164i128 << 16;

    assert!(root * root <= scaled);
    assert!((root + 1) * (root + 1) > scaled);
    // And the lossy version, for the record: the product truncates to exactly
    // the input rather than past it.
    let up = Fx::from_raw(root as i32 + 1);
    assert_eq!(up.wide() * up.wide(), v);
}
