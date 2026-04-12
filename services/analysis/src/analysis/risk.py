"""
Seismic risk estimation: felt radius and Modified Mercalli Intensity (MMI).

All relationships are empirical approximations suitable for rapid situational
awareness and alert classification.  They are NOT appropriate for
engineering-level damage assessment, site-specific hazard analysis, or
structural vulnerability estimation.

No machine-learning prediction is performed here.  These are physics-derived
point-source attenuation relations applied to catalogue magnitude and depth.

References
----------
* Bakun, W.H. & Scotti, O. (2006) "Regional intensity attenuation models
  for France and the estimation of magnitude and location of historical
  earthquakes."  Geophysical Journal International 164(3): 596–610.
  → Felt-radius formula.
* Wald, D.J., Quitoriano, V., Heaton, T.H. & Kanamori, H. (1999)
  "Relationships between Peak Ground Acceleration, Peak Ground Velocity, and
  Modified Mercalli Intensity in California."  Earthquake Spectra 15(3).
  → Epicentral MMI attenuation.
"""
from __future__ import annotations

import math
from typing import Optional


def estimate_felt_radius_km(
    ml_magnitude: float,
    depth_km: Optional[float],
) -> float:
    """
    Estimate the radius (km) within which shaking is expected to be felt.

    Uses log10(R_hypo) ≈ 0.5·ML − 0.8 (Bakun & Scotti 2006 simplified),
    projected to the surface using: R_surface² = R_hypo² − depth².

    Returns at least 1.0 km.

    Parameters
    ----------
    ml_magnitude:
        Refined local magnitude.
    depth_km:
        Hypocentral depth in km.  Defaults to 10 km when None.
    """
    depth = max(depth_km if depth_km is not None else 10.0, 0.0)
    log_r_hypo = 0.5 * ml_magnitude - 0.8
    r_hypo_km = max(10.0 ** log_r_hypo, 1.0)
    surface_sq = r_hypo_km ** 2 - depth ** 2
    felt_km = math.sqrt(max(surface_sq, 0.0))
    return max(round(felt_km, 2), 1.0)


def estimate_epicentral_mmi(
    ml_magnitude: float,
    depth_km: Optional[float],
) -> float:
    """
    Estimate the Modified Mercalli Intensity (MMI) at the epicentre.

    Uses the Wald et al. (1999) simplified form at zero epicentral distance:
        MMI ≈ 1.5·ML − 1.5·log10(depth) − 0.5

    The result is clamped to the MMI scale [I, XII] and rounded to 2 dp.

    Parameters
    ----------
    ml_magnitude:
        Refined local magnitude.
    depth_km:
        Hypocentral depth in km.  Defaults to 10 km when None; minimum 1 km
        to avoid log10(0).
    """
    depth = max(depth_km if depth_km is not None else 10.0, 1.0)
    mmi = 1.5 * ml_magnitude - 1.5 * math.log10(depth) - 0.5
    return round(min(max(mmi, 1.0), 12.0), 2)


def alert_level(ml_magnitude: float) -> Optional[str]:
    """
    Map a refined ML magnitude to a three-tier alert level.

    Returns
    -------
    "YELLOW"  for ML ≥ 5.0
    "ORANGE"  for ML ≥ 6.0
    "RED"     for ML ≥ 7.0
    None      below threshold

    The caller is responsible for applying the configured threshold from
    Config.alert_magnitude_threshold before calling this function.
    """
    if ml_magnitude >= 7.0:
        return "RED"
    if ml_magnitude >= 6.0:
        return "ORANGE"
    if ml_magnitude >= 5.0:
        return "YELLOW"
    return None
