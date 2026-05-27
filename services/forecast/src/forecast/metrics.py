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
        self.etas_duration: Histogram = Histogram(
            "seismosis_forecast_etas_duration_seconds",
            "Wall-clock time to compute a single ETAS forecast in seconds",
            registry=registry,
        )
        self.gr_recomputes: Counter = Counter(
            "seismosis_forecast_gr_recomputes_total",
            "Gutenberg-Richter analyses (global + grid cells) written to the database",
            registry=registry,
        )
        self.gr_errors: Counter = Counter(
            "seismosis_forecast_gr_errors_total",
            "Gutenberg-Richter computation errors",
            registry=registry,
        )
        self.classifier_inferences: Counter = Counter(
            "seismosis_forecast_classifier_inferences_total",
            "Seismic events classified by the seismicity classifier",
            registry=registry,
        )
        self.classifier_errors: Counter = Counter(
            "seismosis_forecast_classifier_errors_total",
            "Seismicity classifier errors",
            registry=registry,
        )
        self.model_confidence: Gauge = Gauge(
            "seismosis_forecast_model_confidence",
            "Latest cross-validation macro-F1 score of the active seismicity classifier",
            registry=registry,
        )
        self.cycle_duration: Histogram = Histogram(
            "seismosis_forecast_cycle_duration_seconds",
            "Duration of a full forecast cycle in seconds",
            registry=registry,
        )
