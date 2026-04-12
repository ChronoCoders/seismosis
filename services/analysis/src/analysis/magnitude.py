"""
Local magnitude (ML) refinement via empirical cross-scale conversion.

No waveform amplitude data is available at analysis time — refinement is
based on published empirical relations between magnitude scales and
depth-dependent attenuation corrections.

Because we operate on catalogue values only, the returned ml_magnitude is an
estimate with associated uncertainty (see magnitude_uncertainty()).  It is
suitable for risk estimation and alert classification but not for
engineering seismology.

References
----------
* Grünthal, G. et al. (2009) "The Unified Catalogue of Earthquakes in
  Central, Northern, and Northwestern Europe (CENEC) — updated and expanded
  to the last millennium."  Journal of Seismology 13(4): 517–541.
  → Mw ↔ ML conversion coefficients used here.
* Richter, C.F. (1935) "An instrumental earthquake magnitude scale."
  BSSA 25(1): 1–32.
  → Original ML definition and depth-attenuation context.
* Street, R.L. & Turcotte, F.T. (1977) "A study of northeastern North
  American spectral moments, magnitudes, and intensities."  BSSA 67(3).
  → ML ↔ Ms empirical offset.
"""
from __future__ import annotations

from typing import Optional


def refine_to_ml(
    magnitude: float,
    magnitude_type: str,
    depth_km: Optional[float],
) -> tuple[float, str]:
    """
    Convert the reported magnitude to the local magnitude (ML) scale.

    Parameters
    ----------
    magnitude:
        Reported magnitude value.
    magnitude_type:
        Magnitude scale (ML, Mw, MwC, mb, Ms, …).  Case-insensitive.
    depth_km:
        Hypocentral depth in km.  Defaults to 10 km when None.

    Returns
    -------
    (ml_magnitude, source_label)
        ml_magnitude : refined ML value, rounded to 2 decimal places
        source_label : tag describing the conversion path for audit
    """
    mtype = magnitude_type.upper().strip()
    depth = max(depth_km if depth_km is not None else 10.0, 0.1)

    if mtype == "ML":
        return _depth_correct_ml(magnitude, depth)

    if mtype in {"MW", "MWC", "MWW", "MWB", "MWP"}:
        return _mw_to_ml(magnitude, depth)

    if mtype in {"MB", "MB_LG", "MBH"}:
        # mb and ML are approximately equal for shallow crustal events.
        # mb slightly underestimates at M > 5 where surface-wave effects dominate.
        return round(magnitude + 0.1, 2), "converted_from_mb"

    if mtype in {"MS", "MS_20", "MS_BB"}:
        # Surface-wave magnitude tends to overestimate at shallow depths.
        # Simple empirical offset from Street & Turcotte (1977).
        return round(magnitude - 0.3, 2), "converted_from_ms"

    # Unknown scale — pass through unchanged.
    return round(magnitude, 2), "passthrough"


def magnitude_uncertainty(
    ml_magnitude: float,
    ml_magnitude_source: str,
) -> float:
    """
    Estimate the 1-sigma uncertainty on the refined ML value.

    Cross-scale conversions carry inherent scatter.  These values are
    conservative bounds for downstream risk calculations.

    Parameters
    ----------
    ml_magnitude:
        Refined ML value (used for magnitude-dependent bounds on Mw).
    ml_magnitude_source:
        Source label from refine_to_ml().

    Returns
    -------
    1-sigma uncertainty in magnitude units.
    """
    if ml_magnitude_source == "ml_depth_corrected":
        return 0.05
    if ml_magnitude_source == "converted_from_mw":
        # Grünthal et al. (2009) report σ ≈ 0.2 for M ≤ 5, ≈ 0.3 for M > 5.
        return 0.2 if ml_magnitude <= 5.0 else 0.3
    if ml_magnitude_source == "converted_from_mb":
        return 0.3
    if ml_magnitude_source == "converted_from_ms":
        return 0.25
    return 0.1  # passthrough — no conversion applied


# ─── Internal helpers ──────────────────────────────────────────────────────────

def _depth_correct_ml(magnitude: float, depth_km: float) -> tuple[float, str]:
    """
    Apply a depth-dependent correction to a reported ML value.

    Shallow events (< 30 km) are slightly overestimated on vertical
    short-period seismographs due to near-surface amplification.
    The correction is at most −0.05 magnitude units (at the surface).
    """
    shallow_factor = max(0.0, (30.0 - min(depth_km, 30.0)) / 30.0)
    correction = -0.05 * shallow_factor
    return round(magnitude + correction, 2), "ml_depth_corrected"


def _mw_to_ml(magnitude: float, depth_km: float) -> tuple[float, str]:
    """
    Convert moment magnitude (Mw) to local magnitude (ML).

    Applies Grünthal et al. (2009) piecewise coefficients for M 2–6,
    then applies the same depth correction as _depth_correct_ml.
    """
    if magnitude <= 3.5:
        # Table 4 low-magnitude regime
        ml = 0.835 + 1.062 * magnitude
    elif magnitude <= 6.0:
        # Table 4 mid-magnitude regime (linear interpolation)
        ml = 0.921 * magnitude + 1.13
    else:
        # At large magnitudes ML saturates; scales converge to within ~0.1
        ml = magnitude + 0.05

    shallow_factor = max(0.0, (30.0 - min(depth_km, 30.0)) / 30.0)
    ml += -0.05 * shallow_factor

    return round(ml, 2), "converted_from_mw"
