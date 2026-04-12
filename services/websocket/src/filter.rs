//! Per-client subscription filter.
//!
//! Filters are set at connection time via URL query parameters and can be
//! updated after connection via a JSON `"subscribe"` control message.
//!
//! ## Query parameter format
//!
//! ```
//! ws://host:9093/?min_magnitude=4.0&lat_min=36.0&lat_max=42.0&lon_min=26.0&lon_max=45.0&source_networks=USGS,EMSC
//! ```
//!
//! All parameters are optional.  Unrecognised or malformed parameters are
//! silently ignored; the remaining valid parameters are applied.
//!
//! ## JSON update format
//!
//! ```json
//! {
//!   "type": "subscribe",
//!   "min_magnitude": 4.0,
//!   "lat_min": 36.0, "lat_max": 42.0,
//!   "lon_min": 26.0, "lon_max": 45.0,
//!   "source_networks": ["USGS", "EMSC"]
//! }
//! ```
//!
//! Omitted fields reset to their defaults (no restriction).

use crate::event::{AlertEvent, EnrichedEvent};

/// Subscription filter applied to every broadcast event for a single client.
#[derive(Debug, Clone)]
pub struct SubscriptionFilter {
    /// Minimum `ml_magnitude` required to pass. Default: 0.0 (all events).
    pub min_magnitude: f64,

    /// Southern latitude boundary (inclusive). None = no restriction.
    pub lat_min: Option<f64>,
    /// Northern latitude boundary (inclusive). None = no restriction.
    pub lat_max: Option<f64>,
    /// Western longitude boundary (inclusive). None = no restriction.
    pub lon_min: Option<f64>,
    /// Eastern longitude boundary (inclusive). None = no restriction.
    pub lon_max: Option<f64>,

    /// Allowed source networks (upper-cased). Empty = all networks.
    pub source_networks: Vec<String>,
}

impl SubscriptionFilter {
    /// Parse a filter from URL query parameters, using `default_min_magnitude`
    /// when `min_magnitude` is not present in the query string.
    pub fn from_query(query: &str, default_min_magnitude: f64) -> Self {
        let mut filter = Self {
            min_magnitude: default_min_magnitude,
            lat_min: None,
            lat_max: None,
            lon_min: None,
            lon_max: None,
            source_networks: Vec::new(),
        };

        for pair in query.split('&') {
            let mut kv = pair.splitn(2, '=');
            let key = match kv.next() {
                Some(k) if !k.is_empty() => k.trim(),
                _ => continue,
            };
            let val = match kv.next() {
                Some(v) if !v.is_empty() => v.trim(),
                _ => continue,
            };

            match key {
                "min_magnitude" => {
                    if let Ok(v) = val.parse::<f64>() {
                        if v.is_finite() {
                            filter.min_magnitude = v;
                        }
                    }
                }
                "lat_min" => {
                    if let Ok(v) = val.parse::<f64>() {
                        filter.lat_min = Some(v);
                    }
                }
                "lat_max" => {
                    if let Ok(v) = val.parse::<f64>() {
                        filter.lat_max = Some(v);
                    }
                }
                "lon_min" => {
                    if let Ok(v) = val.parse::<f64>() {
                        filter.lon_min = Some(v);
                    }
                }
                "lon_max" => {
                    if let Ok(v) = val.parse::<f64>() {
                        filter.lon_max = Some(v);
                    }
                }
                "source_networks" => {
                    filter.source_networks = val
                        .split(',')
                        .map(|s| s.trim().to_uppercase())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                _ => {} // silently ignore unknown parameters
            }
        }

        filter
    }

    /// Returns `true` if an enriched event passes this filter.
    pub fn matches_enriched(&self, event: &EnrichedEvent) -> bool {
        if event.ml_magnitude < self.min_magnitude {
            return false;
        }
        if !self.within_bbox(event.latitude, event.longitude) {
            return false;
        }
        if !self.source_networks.is_empty() {
            let net = event.source_network.to_uppercase();
            if !self.source_networks.iter().any(|n| n == &net) {
                return false;
            }
        }
        true
    }

    /// Returns `true` if an alert event passes this filter.
    ///
    /// Alert events carry no `source_network` field, so only magnitude and
    /// bounding-box filters are applied.
    pub fn matches_alert(&self, event: &AlertEvent) -> bool {
        if event.ml_magnitude < self.min_magnitude {
            return false;
        }
        if !self.within_bbox(event.latitude, event.longitude) {
            return false;
        }
        true
    }

    fn within_bbox(&self, lat: f64, lon: f64) -> bool {
        if let Some(min) = self.lat_min {
            if lat < min {
                return false;
            }
        }
        if let Some(max) = self.lat_max {
            if lat > max {
                return false;
            }
        }
        if let Some(min) = self.lon_min {
            if lon < min {
                return false;
            }
        }
        if let Some(max) = self.lon_max {
            if lon > max {
                return false;
            }
        }
        true
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn enriched(ml: f64, lat: f64, lon: f64, net: &str) -> EnrichedEvent {
        EnrichedEvent {
            source_id: "test".into(),
            source_network: net.into(),
            event_time_ms: 0,
            latitude: lat,
            longitude: lon,
            depth_km: Some(10.0),
            magnitude: ml,
            magnitude_type: "ML".into(),
            region_name: None,
            quality_indicator: "A".into(),
            raw_payload: "{}".into(),
            ingested_at_ms: 0,
            pipeline_version: "0.1.0".into(),
            ml_magnitude: ml,
            ml_magnitude_source: "ml_depth_corrected".into(),
            is_aftershock: false,
            mainshock_source_id: None,
            mainshock_magnitude: None,
            mainshock_distance_km: None,
            mainshock_time_delta_hours: None,
            estimated_felt_radius_km: 10.0,
            estimated_intensity_mmi: 4.0,
            enriched_at_ms: 0,
            analysis_version: "0.1.0".into(),
        }
    }

    fn alert(ml: f64, lat: f64, lon: f64) -> AlertEvent {
        AlertEvent {
            source_id: "test".into(),
            event_time_ms: 0,
            latitude: lat,
            longitude: lon,
            depth_km: Some(10.0),
            magnitude: ml,
            ml_magnitude: ml,
            region_name: None,
            estimated_intensity_mmi: 6.0,
            estimated_felt_radius_km: 50.0,
            is_aftershock: false,
            alert_level: "YELLOW".into(),
            triggered_at_ms: 0,
        }
    }

    #[test]
    fn empty_filter_matches_everything() {
        let f = SubscriptionFilter::from_query("", 0.0);
        assert!(f.matches_enriched(&enriched(0.1, 0.0, 0.0, "USGS")));
        assert!(f.matches_enriched(&enriched(9.0, 90.0, 180.0, "EMSC")));
        assert!(f.matches_alert(&alert(5.0, 40.0, 30.0)));
    }

    #[test]
    fn magnitude_filter_rejects_below_threshold() {
        let f = SubscriptionFilter::from_query("min_magnitude=4.0", 0.0);
        assert!(!f.matches_enriched(&enriched(3.9, 0.0, 0.0, "USGS")));
        assert!(f.matches_enriched(&enriched(4.0, 0.0, 0.0, "USGS")));
        assert!(f.matches_enriched(&enriched(4.1, 0.0, 0.0, "USGS")));
    }

    #[test]
    fn bbox_filter_rejects_outside() {
        let f = SubscriptionFilter::from_query(
            "lat_min=36.0&lat_max=42.0&lon_min=26.0&lon_max=45.0",
            0.0,
        );
        // Inside
        assert!(f.matches_enriched(&enriched(4.0, 39.0, 35.0, "USGS")));
        // Outside latitude
        assert!(!f.matches_enriched(&enriched(4.0, 35.9, 35.0, "USGS")));
        assert!(!f.matches_enriched(&enriched(4.0, 42.1, 35.0, "USGS")));
        // Outside longitude
        assert!(!f.matches_enriched(&enriched(4.0, 39.0, 25.9, "USGS")));
        assert!(!f.matches_enriched(&enriched(4.0, 39.0, 45.1, "USGS")));
    }

    #[test]
    fn source_network_filter() {
        let f = SubscriptionFilter::from_query("source_networks=USGS,EMSC", 0.0);
        assert!(f.matches_enriched(&enriched(4.0, 0.0, 0.0, "USGS")));
        assert!(f.matches_enriched(&enriched(4.0, 0.0, 0.0, "emsc"))); // case-insensitive
        assert!(!f.matches_enriched(&enriched(4.0, 0.0, 0.0, "AFAD")));
    }

    #[test]
    fn alert_filter_ignores_source_network() {
        let f = SubscriptionFilter::from_query("source_networks=USGS", 0.0);
        // Alerts have no network field — the network filter must not apply.
        assert!(f.matches_alert(&alert(5.0, 39.0, 35.0)));
    }

    #[test]
    fn from_query_uses_default_min_magnitude_when_absent() {
        let f = SubscriptionFilter::from_query("lat_min=36.0", 3.0);
        assert_eq!(f.min_magnitude, 3.0);
    }

    #[test]
    fn from_query_overrides_default_when_present() {
        let f = SubscriptionFilter::from_query("min_magnitude=5.0", 3.0);
        assert_eq!(f.min_magnitude, 5.0);
    }

    #[test]
    fn malformed_params_are_ignored() {
        let f = SubscriptionFilter::from_query("min_magnitude=abc&lat_min=notanumber", 0.0);
        assert_eq!(f.min_magnitude, 0.0);
        assert!(f.lat_min.is_none());
    }
}
