"""Unit tests for forecast.etas

Test plan
---------
1. fit_etas falls back to defaults when catalog is too small
2. fit_etas returns physically plausible parameters on a synthetic catalog
3. forecast returns correct structure and value ranges
4. forecast with spatial coords populates spatial_heatmap
5. forecast without spatial coords leaves spatial_heatmap None
6. p_exceedance keys and values are correct
7. daily_rates length matches horizon_days
8. _omori_integral analytic check (p=2, exact closed-form)
9. _haversine_km known distance (London–Paris ≈ 340 km)
10. compute_spatial_heatmap returns valid GeoJSON FeatureCollection
"""
from __future__ import annotations

import math
from datetime import datetime, timedelta, timezone

import pytest

from forecast.etas import (
    EtasParams,
    EtasResult,
    _DEFAULT_PARAMS,
    _N_MIN_FIT,
    _omori_integral,
    _haversine_km,
    compute_spatial_heatmap,
    fit_etas,
    forecast,
)

import numpy as np


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

_EPOCH = datetime(2020, 1, 1, tzinfo=timezone.utc)


def _make_catalog(
    n: int = 200,
    mag_mean: float = 2.5,
    seed: int = 42,
) -> tuple[list[datetime], list[float]]:
    """Return a synthetic Poisson catalog with exponential inter-arrival times."""
    rng = np.random.default_rng(seed)
    # exponential inter-arrival times (mean 6 hours)
    iats = rng.exponential(scale=0.25, size=n)  # days
    times_days = np.cumsum(iats)
    mags = rng.exponential(scale=1.0, size=n) + 1.5  # magnitude above mc
    times = [_EPOCH + timedelta(days=float(t)) for t in times_days]
    return times, mags.tolist()


def _make_spatial_catalog(
    n: int = 50,
    center_lat: float = 38.0,
    center_lon: float = 30.0,
    seed: int = 7,
) -> tuple[list[datetime], list[float], list[float], list[float]]:
    rng = np.random.default_rng(seed)
    iats = rng.exponential(scale=0.5, size=n)
    times_days = np.cumsum(iats)
    mags = rng.exponential(scale=1.0, size=n) + 1.5
    lats = center_lat + rng.uniform(-1.0, 1.0, size=n)
    lons = center_lon + rng.uniform(-1.0, 1.0, size=n)
    times = [_EPOCH + timedelta(days=float(t)) for t in times_days]
    return times, mags.tolist(), lats.tolist(), lons.tolist()


# ---------------------------------------------------------------------------
# 1. fit_etas falls back to defaults for small catalogs
# ---------------------------------------------------------------------------

def test_fit_etas_fallback_on_empty() -> None:
    import numpy as np
    result = fit_etas(np.array([]), np.array([]), mc=1.5, T_days=1.0)
    assert result == _DEFAULT_PARAMS


def test_fit_etas_fallback_below_minimum() -> None:
    import numpy as np
    n = _N_MIN_FIT - 1
    times = np.linspace(0.0, 10.0, n)
    mags = np.full(n, 2.0)
    result = fit_etas(times, mags, mc=1.5, T_days=10.0)
    assert result == _DEFAULT_PARAMS


# ---------------------------------------------------------------------------
# 2. fit_etas returns physically plausible parameters on synthetic catalog
# ---------------------------------------------------------------------------

def test_fit_etas_plausible_params() -> None:
    import numpy as np
    rng = np.random.default_rng(99)
    n = 150
    times = np.sort(rng.uniform(0.0, 365.0, size=n))
    mags = rng.exponential(1.0, size=n) + 1.5
    params = fit_etas(times, mags, mc=1.5, T_days=365.0)

    assert isinstance(params, EtasParams)
    assert params.mu > 0.0, "background rate must be positive"
    assert params.K > 0.0, "productivity must be positive"
    assert params.alpha > 0.0, "magnitude scaling must be positive"
    assert params.c > 0.0, "Omori time offset must be positive"
    assert params.p > 1.0, "Omori exponent should be >1 for stationary process"


# ---------------------------------------------------------------------------
# 3. forecast returns correct structure and value ranges
# ---------------------------------------------------------------------------

def test_forecast_structure_and_ranges() -> None:
    times, mags = _make_catalog(n=100)
    ms_time = times[-1] + timedelta(hours=1)

    result = forecast(
        mainshock_time=ms_time,
        mainshock_mag=5.0,
        catalog_times=times,
        catalog_mags=mags,
        mc=1.5,
        horizon_days=30,
        min_forecast_mag=2.0,
    )

    assert isinstance(result, EtasResult)
    assert result.expected_count >= 0.0
    assert 0.0 <= result.p_at_least_one <= 1.0
    assert len(result.daily_rates) == 30
    assert all(r >= 0.0 for r in result.daily_rates)
    assert result.n_catalog > 0
    assert result.model_version != ""
    assert result.spatial_heatmap is None


# ---------------------------------------------------------------------------
# 4. forecast populates spatial_heatmap when coords provided
# ---------------------------------------------------------------------------

def test_forecast_with_spatial_heatmap() -> None:
    times, mags, lats, lons = _make_spatial_catalog(n=60)
    ms_time = times[-1] + timedelta(hours=2)

    result = forecast(
        mainshock_time=ms_time,
        mainshock_mag=5.5,
        catalog_times=times,
        catalog_mags=mags,
        mc=1.5,
        horizon_days=30,
        catalog_lats=lats,
        catalog_lons=lons,
        mainshock_lat=38.0,
        mainshock_lon=30.0,
    )

    assert result.spatial_heatmap is not None
    hm = result.spatial_heatmap
    assert hm["type"] == "FeatureCollection"
    assert len(hm["features"]) > 0
    # Every feature has required properties
    for feat in hm["features"]:
        props = feat["properties"]
        assert "probability" in props
        assert "expected_count" in props
        assert 0.0 <= props["probability"] <= 1.0
        assert props["expected_count"] >= 0.0


# ---------------------------------------------------------------------------
# 5. forecast without spatial coords leaves spatial_heatmap None
# ---------------------------------------------------------------------------

def test_forecast_no_spatial_heatmap_when_coords_missing() -> None:
    times, mags = _make_catalog(n=50)
    ms_time = times[-1] + timedelta(hours=1)
    result = forecast(
        mainshock_time=ms_time,
        mainshock_mag=4.0,
        catalog_times=times,
        catalog_mags=mags,
    )
    assert result.spatial_heatmap is None


# ---------------------------------------------------------------------------
# 6. p_exceedance keys and values
# ---------------------------------------------------------------------------

def test_p_exceedance_keys_and_ordering() -> None:
    times, mags = _make_catalog(n=100)
    ms_time = times[-1] + timedelta(hours=1)
    result = forecast(
        mainshock_time=ms_time,
        mainshock_mag=6.0,
        catalog_times=times,
        catalog_mags=mags,
        mc=1.5,
    )
    expected_keys = {"2.0", "3.0", "4.0", "5.0"}
    assert set(result.p_exceedance.keys()) == expected_keys

    probs = [result.p_exceedance[k] for k in sorted(result.p_exceedance)]
    # P(M≥2) ≥ P(M≥3) ≥ P(M≥4) ≥ P(M≥5)  (monotone in threshold)
    for i in range(len(probs) - 1):
        assert probs[i] >= probs[i + 1], (
            f"p_exceedance not monotone: P(M≥{2+i})={probs[i]:.4f} < P(M≥{3+i})={probs[i+1]:.4f}"
        )
    for p in probs:
        assert 0.0 <= p <= 1.0


# ---------------------------------------------------------------------------
# 7. daily_rates length matches horizon_days
# ---------------------------------------------------------------------------

@pytest.mark.parametrize("horizon", [7, 30, 90])
def test_daily_rates_length(horizon: int) -> None:
    times, mags = _make_catalog(n=80)
    ms_time = times[-1] + timedelta(hours=1)
    result = forecast(
        mainshock_time=ms_time,
        mainshock_mag=4.5,
        catalog_times=times,
        catalog_mags=mags,
        horizon_days=horizon,
    )
    assert len(result.daily_rates) == horizon


# ---------------------------------------------------------------------------
# 8. _omori_integral analytic check
# ---------------------------------------------------------------------------

def test_omori_integral_p2_analytic() -> None:
    """For p=2, the Omori integral has a closed form:
        ∫_0^H (s + offset + c)^{-2} ds
        = 1/(offset+c) - 1/(H+offset+c)
    """
    c = 0.01
    p = 2.0
    offset = 0.5   # days since event
    horizon = 30.0

    computed = _omori_integral(offset, horizon, c, p)

    a = offset + c
    b = horizon + offset + c
    analytic = 1.0 / a - 1.0 / b

    assert abs(computed - analytic) < 1e-10, (
        f"_omori_integral mismatch: computed={computed}, analytic={analytic}"
    )


def test_omori_integral_p1_log_form() -> None:
    """For p≈1, the integral should be log((H+offset+c)/(offset+c))."""
    c = 0.01
    p = 1.0 + 1e-8   # very close to 1
    offset = 1.0
    horizon = 30.0

    computed = _omori_integral(offset, horizon, c, p)
    analytic = math.log((horizon + offset + c) / (offset + c))

    assert abs(computed - analytic) < 1e-4


# ---------------------------------------------------------------------------
# 9. _haversine_km known distance
# ---------------------------------------------------------------------------

def test_haversine_london_paris() -> None:
    """London (51.5°N, 0.12°W) to Paris (48.86°N, 2.35°E) ≈ 341 km."""
    d = _haversine_km(51.5074, -0.1278, 48.8566, 2.3522)
    assert 335.0 < d < 350.0, f"London–Paris distance = {d:.1f} km, expected ~341 km"


def test_haversine_same_point() -> None:
    d = _haversine_km(38.0, 30.0, 38.0, 30.0)
    assert d == pytest.approx(0.0, abs=1e-6)


# ---------------------------------------------------------------------------
# 10. compute_spatial_heatmap returns valid GeoJSON
# ---------------------------------------------------------------------------

def test_compute_spatial_heatmap_geojson_structure() -> None:
    times, mags, lats, lons = _make_spatial_catalog(n=30)
    ms_time = _EPOCH + timedelta(days=30.0)

    heatmap = compute_spatial_heatmap(
        mainshock_lat=38.0,
        mainshock_lon=30.0,
        mainshock_mag=5.0,
        mainshock_time=ms_time,
        catalog_times=times,
        catalog_mags=mags,
        catalog_lats=lats,
        catalog_lons=lons,
        params=_DEFAULT_PARAMS,
        horizon_days=30,
        grid_spacing_deg=0.5,   # coarser grid for speed
        grid_radius_deg=1.0,
    )

    assert heatmap["type"] == "FeatureCollection"
    features = heatmap["features"]
    assert len(features) > 0

    for feat in features:
        assert feat["type"] == "Feature"
        geom = feat["geometry"]
        assert geom["type"] == "Polygon"
        # Polygon ring has 5 points (closed)
        ring = geom["coordinates"][0]
        assert len(ring) == 5
        assert ring[0] == ring[-1], "polygon ring must be closed"
        # Properties
        props = feat["properties"]
        assert 0.0 <= props["probability"] <= 1.0
        assert props["expected_count"] >= 0.0
