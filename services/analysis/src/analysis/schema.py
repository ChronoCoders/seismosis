"""
Avro schemas and Schema Registry subjects for the analysis service.

The enriched schema carries all fields from earthquakes.raw-value plus
analysis-derived enrichment.  The alert schema is a compact subset for
the earthquakes.alerts topic.
"""

ENRICHED_SUBJECT = "earthquakes.enriched-value"
ALERT_SUBJECT = "earthquakes.alerts-value"

ENRICHED_SCHEMA: dict = {
    "type": "record",
    "name": "EnrichedEarthquakeEvent",
    "namespace": "com.seismosis",
    "doc": (
        "Seismic event enriched with ML magnitude refinement, "
        "ETAS aftershock classification, and risk estimates."
    ),
    "fields": [
        # ── Carried through from raw ─────────────────────────────────────
        {"name": "source_id", "type": "string"},
        {"name": "source_network", "type": "string"},
        {
            "name": "event_time_ms",
            "type": {"type": "long", "logicalType": "timestamp-millis"},
        },
        {"name": "latitude", "type": "double"},
        {"name": "longitude", "type": "double"},
        {"name": "depth_km", "type": ["null", "double"], "default": None},
        {"name": "magnitude", "type": "double"},
        {"name": "magnitude_type", "type": "string"},
        {"name": "region_name", "type": ["null", "string"], "default": None},
        {"name": "quality_indicator", "type": "string"},
        {"name": "raw_payload", "type": "string"},
        {
            "name": "ingested_at_ms",
            "type": {"type": "long", "logicalType": "timestamp-millis"},
        },
        {"name": "pipeline_version", "type": "string"},
        # ── Analysis enrichment ──────────────────────────────────────────
        {
            "name": "ml_magnitude",
            "type": "double",
            "doc": (
                "Refined local magnitude (ML scale). Derived from the reported "
                "magnitude via empirical cross-scale conversion and depth correction."
            ),
        },
        {
            "name": "ml_magnitude_source",
            "type": "string",
            "doc": (
                "Conversion path: raw_ml | ml_depth_corrected | "
                "converted_from_mw | converted_from_mb | converted_from_ms | passthrough"
            ),
        },
        {"name": "is_aftershock", "type": "boolean"},
        {
            "name": "mainshock_source_id",
            "type": ["null", "string"],
            "default": None,
            "doc": "source_id of the candidate mainshock when is_aftershock = true.",
        },
        {
            "name": "mainshock_magnitude",
            "type": ["null", "double"],
            "default": None,
        },
        {
            "name": "mainshock_distance_km",
            "type": ["null", "double"],
            "default": None,
            "doc": "Epicentral distance to the mainshock in km.",
        },
        {
            "name": "mainshock_time_delta_hours",
            "type": ["null", "double"],
            "default": None,
            "doc": "Hours elapsed since the mainshock.",
        },
        {
            "name": "estimated_felt_radius_km",
            "type": "double",
            "doc": (
                "Empirical estimate of the radius (km) within which shaking "
                "is likely to be felt.  Based on Bakun & Scotti (2006) simplified."
            ),
        },
        {
            "name": "estimated_intensity_mmi",
            "type": "double",
            "doc": (
                "Estimated Modified Mercalli Intensity at the epicentre. "
                "Based on Wald et al. (1999) simplified attenuation."
            ),
        },
        {
            "name": "enriched_at_ms",
            "type": {"type": "long", "logicalType": "timestamp-millis"},
        },
        {"name": "analysis_version", "type": "string"},
    ],
}

ALERT_SCHEMA: dict = {
    "type": "record",
    "name": "EarthquakeAlert",
    "namespace": "com.seismosis",
    "doc": "Alert record produced for seismic events meeting the magnitude threshold.",
    "fields": [
        {"name": "source_id", "type": "string"},
        {
            "name": "event_time_ms",
            "type": {"type": "long", "logicalType": "timestamp-millis"},
        },
        {"name": "latitude", "type": "double"},
        {"name": "longitude", "type": "double"},
        {"name": "depth_km", "type": ["null", "double"], "default": None},
        {"name": "magnitude", "type": "double"},
        {"name": "ml_magnitude", "type": "double"},
        {"name": "region_name", "type": ["null", "string"], "default": None},
        {"name": "estimated_intensity_mmi", "type": "double"},
        {"name": "estimated_felt_radius_km", "type": "double"},
        {"name": "is_aftershock", "type": "boolean"},
        {
            "name": "alert_level",
            "type": "string",
            "doc": "YELLOW (ML ≥ 5.0) | ORANGE (ML ≥ 6.0) | RED (ML ≥ 7.0)",
        },
        {
            "name": "triggered_at_ms",
            "type": {"type": "long", "logicalType": "timestamp-millis"},
        },
    ],
}
