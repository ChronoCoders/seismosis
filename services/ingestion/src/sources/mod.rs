pub mod afad;
pub mod emsc;
pub mod usgs;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{error::ParseError, schema::RawEarthquakeEvent};

/// All seismic data sources implement this trait.
///
/// `fetch` is called on every poll cycle with the earliest event time to
/// request. The lookback window (`since = now - lookback`) ensures late-
/// arriving events are captured even if an earlier poll missed them.
/// The deduplication layer absorbs any resulting duplicates.
#[async_trait]
pub trait SeismicSource: Send + Sync {
    /// Short identifier used in logs and metric label values.
    fn name(&self) -> &'static str;

    /// Fetch all events with `event_time >= since`.
    ///
    /// Returns only successfully parsed events. Individual parse failures are
    /// logged internally and counted in the `events_rejected_total` metric —
    /// they do not abort the call.
    async fn fetch(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<RawEarthquakeEvent>, crate::error::IngestError>;
}

// ─── Shared parsing helpers ───────────────────────────────────────────────────

/// Normalise a magnitude type string to uppercase.
///
/// Sources are inconsistent: USGS uses "mb", "ml"; EMSC uses "ML", "mb".
/// We always store uppercase so downstream comparisons are deterministic.
pub fn normalise_mag_type(raw: &str) -> String {
    raw.trim().to_uppercase()
}

/// Map a USGS event status string to a quality indicator character.
///
/// USGS statuses: "reviewed" | "automatic" | (other)
pub fn usgs_quality(status: Option<&str>) -> &'static str {
    match status {
        Some("reviewed") => "A",
        Some("automatic") => "D",
        _ => "D",
    }
}

/// Map an EMSC event type string to a quality indicator character.
///
/// EMSC `evtype`: "ke" = known earthquake (preliminary review), other = unreviewed.
pub fn emsc_quality(evtype: Option<&str>) -> &'static str {
    match evtype {
        Some("ke") => "C",
        _ => "D",
    }
}

/// Map an AFAD event type string to a quality indicator character.
///
/// AFAD `type`: "Ke" = Kesinleşmiş (manually confirmed). Other values are
/// automatic or preliminary solutions.
pub fn afad_quality(event_type: Option<&str>) -> &'static str {
    match event_type {
        Some("Ke") => "B",
        _ => "D",
    }
}

/// Validate latitude and longitude are within WGS-84 bounds.
///
/// Explicit `is_finite()` guards are checked before the range comparisons.
/// `NaN` and `±Inf` satisfy `!range.contains(&v)` and would be rejected by the
/// range check alone, but the explicit guard produces a clearer error message
/// and guards against future refactors that might reorder the checks.
pub fn validate_coordinates(
    lat: f64,
    lon: f64,
    src: &'static str,
    event_id: &str,
) -> Result<(), ParseError> {
    if !lat.is_finite() {
        return Err(ParseError::InvalidField {
            field: "latitude",
            src,
            event_id: event_id.to_owned(),
            detail: format!("{} is not a finite number", lat),
        });
    }
    if !lon.is_finite() {
        return Err(ParseError::InvalidField {
            field: "longitude",
            src,
            event_id: event_id.to_owned(),
            detail: format!("{} is not a finite number", lon),
        });
    }
    if !(-90.0..=90.0).contains(&lat) {
        return Err(ParseError::InvalidField {
            field: "latitude",
            src,
            event_id: event_id.to_owned(),
            detail: format!("{} is outside [-90, 90]", lat),
        });
    }
    if !(-180.0..=180.0).contains(&lon) {
        return Err(ParseError::InvalidField {
            field: "longitude",
            src,
            event_id: event_id.to_owned(),
            detail: format!("{} is outside [-180, 180]", lon),
        });
    }
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // ── normalise_mag_type ────────────────────────────────────────────────

    #[test]
    fn normalise_mag_type_uppercases_lowercase() {
        assert_eq!(normalise_mag_type("ml"), "ML");
        assert_eq!(normalise_mag_type("mb"), "MB");
        assert_eq!(normalise_mag_type("mw"), "MW");
    }

    #[test]
    fn normalise_mag_type_trims_whitespace() {
        assert_eq!(normalise_mag_type("  ml  "), "ML");
    }

    #[test]
    fn normalise_mag_type_already_uppercase_unchanged() {
        assert_eq!(normalise_mag_type("ML"), "ML");
        assert_eq!(normalise_mag_type("UNKNOWN"), "UNKNOWN");
    }

    // ── usgs_quality ─────────────────────────────────────────────────────

    #[test]
    fn usgs_quality_reviewed_is_a() {
        assert_eq!(usgs_quality(Some("reviewed")), "A");
    }

    #[test]
    fn usgs_quality_automatic_is_d() {
        assert_eq!(usgs_quality(Some("automatic")), "D");
    }

    #[test]
    fn usgs_quality_none_is_d() {
        assert_eq!(usgs_quality(None), "D");
    }

    #[test]
    fn usgs_quality_unknown_string_is_d() {
        assert_eq!(usgs_quality(Some("preliminary")), "D");
    }

    // ── emsc_quality ─────────────────────────────────────────────────────

    #[test]
    fn emsc_quality_ke_is_c() {
        assert_eq!(emsc_quality(Some("ke")), "C");
    }

    #[test]
    fn emsc_quality_other_is_d() {
        assert_eq!(emsc_quality(Some("se")), "D");
        assert_eq!(emsc_quality(None), "D");
    }

    // ── validate_coordinates ──────────────────────────────────────────────

    #[test]
    fn valid_coordinates_accepted() {
        assert!(validate_coordinates(0.0, 0.0, "T", "e").is_ok());
        assert!(validate_coordinates(90.0, 180.0, "T", "e").is_ok());
        assert!(validate_coordinates(-90.0, -180.0, "T", "e").is_ok());
        assert!(validate_coordinates(37.5, -122.1, "T", "e").is_ok());
    }

    #[test]
    fn lat_too_high_rejected() {
        let err = validate_coordinates(90.001, 0.0, "T", "e").unwrap_err();
        assert!(err.to_string().contains("latitude"));
    }

    #[test]
    fn lat_too_low_rejected() {
        let err = validate_coordinates(-90.001, 0.0, "T", "e").unwrap_err();
        assert!(err.to_string().contains("latitude"));
    }

    #[test]
    fn lon_too_high_rejected() {
        let err = validate_coordinates(0.0, 180.001, "T", "e").unwrap_err();
        assert!(err.to_string().contains("longitude"));
    }

    #[test]
    fn lon_too_low_rejected() {
        let err = validate_coordinates(0.0, -180.001, "T", "e").unwrap_err();
        assert!(err.to_string().contains("longitude"));
    }

    #[test]
    fn nan_lat_rejected() {
        assert!(validate_coordinates(f64::NAN, 0.0, "T", "e").is_err());
    }

    #[test]
    fn inf_lat_rejected() {
        assert!(validate_coordinates(f64::INFINITY, 0.0, "T", "e").is_err());
        assert!(validate_coordinates(f64::NEG_INFINITY, 0.0, "T", "e").is_err());
    }

    #[test]
    fn nan_lon_rejected() {
        assert!(validate_coordinates(0.0, f64::NAN, "T", "e").is_err());
    }

    #[test]
    fn inf_lon_rejected() {
        assert!(validate_coordinates(0.0, f64::INFINITY, "T", "e").is_err());
    }

    // ── proptest: coordinate bounds ────────────────────────────────────────

    proptest! {
        #[test]
        fn prop_valid_coordinates_accepted(
            lat in -90.0f64..=90.0f64,
            lon in -180.0f64..=180.0f64,
        ) {
            prop_assume!(lat.is_finite() && lon.is_finite());
            prop_assert!(validate_coordinates(lat, lon, "T", "e").is_ok());
        }

        #[test]
        fn prop_lat_above_90_rejected(extra in 0.001f64..900.0f64) {
            prop_assert!(validate_coordinates(90.0 + extra, 0.0, "T", "e").is_err());
        }

        #[test]
        fn prop_lat_below_neg90_rejected(extra in 0.001f64..900.0f64) {
            prop_assert!(validate_coordinates(-90.0 - extra, 0.0, "T", "e").is_err());
        }

        #[test]
        fn prop_lon_above_180_rejected(extra in 0.001f64..900.0f64) {
            prop_assert!(validate_coordinates(0.0, 180.0 + extra, "T", "e").is_err());
        }

        #[test]
        fn prop_lon_below_neg180_rejected(extra in 0.001f64..900.0f64) {
            prop_assert!(validate_coordinates(0.0, -180.0 - extra, "T", "e").is_err());
        }
    }
}
