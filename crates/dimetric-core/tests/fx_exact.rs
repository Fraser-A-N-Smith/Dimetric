use dimetric_core::fx::{Fx, FxParseError, FxWide};

#[test]
fn every_representable_value_round_trips_through_decimal() {
    // Exhaustive over the fractional range at several integer offsets: if any
    // Fx failed to render as an exact terminating decimal, this finds it.
    for int_part in [-32768i32, -1, 0, 1, 12345, 32767] {
        for frac in 0..65536i32 {
            let raw = int_part.wrapping_mul(65536).wrapping_add(frac);
            let v = Fx::from_raw(raw);
            let text = v.to_exact_string();
            let back = Fx::parse_exact(&text).expect("exact text must parse");
            assert_eq!(v, back, "round trip failed for raw {raw} rendered as {text}");
        }
    }
}

#[test]
fn smallest_step_renders_in_full() {
    assert_eq!(Fx::DELTA.to_exact_string(), "0.0000152587890625");
    assert_eq!(Fx::from_raw(-1).to_exact_string(), "-0.0000152587890625");
}

#[test]
fn whole_numbers_keep_a_decimal_point() {
    // Without this, TOML would type the value as an integer, not a scalar.
    assert_eq!(Fx::from_int(48).to_exact_string(), "48.0");
    assert_eq!(Fx::ZERO.to_exact_string(), "0.0");
    assert_eq!(Fx::from_int(-7).to_exact_string(), "-7.0");
}

#[test]
fn inexact_input_is_refused_rather_than_rounded() {
    let err = Fx::parse_exact("0.1").unwrap_err();
    assert!(matches!(err, FxParseError::NotRepresentable(_)), "{err:?}");
    assert!(Fx::parse_exact("0.5").is_ok());
    assert!(Fx::parse_exact("0.25").is_ok());
    assert!(Fx::parse_exact("0.00001").is_err());
}

#[test]
fn exponent_notation_is_refused() {
    assert!(matches!(
        Fx::parse_exact("1e3"),
        Err(FxParseError::UnsupportedForm(_))
    ));
}

#[test]
fn wide_holds_the_square_of_any_fx() {
    let far = Fx::from_int(32767);
    let sq = far.wide() * far.wide();
    assert!(sq.narrow().is_err(), "the square must not fit back into Fx");
    assert_eq!(sq.sqrt(), far, "sqrt of a perfect square is exact");
}

#[test]
fn sqrt_is_exact_on_squares_and_monotonic() {
    for n in [0i32, 1, 2, 3, 16, 100, 1000, 32767] {
        let v = Fx::from_int(n);
        assert_eq!((v.wide() * v.wide()).sqrt(), v, "sqrt({n}^2)");
    }
    let mut prev = Fx::ZERO;
    for raw in (0..1_000_000i64).step_by(9973) {
        let r = FxWide::from_raw(raw).sqrt();
        assert!(r >= prev);
        prev = r;
    }
}

#[test]
fn saturating_operators_agree_in_every_build_profile() {
    // The explicit methods never assert, so they are safe to test in debug.
    assert_eq!(Fx::MAX.saturating_add(Fx::ONE), Fx::MAX);
    assert_eq!(Fx::MIN.saturating_sub(Fx::ONE), Fx::MIN);
    assert_eq!(Fx::MAX.saturating_mul(Fx::from_int(2)), Fx::MAX);
    assert_eq!(Fx::ONE.saturating_div(Fx::ZERO), Fx::MAX);
    assert_eq!(Fx::NEG_ONE.saturating_div(Fx::ZERO), Fx::MIN);
    assert_eq!(Fx::MAX.checked_add(Fx::ONE), None);
}

#[test]
fn rounding_is_symmetric_about_zero() {
    assert_eq!(Fx::parse_exact("2.5").unwrap().round_int(), 3);
    assert_eq!(Fx::parse_exact("-2.5").unwrap().round_int(), -3);
    assert_eq!(Fx::parse_exact("3.5").unwrap().round_int(), 4);
    assert_eq!(Fx::parse_exact("-3.5").unwrap().round_int(), -4);
    assert_eq!(Fx::parse_exact("2.25").unwrap().round_int(), 2);
    assert_eq!(Fx::parse_exact("-2.25").unwrap().round_int(), -2);
}
