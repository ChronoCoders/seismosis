"""Gutenberg-Richter b-value analysis on a spatial grid.

Implements:
- Maximum Curvature method for magnitude of completeness (Mc) estimation.
- Aki-Utsu MLE b-value estimation.
- 0.5° spatial grid decomposition for Turkey region.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from datetime import datetime
from typing import Any, Optional, Sequence

import numpy as np
import numpy.typing as npt

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

MODEL_VERSION: str = "gr-aki-utsu-maxcurv-v1"
_BIN_WIDTH: float = 0.1
_N_MIN: int = 30
GRID_STEP: float = 0.5


# ---------------------------------------------------------------------------
# Result dataclass
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class GrResult:
    b_value: float
    b_std: float
    a_value: float
    mc: float
    n_events: int
    catalog_start: datetime
    catalog_end: datetime


# ---------------------------------------------------------------------------
# Mc estimation (Maximum Curvature)
# ---------------------------------------------------------------------------


def estimate_mc(magnitudes: npt.NDArray[Any]) -> float:
    """Estimate the magnitude of completeness using the Maximum Curvature method.

    Returns the bin centre with the highest non-cumulative frequency.
    Falls back to min(magnitudes) when fewer than 5 events are present.
    """
    if len(magnitudes) < 5:
        return float(np.min(magnitudes))

    mag_min: float = float(np.floor(np.min(magnitudes) / _BIN_WIDTH) * _BIN_WIDTH)
    mag_max: float = float(np.ceil(np.max(magnitudes) / _BIN_WIDTH) * _BIN_WIDTH)

    bins: npt.NDArray[Any] = np.arange(mag_min, mag_max + _BIN_WIDTH, _BIN_WIDTH)
    counts: npt.NDArray[Any]
    edges: npt.NDArray[Any]
    counts, edges = np.histogram(magnitudes, bins=bins)

    if len(counts) == 0:
        return float(np.min(magnitudes))

    peak_idx: int = int(np.argmax(counts))
    # Bin centre
    mc: float = float(edges[peak_idx] + _BIN_WIDTH / 2.0)
    return mc


# ---------------------------------------------------------------------------
# b-value estimation (Aki-Utsu MLE)
# ---------------------------------------------------------------------------


def estimate_b_aki_utsu(
    magnitudes: npt.NDArray[Any],
    mc: float,
) -> tuple[float, float]:
    """Estimate b-value and its standard error via Aki-Utsu MLE.

    Returns
    -------
    (b_value, b_std)
        b-value clipped to [0.3, 3.0] and the associated standard error.
    """
    above: npt.NDArray[Any] = magnitudes[magnitudes >= mc]
    n: int = len(above)

    if n < _N_MIN:
        return 1.0, 0.1

    mean_mag: float = float(np.mean(above))
    denom: float = mean_mag - mc

    if denom <= 0.0:
        return 1.0, 0.1

    # Aki-Utsu MLE
    b_raw: float = math.log10(math.e) / denom
    b: float = float(np.clip(b_raw, 0.3, 3.0))

    # Standard error: σ_b = b² * sqrt(2/N) / log10(e)
    b_std: float = (b ** 2) * math.sqrt(2.0 / n) / math.log10(math.e)

    return b, b_std


# ---------------------------------------------------------------------------
# Full catalog analysis
# ---------------------------------------------------------------------------


def analyze_catalog(
    times: Sequence[datetime],
    lats: Sequence[float],
    lons: Sequence[float],
    mags: Sequence[float],
    catalog_start: datetime,
    catalog_end: datetime,
    region_name: Optional[str] = None,
) -> Optional[GrResult]:
    """Compute GR b-value for a seismic catalogue.

    Returns *None* when the catalogue has fewer than *_N_MIN* events.
    """
    if len(mags) < _N_MIN:
        return None

    mags_arr: npt.NDArray[Any] = np.array(mags, dtype=np.float64)
    mc: float = estimate_mc(mags_arr)
    above_mc: npt.NDArray[Any] = mags_arr[mags_arr >= mc]

    b, b_std = estimate_b_aki_utsu(mags_arr, mc)

    n_above: int = len(above_mc)
    a_value: float = math.log10(n_above) + b * mc if n_above > 0 else 0.0

    return GrResult(
        b_value=b,
        b_std=b_std,
        a_value=a_value,
        mc=mc,
        n_events=n_above,
        catalog_start=catalog_start,
        catalog_end=catalog_end,
    )


# ---------------------------------------------------------------------------
# Spatial grid helpers
# ---------------------------------------------------------------------------


def build_grid_cells(
    min_lat: float,
    max_lat: float,
    min_lon: float,
    max_lon: float,
    step: float = GRID_STEP,
) -> list[tuple[float, float, float, float]]:
    """Return a list of (lat_min, lat_max, lon_min, lon_max) grid cells.

    Covers the bounding box [min_lat, max_lat] × [min_lon, max_lon] with
    cells of size *step* × *step* degrees.
    """
    cells: list[tuple[float, float, float, float]] = []

    lat: float = min_lat
    while lat < max_lat:
        lat_max: float = min(lat + step, max_lat)
        lon: float = min_lon
        while lon < max_lon:
            lon_max: float = min(lon + step, max_lon)
            cells.append((lat, lat_max, lon, lon_max))
            lon = round(lon + step, 6)
        lat = round(lat + step, 6)

    return cells


def cell_to_wkt(
    lat_min: float,
    lat_max: float,
    lon_min: float,
    lon_max: float,
) -> str:
    """Convert a bounding-box cell to a WKT POLYGON string (EPSG:4326)."""
    return (
        f"POLYGON(("
        f"{lon_min} {lat_min}, "
        f"{lon_max} {lat_min}, "
        f"{lon_max} {lat_max}, "
        f"{lon_min} {lat_max}, "
        f"{lon_min} {lat_min}"
        f"))"
    )
