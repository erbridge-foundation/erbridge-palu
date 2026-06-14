//! Pure jump-range math shared by future jump/bridge features. SDE-sourced:
//! base ranges and the JDC bonus come from the catalog; the only hardcoded
//! value is the light-year, a physical constant rather than a CCP balance lever.
//!
//! Radius queries use **squared metres** to match `kiddo`'s squared-distance
//! API (`SquaredEuclidean`) and avoid a per-candidate `sqrt`.

/// Metres in one light-year. The single hardcoded constant in this module.
pub const LY_IN_METERS: f64 = 9.4607e15;

/// Default Jump Drive Calibration level: maxed (5), the planning assumption for
/// reachability.
pub const DEFAULT_JDC_LEVEL: u8 = 5;

/// A hull's effective jump range in light-years:
/// `base_ly × (1 + bonus_per_level × jdc_level)`.
///
/// `bonus_per_level` is the JDC `jumpDriveRangeBonus` as a fraction (e.g. 0.20),
/// read from the SDE. JDC is the only skill that affects jump range, so this
/// single-bonus formula is universal across all jump-capable hulls.
pub fn effective_ly(base_ly: f64, bonus_per_level: f64, jdc_level: u8) -> f64 {
    base_ly * (1.0 + bonus_per_level * jdc_level as f64)
}

/// `effective_ly` at the default (maxed) JDC level.
pub fn effective_ly_maxed(base_ly: f64, bonus_per_level: f64) -> f64 {
    effective_ly(base_ly, bonus_per_level, DEFAULT_JDC_LEVEL)
}

/// A light-year range as a squared-metre radius: `(ly × LY_IN_METERS)²`. Suited
/// to `kiddo`'s squared-distance radius queries.
pub fn radius_m2(ly: f64) -> f64 {
    let m = ly * LY_IN_METERS;
    m * m
}

/// Distance in light-years between two K-space coordinates (metres). Inverse of
/// `radius_m2` at the boundary: a point exactly `ly` away maps to `radius_m2(ly)`.
pub fn ly_between(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt() / LY_IN_METERS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_range_applies_jdc_bonus() {
        // 4.0 LY base at JDC 5 with +20%/level → 4.0 × (1 + 0.20×5) = 8.0.
        assert_eq!(effective_ly(4.0, 0.20, 5), 8.0);
    }

    #[test]
    fn default_level_is_five() {
        assert_eq!(DEFAULT_JDC_LEVEL, 5);
        // The maxed helper equals an explicit level-5 call.
        assert_eq!(effective_ly_maxed(4.0, 0.20), effective_ly(4.0, 0.20, 5));
        assert_eq!(effective_ly_maxed(4.0, 0.20), 8.0);
    }

    #[test]
    fn radius_and_ly_between_round_trip_at_boundary() {
        // A point exactly `ly` away in metres must map back to `ly`, and its
        // squared distance must equal radius_m2(ly) (same constant both ways).
        let ly = 8.0;
        let origin = [0.0, 0.0, 0.0];
        let far = [ly * LY_IN_METERS, 0.0, 0.0];
        let measured = ly_between(origin, far);
        assert!((measured - ly).abs() < 1e-9, "round-trip ly = {measured}");

        // Squared distance equals the radius (boundary point is on the sphere).
        let dx = far[0] - origin[0];
        let sq_dist = dx * dx;
        assert!((sq_dist - radius_m2(ly)).abs() / radius_m2(ly) < 1e-12);
    }

    #[test]
    fn ly_between_is_symmetric_and_zero_for_same_point() {
        let p = [1.0e16, -2.0e16, 3.0e16];
        assert_eq!(ly_between(p, p), 0.0);
        let q = [4.0e16, 5.0e16, -6.0e16];
        assert_eq!(ly_between(p, q), ly_between(q, p));
    }
}
