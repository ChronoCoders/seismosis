"""Prometheus metrics for the forecast service."""

from __future__ import annotations

from prometheus_client import CollectorRegistry, Counter, Gauge, Histogram


class Metrics:
    """All Prometheus metrics for the forecast service.

    Instantiated with a *CollectorRegistry* so that tests can use an isolated
    registry without polluting the global default one.
    """

    def __init__(self, registry: CollectorRegistry) -> None:
        self.etas_computed: Counter = Counter(
            "seismosis_forecast_etas_computed_total",
            "ETAS forecasts written to the database",
            registry=registry,
        )
        self.etas_errors: Counter = Counter(
            "seismosis_forecast_etas_errors_total",
            "ETAS computation errors",
            registry=registry,
        )
        self.gr_cells_computed: Counter = Counter(
            "seismosis_forecast_gr_cells_computed_total",
            "Gutenberg-Richter grid cells written to the database",
            registry=registry,
        )
        self.gr_errors: Counter = Counter(
            "seismosis_forecast_gr_errors_total",
            "Gutenberg-Richter computation errors",
            registry=registry,
        )
        self.classifier_events: Counter = Counter(
            "seismosis_forecast_classifier_events_total",
            "Seismic events classified",
            registry=registry,
        )
        self.classifier_errors: Counter = Counter(
            "seismosis_forecast_classifier_errors_total",
            "Seismicity classifier errors",
            registry=registry,
        )
        self.cycle_duration: Histogram = Histogram(
            "seismosis_forecast_cycle_duration_seconds",
            "Duration of a full forecast cycle in seconds",
            registry=registry,
        )
