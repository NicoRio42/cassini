//! Conversions between Lambert-93 (EPSG:2154) and WGS84 longitudes and
//! latitudes (EPSG:4326).
//!
//! Lambert-93 is a Lambert conformal conic projection of the RGF93 datum,
//! which is treated as WGS84, as `cs2cs +init=epsg:2154 +to +init=epsg:4326`
//! does. The formulas are those of Snyder, "Map Projections: A Working
//! Manual" (1987), pages 107 to 109, as implemented by PROJ.

use std::{
    f64::consts::{FRAC_PI_2, FRAC_PI_4},
    sync::LazyLock,
};

/// The GRS80 ellipsoid.
const SEMI_MAJOR_AXIS: f64 = 6_378_137.0;
const INVERSE_FLATTENING: f64 = 298.257_222_101;

/// The Lambert-93 parameters, angles in degrees.
const STANDARD_PARALLEL_1: f64 = 49.0;
const STANDARD_PARALLEL_2: f64 = 44.0;
const ORIGIN_LATITUDE: f64 = 46.5;
const CENTRAL_MERIDIAN: f64 = 3.0;
const FALSE_EASTING: f64 = 700_000.0;
const FALSE_NORTHING: f64 = 6_600_000.0;

/// The constants of Lambert-93, computed once.
static LAMBERT_93: LazyLock<Lambert93> = LazyLock::new(Lambert93::new);

/// The constants of the projection derived from its parameters.
struct Lambert93 {
    eccentricity: f64,
    /// The cone constant `n`.
    n: f64,
    /// The semi-major axis times Snyder's `F`.
    a_f: f64,
    /// The radius of the origin latitude, `ρ0`.
    rho_0: f64,
}

impl Lambert93 {
    fn new() -> Self {
        let flattening = 1.0 / INVERSE_FLATTENING;
        let eccentricity = (flattening * (2.0 - flattening)).sqrt();
        let phi_1 = STANDARD_PARALLEL_1.to_radians();
        let phi_2 = STANDARD_PARALLEL_2.to_radians();
        let m_1 = msfn(phi_1, eccentricity);
        let m_2 = msfn(phi_2, eccentricity);
        let t_1 = tsfn(phi_1, eccentricity);
        let t_2 = tsfn(phi_2, eccentricity);
        let n = (m_1 / m_2).ln() / (t_1 / t_2).ln();
        let a_f = SEMI_MAJOR_AXIS * m_1 / (n * t_1.powf(n));
        let rho_0 = a_f * tsfn(ORIGIN_LATITUDE.to_radians(), eccentricity).powf(n);
        Self {
            eccentricity,
            n,
            a_f,
            rho_0,
        }
    }

    /// The latitude, in radians, whose `tsfn` is `t`.
    fn latitude(&self, t: f64) -> f64 {
        let half_e = self.eccentricity / 2.0;
        let mut phi = FRAC_PI_2 - 2.0 * t.atan();
        for _ in 0..15 {
            let e_sin = self.eccentricity * phi.sin();
            let next = FRAC_PI_2 - 2.0 * (t * ((1.0 - e_sin) / (1.0 + e_sin)).powf(half_e)).atan();
            let converged = (next - phi).abs() < 1e-14;
            phi = next;
            if converged {
                break;
            }
        }
        phi
    }
}

/// Snyder's `m`, equation 14-15.
fn msfn(phi: f64, eccentricity: f64) -> f64 {
    let e_sin = eccentricity * phi.sin();
    phi.cos() / (1.0 - e_sin * e_sin).sqrt()
}

/// Snyder's `t`, equation 15-9.
fn tsfn(phi: f64, eccentricity: f64) -> f64 {
    let e_sin = eccentricity * phi.sin();
    (FRAC_PI_4 - phi / 2.0).tan() / ((1.0 - e_sin) / (1.0 + e_sin)).powf(eccentricity / 2.0)
}

/// The WGS84 longitude and latitude, in degrees, of the Lambert-93 point
/// (`x`, `y`).
pub fn lambert93_to_wgs84(x: f64, y: f64) -> (f64, f64) {
    let projection = &*LAMBERT_93;
    let dx = x - FALSE_EASTING;
    let dy = projection.rho_0 - (y - FALSE_NORTHING);
    // `n` is positive, as both standard parallels are north.
    let rho = dx.hypot(dy);
    let theta = dx.atan2(dy);
    let t = (rho / projection.a_f).powf(1.0 / projection.n);
    let lon = CENTRAL_MERIDIAN + (theta / projection.n).to_degrees();
    let lat = projection.latitude(t).to_degrees();
    (lon, lat)
}

/// The Lambert-93 coordinates of the WGS84 point at longitude `lon` and
/// latitude `lat`, in degrees.
pub fn wgs84_to_lambert93(lon: f64, lat: f64) -> (f64, f64) {
    let projection = &*LAMBERT_93;
    let rho = projection.a_f * tsfn(lat.to_radians(), projection.eccentricity).powf(projection.n);
    let theta = projection.n * (lon - CENTRAL_MERIDIAN).to_radians();
    let x = FALSE_EASTING + rho * theta.sin();
    let y = FALSE_NORTHING + projection.rho_0 - rho * theta.cos();
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lambert-93 points spread across France and their WGS84 coordinates,
    /// from `cs2cs +init=epsg:2154 +to +init=epsg:4326 -f %.10f` (PROJ 9.7).
    const LAMBERT_93_TO_WGS84: [((f64, f64), (f64, f64)); 7] = [
        ((906_800.0, 6_453_800.0), (5.6320355985, 45.1524733147)),
        ((652_000.0, 6_862_000.0), (2.3458119587, 48.8562481410)),
        ((145_000.0, 6_837_000.0), (-4.5090047924, 48.3956138402)),
        ((704_000.0, 7_059_000.0), (3.0564358961, 50.6279745958)),
        ((1_200_000.0, 6_150_000.0), (9.0568890075, 42.2774427208)),
        ((350_000.0, 6_300_000.0), (-1.3440311531, 43.7123608050)),
        ((1_050_000.0, 6_300_000.0), (7.3440311531, 43.7123608050)),
    ];

    /// WGS84 points and their Lambert-93 coordinates, from
    /// `cs2cs +init=epsg:4326 +to +init=epsg:2154 -f %.6f` (PROJ 9.7).
    const WGS84_TO_LAMBERT_93: [((f64, f64), (f64, f64)); 6] = [
        ((6.95, 45.85), (1_006_407.592446, 6_535_481.733621)),
        ((2.35, 48.86), (652_310.718019, 6_862_414.636762)),
        ((-4.48, 48.39), (147_078.330538, 6_836_175.333849)),
        ((3.06, 50.63), (704_252.441015, 7_059_225.874789)),
        ((9.2, 42.1), (1_213_338.715444, 6_131_231.847052)),
        ((-1.0, 43.5), (376_498.884443, 6_274_963.939290)),
    ];

    #[test]
    fn converts_lambert_93_to_wgs84_like_cs2cs() {
        for ((x, y), (expected_lon, expected_lat)) in LAMBERT_93_TO_WGS84 {
            let (lon, lat) = lambert93_to_wgs84(x, y);
            assert!((lon - expected_lon).abs() < 1e-8, "({x}, {y}): lon {lon}");
            assert!((lat - expected_lat).abs() < 1e-8, "({x}, {y}): lat {lat}");
        }
    }

    #[test]
    fn converts_wgs84_to_lambert_93_like_cs2cs() {
        for ((lon, lat), (expected_x, expected_y)) in WGS84_TO_LAMBERT_93 {
            let (x, y) = wgs84_to_lambert93(lon, lat);
            assert!((x - expected_x).abs() < 1e-5, "({lon}, {lat}): x {x}");
            assert!((y - expected_y).abs() < 1e-5, "({lon}, {lat}): y {y}");
        }
    }

    #[test]
    fn maps_the_origin_to_the_false_origin() {
        let (x, y) = wgs84_to_lambert93(CENTRAL_MERIDIAN, ORIGIN_LATITUDE);
        assert!((x - FALSE_EASTING).abs() < 1e-6 && (y - FALSE_NORTHING).abs() < 1e-6);
    }

    #[test]
    fn round_trips() {
        for ((x, y), _) in LAMBERT_93_TO_WGS84 {
            let (lon, lat) = lambert93_to_wgs84(x, y);
            let (round_x, round_y) = wgs84_to_lambert93(lon, lat);
            assert!((round_x - x).abs() < 1e-6 && (round_y - y).abs() < 1e-6);
        }
    }
}
