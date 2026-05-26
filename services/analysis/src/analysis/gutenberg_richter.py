"""Gutenberg-Richter b-value estimation (Aki-Utsu MLE)."""
from __future__ import annotations

import math
from typing import Any

import numpy as np
import numpy.typing as npt

_MIN_EVENTS_B = 20


def estimate_mc(magnitudes: npt.NDArray[Any], bin_width: float = 0.1) -> float:
    """
    Maximum Curvature method for magnitude of completeness.

    Builds a non-cumulative frequency-magnitude distribution (FMD) histogram
    and returns the bin centre with the highest frequency as Mc.

    Parameters
    ----------
    magnitudes:
        Array of magnitude values.
    bin_width:
        Histogram bin width in magnitude units (default 0.1).

    Returns
    -------
    Magnitude of completeness (centre of peak-frequency bin).
    """
    if magnitudes.size == 0:
        return 0.0

    m_min = float(np.floor(magnitudes.min() / bin_width) * bin_width)
    m_max = float(np.ceil(magnitudes.max() / bin_width) * bin_width)

    # np.arange can miss the last bin due to float precision; add a small pad.
    bins = np.arange(m_min, m_max + bin_width * 1.01, bin_width)
    counts, edges = np.histogram(magnitudes, bins=bins)

    if counts.size == 0:
        return float(magnitudes.min())

    peak_idx = int(np.argmax(counts))
    # Bin centre
    mc = float(edges[peak_idx] + bin_width / 2.0)
    return round(mc, 2)


def estimate_b_value(
    magnitudes: npt.NDArray[Any],
    mc: float,
) -> tuple[float, float, float, int]:
    """
    Aki-Utsu MLE b-value estimation.

    Parameters
    ----------
    magnitudes:
        Array of all magnitude values.
    mc:
        Magnitude of completeness; only events with M ≥ mc are used.

    Returns
    -------
    (b_value, b_std, a_value, n_events)
        b_value : Gutenberg-Richter b-value
        b_std   : standard deviation (Shi & Bolt 1982)
        a_value : log10(N) + b * mc
        n_events: number of events used (M ≥ mc)

    Raises
    ------
    ValueError
        If fewer than 20 events are above mc.
    """
    above: npt.NDArray[Any] = magnitudes[magnitudes >= mc]
    n_events = int(above.size)

    if n_events < _MIN_EVENTS_B:
        raise ValueError(
            f"Insufficient events above mc={mc:.2f} for b-value estimation: "
            f"got {n_events}, need at least {_MIN_EVENTS_B}."
        )

    mean_mag = float(np.mean(above))
    denom = mean_mag - mc
    if denom <= 0.0:
        raise ValueError(
            f"Mean magnitude ({mean_mag:.3f}) is not greater than mc ({mc:.3f}); "
            "cannot estimate b-value."
        )

    b_value = math.log10(math.e) / denom               # Aki-Utsu MLE
    b_std = b_value / math.sqrt(n_events)               # Shi & Bolt 1982
    a_value = math.log10(n_events) + b_value * mc       # GR intercept

    return round(b_value, 4), round(b_std, 4), round(a_value, 4), n_events


def compute_gr_for_region(
    event_times: list[str],
    magnitudes: list[float],
    region_name: str,
    model_version: str = "gr-v1.0",
) -> dict[str, object]:
    """
    Compute Gutenberg-Richter parameters for a region and return a dict
    ready for DB insertion into seismology.gr_analysis.

    Parameters
    ----------
    event_times:
        ISO 8601 UTC timestamp strings for each event.
    magnitudes:
        Magnitude value for each event (parallel to event_times).
    region_name:
        Human-readable region identifier.
    model_version:
        Model version tag (default "gr-v1.0").

    Returns
    -------
    Dict with keys: b_value, b_std, a_value, mc, n_events,
    catalog_start, catalog_end, region_name, model_version.

    Raises
    ------
    ValueError
        Propagated from estimate_b_value if insufficient events above mc.
    """
    mag_arr = np.array(magnitudes, dtype=float)
    mc = estimate_mc(mag_arr)
    b_value, b_std, a_value, n_events = estimate_b_value(mag_arr, mc)

    sorted_times = sorted(event_times)
    catalog_start = sorted_times[0] if sorted_times else ""
    catalog_end = sorted_times[-1] if sorted_times else ""

    return {
        "b_value": b_value,
        "b_std": b_std,
        "a_value": a_value,
        "mc": mc,
        "n_events": n_events,
        "catalog_start": catalog_start,
        "catalog_end": catalog_end,
        "region_name": region_name,
        "model_version": model_version,
    }
