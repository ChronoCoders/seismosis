"""
Prometheus metrics for the historical ingestion service.

Exposed on :9097/metrics via the HTTP server started in main.py.
Metric naming convention: seismosis_historical_{metric}_{unit}
"""
from __future__ import annotations

from prometheus_client import CollectorRegistry, Counter, Histogram


class Metrics:
    """Container for all Prometheus metrics used by the historical ingestion service."""

    def __init__(self, registry: CollectorRegistry) -> None:
        self.events_ingested_total: Counter = Counter(
            "seismosis_historical_events_ingested_total",
            "Total number of historical seismic events successfully upserted into PostgreSQL",
            registry=registry,
        )
        self.fetch_errors_total: Counter = Counter(
            "seismosis_historical_fetch_errors_total",
            "Total number of HTTP fetch errors encountered while downloading from USGS ComCat",
            registry=registry,
        )
        self.ingest_duration_seconds: Histogram = Histogram(
            "seismosis_historical_ingest_duration_seconds",
            "Duration of each batch upsert operation in seconds",
            buckets=(0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0),
            registry=registry,
        )
