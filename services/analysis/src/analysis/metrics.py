"""Prometheus metrics for the analysis service."""
from __future__ import annotations

from prometheus_client import Counter, Histogram, Registry


class Metrics:
    """All Prometheus metrics registered against a caller-provided Registry."""

    def __init__(self, registry: Registry) -> None:
        self.messages_consumed_total = Counter(
            "seismosis_analysis_messages_consumed_total",
            "Total Kafka messages consumed from earthquakes.raw",
            ["topic"],
            registry=registry,
        )
        self.messages_enriched_total = Counter(
            "seismosis_analysis_messages_enriched_total",
            "Total events successfully enriched and produced to earthquakes.enriched",
            ["source_network"],
            registry=registry,
        )
        self.aftershocks_detected_total = Counter(
            "seismosis_analysis_aftershocks_detected_total",
            "Total events classified as aftershocks via ETAS window analysis",
            registry=registry,
        )
        self.alerts_produced_total = Counter(
            "seismosis_analysis_alerts_produced_total",
            "Total alert events produced to earthquakes.alerts",
            ["alert_level"],
            registry=registry,
        )
        self.dead_letter_total = Counter(
            "seismosis_analysis_dead_letter_total",
            "Total messages routed to the dead-letter topic",
            ["failure_reason"],
            registry=registry,
        )
        self.processing_duration_seconds = Histogram(
            "seismosis_analysis_processing_duration_seconds",
            "End-to-end processing time per message (decode → enrich → produce → db → cache)",
            ["topic"],
            buckets=[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0],
            registry=registry,
        )
        self.magnitude_correction = Histogram(
            "seismosis_analysis_magnitude_correction",
            "Signed difference between refined ML and reported magnitude (ml_magnitude − magnitude)",
            ["magnitude_type"],
            buckets=[-2.0, -1.5, -1.0, -0.5, -0.25, -0.1, 0.0, 0.1, 0.25, 0.5, 1.0, 1.5, 2.0],
            registry=registry,
        )
        self.db_write_errors_total = Counter(
            "seismosis_analysis_db_write_errors_total",
            "PostgreSQL write failures (non-fatal; enriched event still produced to topic)",
            registry=registry,
        )
        self.cache_write_errors_total = Counter(
            "seismosis_analysis_cache_write_errors_total",
            "Redis write failures (non-fatal)",
            registry=registry,
        )
