"""Tests for the ETAS aftershock forecasting module."""
from __future__ import annotations

import numpy as np
import pytest

from analysis.etas import DEFAULT_PARAMS, ETASForecast, forecast_aftershock


class TestForecastReturnsValidProbabilities:
    def test_forecast_returns_valid_probabilities(self) -> None:
        """
        forecast_aftershock must return an ETASForecast where all probabilities
        are in [0, 1] and expected_count is non-negative.
        """
        forecast = forecast_aftershock(
            mainshock_magnitude=6.5,
            mainshock_time_days=0.0,
            params=DEFAULT_PARAMS,
            horizon_days=30,
            min_magnitude=3.0,
        )

        assert isinstance(forecast, ETASForecast)
        assert forecast.expected_count >= 0.0, (
            f"expected_count must be non-negative, got {forecast.expected_count}."
        )
        assert 0.0 <= forecast.p_at_least_one <= 1.0, (
            f"p_at_least_one must be in [0,1], got {forecast.p_at_least_one}."
        )
        assert len(forecast.daily_rates) == 30, (
            "daily_rates must have one entry per horizon day."
        )
        for i, rate in enumerate(forecast.daily_rates):
            assert rate >= 0.0, f"daily_rates[{i}]={rate} is negative."

        for mag, prob in forecast.p_exceedance.items():
            assert 0.0 <= prob <= 1.0, (
                f"p_exceedance[{mag}]={prob} is outside [0,1]."
            )


class TestDefaultParamsProducesForecast:
    def test_default_params_produces_forecast(self) -> None:
        """
        Using DEFAULT_PARAMS, forecast_aftershock must complete without
        raising any exception for a range of mainshock magnitudes.
        """
        for mag in [3.0, 5.0, 7.0, 8.5]:
            forecast = forecast_aftershock(
                mainshock_magnitude=mag,
                mainshock_time_days=0.0,
                params=DEFAULT_PARAMS,
                horizon_days=14,
                min_magnitude=3.0,
            )
            # Basic sanity: larger mainshocks should produce higher expected counts
            assert forecast.expected_count >= 0.0
            assert forecast.model_version == "etas-v1.0"

        # Monotonicity check: M8.5 > M3.0 should yield strictly more aftershocks.
        fc_small = forecast_aftershock(3.0, 0.0, DEFAULT_PARAMS, horizon_days=14)
        fc_large = forecast_aftershock(8.5, 0.0, DEFAULT_PARAMS, horizon_days=14)
        assert fc_large.expected_count > fc_small.expected_count, (
            "Larger mainshocks should produce more expected aftershocks."
        )
