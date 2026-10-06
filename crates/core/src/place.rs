//! A place: a named area with a centre and a radius (spec: Sources →
//! Places). The first release has one, the user's Home, which sun events and
//! the daylight conditions use. It isn't used for arriving or leaving yet.

use serde::{Deserialize, Serialize};

/// The radius a place has unless it is changed, in metres.
pub const DEFAULT_RADIUS_METRES: u32 = 150;

/// A place with a centre and a radius.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub name: String,
    /// Degrees north of the equator.
    pub latitude: f64,
    /// Degrees east of Greenwich.
    pub longitude: f64,
    pub radius_metres: u32,
}

// Coordinates are checked to be finite numbers, so equality is total.
impl Eq for Place {}

impl Place {
    /// The user's Home, with the default radius.
    pub fn home(latitude: f64, longitude: f64) -> Result<Place, String> {
        if !latitude.is_finite() || !(-90.0..=90.0).contains(&latitude) {
            return Err("latitude is between -90 and 90 degrees".into());
        }
        if !longitude.is_finite() || !(-180.0..=180.0).contains(&longitude) {
            return Err("longitude is between -180 and 180 degrees".into());
        }
        Ok(Place {
            name: "Home".to_string(),
            latitude,
            longitude,
            radius_metres: DEFAULT_RADIUS_METRES,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_has_the_default_radius_and_checks_its_coordinates() {
        let p = Place::home(51.5, -0.12).unwrap();
        assert_eq!(p.name, "Home");
        assert_eq!(p.radius_metres, 150);
        assert!(Place::home(90.0, 180.0).is_ok());
        assert!(Place::home(90.1, 0.0).is_err());
        assert!(Place::home(0.0, -180.5).is_err());
        assert!(Place::home(f64::NAN, 0.0).is_err());
        assert!(Place::home(0.0, f64::INFINITY).is_err());
    }
}
