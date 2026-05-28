"""Unit tests for forecast.gutenberg_richter

Test plan
---------
1.  estimate_mc fallback for fewer than 5 events
2.  estimate_mc returns the non-cumulative FMD peak
3.  estimate_b_aki_utsu fallback for fewer than 30 events
4.  estimate_b_aki_utsu analytic check (b=1.0 when mean−Mc = log10(e))
5.  estimate_b_aki_utsu standard-error formula
6.  estimate_b_aki_utsu clips b to [0.3, 3.0]
7.  estimate_b_aki_utsu handles denom ≤ 0 gracefully
8.  analyze_catalog returns None for small catalog
9.  analyze_catalog returns GrResult with consistent fields
10. analyze_catalog a_value = log10(n_above) + b * mc
11. compute_fmd cumulative counts are monotone decreasing
12. compute_fmd entries start at (or above) mc
13. compute_fmd stops at zero count
14. build_grid_cells Turkey region has expected cell count
15. build_grid_cells every cell is within the requested bounding box
16. build_grid_cells cells have the expected step size
17. cell_to_wkt produces a closed WKT POLYGON ring
18. cell_to_wkt encodes coordinates in lon-lat (WGS-84 / GeoJSON) order
"""
from __future__ import annotations

import math
from datetime import datetime, timezone

import numpy as np
import pytest

from forecast.gutenberg_richter import (
    GrResult,
    _BIN_WIDTH,
    _N_MIN,
    GRID_STEP,
    analyze_catalog,
    build_grid_cells,
    cell_to_wkt,
    compute_fmd,
    estimate_b_aki_utsu,
    estimate_mc,
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

_T0 = datetime(2016, 1, 1, tzinfo=timezone.utc)
_T1 = datetime(2026, 1, 1, tzinfo=timezone.utc)
_LOG10_E: float = math.log10(math.e)   # ≈ 0.43429


def _uniform_catalog(
    n: int,
    mag_low: float = 1.5,
    mag_high: float = 5.0,
    seed: int = 0,
) -> np.ndarray:
    rng = np.random.default_rng(seed)
    return rng.uniform(mag_low, mag_high, size=n).astype(np.float64)


def _exponential_catalog(
    n: int,
    mc: float = 1.5,
    b: float = 1.0,
    seed: int = 1,
) -> np.ndarray:
    """Gutenberg-Richter synthetic catalog: mags drawn from Exp(b*ln10) + mc."""
    rng = np.random.default_rng(seed)
    # Inverse-CDF: m = mc - log(U) / (b * ln10)
    u = rng.uniform(0.0, 1.0, size=n)
    mags = mc - np.log(u) / (b * math.log(10.0))
    return mags.astype(np.float64)


# ===========================================================================
# 1. estimate_mc fallback for < 5 events
# ===========================================================================

def test_estimate_mc_fallback_tiny_catalog() -> None:
    mags = np.array([2.0, 3.0, 4.0], dtype=np.float64)
    mc = estimate_mc(mags)
    assert mc == pytest.approx(float(np.min(mags)), abs=1e-9)


# ===========================================================================
# 2. estimate_mc returns the non-cumulative FMD peak
# ===========================================================================

def test_estimate_mc_peak_bin() -> None:
    """Most events cluster in [2.0, 2.1) → Mc should be near 2.05."""
    rng = np.random.default_rng(42)
    peak = rng.uniform(2.0, 2.1, size=300)
    scatter = rng.uniform(2.1, 5.0, size=30)
    mags = np.concatenate([peak, scatter])
    mc = estimate_mc(mags)
    # The peak bin centre is 2.05; allow ±0.15 for histogram edge effects
    assert 1.9 <= mc <= 2.2, f"expected Mc near 2.05, got {mc}"


# ===========================================================================
# 3. estimate_b_aki_utsu fallback for < _N_MIN events
# ===========================================================================

def test_estimate_b_fallback_small() -> None:
    mags = np.array([2.0] * (_N_MIN - 1), dtype=np.float64)
    b, b_std = estimate_b_aki_utsu(mags, mc=1.5)
    assert b == pytest.approx(1.0)
    assert b_std == pytest.approx(0.1)


def test_estimate_b_fallback_empty() -> None:
    b, b_std = estimate_b_aki_utsu(np.array([], dtype=np.float64), mc=1.5)
    assert b == pytest.approx(1.0)
    assert b_std == pytest.approx(0.1)


# ===========================================================================
# 4. estimate_b_aki_utsu analytic check: b = 1.0 when mean − Mc = log10(e)
# ===========================================================================

def test_estimate_b_analytic_b_equals_one() -> None:
    """When all events have magnitude mc + log10(e), mean − mc = log10(e),
    and the Aki-Utsu estimator gives b = log10(e)/log10(e) = 1.0."""
    mc = 1.5
    exact_mag = mc + _LOG10_E
    n = 200
    mags = np.full(n, exact_mag, dtype=np.float64)
    b, _ = estimate_b_aki_utsu(mags, mc=mc)
    assert b == pytest.approx(1.0, abs=1e-9)


# ===========================================================================
# 5. estimate_b_aki_utsu standard-error formula
# ===========================================================================

def test_estimate_b_std_formula() -> None:
    """σ_b = b² * sqrt(2/N) / log10(e)."""
    mc = 1.5
    exact_mag = mc + _LOG10_E   # → b = 1.0
    n = 500
    mags = np.full(n, exact_mag, dtype=np.float64)
    b, b_std = estimate_b_aki_utsu(mags, mc=mc)

    expected_std = (b ** 2) * math.sqrt(2.0 / n) / _LOG10_E
    assert b_std == pytest.approx(expected_std, rel=1e-6)


# ===========================================================================
# 6. estimate_b_aki_utsu clips b to [0.3, 3.0]
# ===========================================================================

def test_estimate_b_clips_high() -> None:
    """Very small mean − mc → very high raw b → clipped to 3.0."""
    mc = 1.5
    # mean − mc = 1e-4 → raw b = log10(e)/1e-4 ≈ 4343 → clipped to 3.0
    mags = np.full(200, mc + 1e-4, dtype=np.float64)
    b, _ = estimate_b_aki_utsu(mags, mc=mc)
    assert b == pytest.approx(3.0, abs=1e-9)


def test_estimate_b_clips_low() -> None:
    """Large mean − mc → small raw b → clipped to 0.3."""
    mc = 1.5
    # mean − mc = 10 → raw b = log10(e)/10 ≈ 0.043 → clipped to 0.3
    mags = np.full(200, mc + 10.0, dtype=np.float64)
    b, _ = estimate_b_aki_utsu(mags, mc=mc)
    assert b == pytest.approx(0.3, abs=1e-9)


# ===========================================================================
# 7. estimate_b_aki_utsu handles denom ≤ 0 (all events exactly at mc)
# ===========================================================================

def test_estimate_b_zero_denom_returns_default() -> None:
    mc = 2.0
    mags = np.full(200, mc, dtype=np.float64)
    b, b_std = estimate_b_aki_utsu(mags, mc=mc)
    assert b == pytest.approx(1.0)
    assert b_std == pytest.approx(0.1)


# ===========================================================================
# 8. analyze_catalog returns None for small catalog
# ===========================================================================

def test_analyze_catalog_none_for_small() -> None:
    n = _N_MIN - 1
    mags = _uniform_catalog(n)
    lats = [38.0] * n
    lons = [30.0] * n
    times = [_T0] * n
    result = analyze_catalog(times, lats, lons, mags.tolist(), _T0, _T1)
    assert result is None


# ===========================================================================
# 9. analyze_catalog returns GrResult with consistent fields
# ===========================================================================

def test_analyze_catalog_returns_grresult() -> None:
    n = 500
    mags = _exponential_catalog(n, mc=1.5, b=1.0)
    lats = [38.0] * n
    lons = [30.0] * n
    times = [_T0] * n
    result = analyze_catalog(times, lats, lons, mags.tolist(), _T0, _T1)

    assert isinstance(result, GrResult)
    assert result.b_value > 0.0
    assert result.b_std > 0.0
    assert result.mc >= float(np.min(mags))
    assert result.n_events > 0
    assert result.n_events <= n
    assert result.catalog_start == _T0
    assert result.catalog_end == _T1
    assert isinstance(result.fmd, list)
    assert len(result.fmd) > 0


# ===========================================================================
# 10. analyze_catalog: a_value = log10(n_above) + b * mc
# ===========================================================================

def test_analyze_catalog_a_value_formula() -> None:
    n = 400
    mags = _exponential_catalog(n, mc=1.5, b=1.0)
    lats = [38.0] * n
    lons = [30.0] * n
    times = [_T0] * n
    result = analyze_catalog(times, lats, lons, mags.tolist(), _T0, _T1)

    assert result is not None
    mags_arr = np.array(mags, dtype=np.float64)
    n_above = int(np.sum(mags_arr >= result.mc))
    expected_a = math.log10(n_above) + result.b_value * result.mc
    assert result.a_value == pytest.approx(expected_a, rel=1e-9)


# ===========================================================================
# 11. compute_fmd cumulative counts are monotone decreasing
# ===========================================================================

def test_compute_fmd_monotone_decreasing() -> None:
    mags = _exponential_catalog(500, mc=1.5, b=1.0)
    mc = estimate_mc(mags)
    fmd = compute_fmd(mags, mc)

    assert len(fmd) >= 2
    counts = [entry["cumulative_count"] for entry in fmd]
    for i in range(len(counts) - 1):
        assert counts[i] >= counts[i + 1], (
            f"FMD not monotone at index {i}: "
            f"count[{i}]={counts[i]} < count[{i+1}]={counts[i+1]}"
        )


# ===========================================================================
# 12. compute_fmd entries start at or above mc
# ===========================================================================

def test_compute_fmd_starts_at_mc() -> None:
    mags = _exponential_catalog(300, mc=1.5, b=1.0)
    mc = 1.5
    fmd = compute_fmd(mags, mc)
    assert len(fmd) > 0
    assert fmd[0]["magnitude"] >= mc - _BIN_WIDTH


# ===========================================================================
# 13. compute_fmd stops when count reaches zero
# ===========================================================================

def test_compute_fmd_stops_at_zero() -> None:
    mags = np.array([2.0, 2.0, 2.5, 3.0], dtype=np.float64)
    fmd = compute_fmd(mags, mc=2.0)
    # All entries must have count > 0
    for entry in fmd:
        assert entry["cumulative_count"] > 0


# ===========================================================================
# 14. build_grid_cells Turkey region has expected cell count
# ===========================================================================

def test_build_grid_cells_turkey_count() -> None:
    """Turkey bounding box 33–45°N, 22–48°E at 0.5° step:
    lat rows = (45−33)/0.5 = 24, lon cols = (48−22)/0.5 = 52 → 1248 cells."""
    cells = build_grid_cells(
        min_lat=33.0, max_lat=45.0, min_lon=22.0, max_lon=48.0, step=0.5
    )
    assert len(cells) == 24 * 52, f"expected 1248 cells, got {len(cells)}"


# ===========================================================================
# 15. build_grid_cells: every cell is within the bounding box
# ===========================================================================

def test_build_grid_cells_within_bounds() -> None:
    min_lat, max_lat, min_lon, max_lon = 33.0, 45.0, 22.0, 48.0
    cells = build_grid_cells(min_lat, max_lat, min_lon, max_lon)
    for lat_min, lat_max, lon_min, lon_max in cells:
        assert lat_min >= min_lat - 1e-9
        assert lat_max <= max_lat + 1e-9
        assert lon_min >= min_lon - 1e-9
        assert lon_max <= max_lon + 1e-9


# ===========================================================================
# 16. build_grid_cells: cells have the expected step size
# ===========================================================================

def test_build_grid_cells_step_size() -> None:
    cells = build_grid_cells(
        min_lat=33.0, max_lat=45.0, min_lon=22.0, max_lon=48.0, step=0.5
    )
    for lat_min, lat_max, lon_min, lon_max in cells:
        assert lat_max - lat_min == pytest.approx(0.5, abs=1e-6)
        assert lon_max - lon_min == pytest.approx(0.5, abs=1e-6)


# ===========================================================================
# 17. cell_to_wkt produces a closed WKT POLYGON ring
# ===========================================================================

def test_cell_to_wkt_closed_ring() -> None:
    wkt = cell_to_wkt(lat_min=38.0, lat_max=38.5, lon_min=29.0, lon_max=29.5)
    assert wkt.startswith("POLYGON((")
    assert wkt.endswith("))")
    # Extract coordinates between POLYGON(( and ))
    inner = wkt[len("POLYGON(("):-len("))")]
    pairs = [p.strip() for p in inner.split(",")]
    assert len(pairs) == 5, "ring must have 5 points (4 corners + closing)"
    assert pairs[0] == pairs[-1], "ring must be closed (first == last)"


# ===========================================================================
# 18. cell_to_wkt encodes coordinates in lon lat (GeoJSON / PostGIS WGS-84)
# ===========================================================================

def test_cell_to_wkt_lon_lat_order() -> None:
    """WKT for PostGIS geography uses lon lat, not lat lon."""
    lat_min, lat_max = 38.0, 38.5
    lon_min, lon_max = 29.0, 29.5
    wkt = cell_to_wkt(lat_min, lat_max, lon_min, lon_max)

    # First point should be lon_min lat_min
    inner = wkt[len("POLYGON(("):-len("))")]
    first_pair = inner.split(",")[0].strip().split()
    first_lon, first_lat = float(first_pair[0]), float(first_pair[1])
    assert first_lon == pytest.approx(lon_min)
    assert first_lat == pytest.approx(lat_min)

    # All longitudes should be in [lon_min, lon_max]
    pairs = [p.strip().split() for p in inner.split(",")]
    for lon_str, lat_str in pairs:
        assert lon_min - 1e-9 <= float(lon_str) <= lon_max + 1e-9
        assert lat_min - 1e-9 <= float(lat_str) <= lat_max + 1e-9
