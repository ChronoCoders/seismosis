"""ETAS (Epidemic Type Aftershock Sequence) model — Ogata 1988.

Fits model parameters by L-BFGS-B MLE, then forecasts expected aftershock
counts over a configurable horizon window.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Any, Sequence

import numpy as np
import numpy.typing as npt
from scipy.optimize import minimize  # type: ignore[import-untyped]

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

MODEL_VERSION: str = "etas-ogata1988-mle-v1"
_MC_DEFAULT: float = 1.5
_N_MIN_FIT: int = 10
_N_MAX_CATALOG: int = 3_000


# ---------------------------------------------------------------------------
# Data classes
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class EtasParams:
    mu: float      # background rate [events/day]
    K: float       # productivity
    alpha: float   # magnitude scaling
    c: float       # Omori time offset [days]
    p: float       # Omori decay exponent (>1)


@dataclass(frozen=True)
class EtasResult:
    expected_count: float
    p_at_least_one: float
    p_exceedance: dict[str, float]   # {"2.0": p, "3.0": p, ...}
    daily_rates: list[float]
    params: EtasParams
    n_catalog: int
    model_version: str


# ---------------------------------------------------------------------------
# Default / fallback parameters
# ---------------------------------------------------------------------------

_DEFAULT_PARAMS = EtasParams(mu=0.05, K=0.1, alpha=1.5, c=0.01, p=1.1)


# ---------------------------------------------------------------------------
# Negative log-likelihood (unconstrained parameterisation)
# ---------------------------------------------------------------------------


def _neg_log_likelihood(
    x: npt.NDArray[Any],
    times_days: npt.NDArray[Any],
    mags: npt.NDArray[Any],
    mc: float,
    T_days: float,
) -> float:
    """Evaluate the ETAS negative log-likelihood.

    Parameterisation (all unconstrained for L-BFGS-B):
        x = [log_mu, log_K, alpha, log_c, log_p_minus1]

    Returns 1e12 on any numerical error to let the optimiser retreat.
    """
    try:
        mu: float = math.exp(x[0])
        K: float = math.exp(x[1])
        alpha: float = float(x[2])
        c: float = math.exp(x[3])
        p: float = 1.0 + math.exp(x[4])

        n: int = len(times_days)
        log_sum: float = 0.0

        for i in range(n):
            if i == 0:
                lam_i: float = mu
            else:
                dt: npt.NDArray[Any] = times_days[i] - times_days[:i]
                kappa: npt.NDArray[Any] = K * np.exp(alpha * (mags[:i] - mc))
                lam_i = float(mu + np.sum(kappa / (dt + c) ** p))

            if lam_i <= 0.0:
                return 1e12
            log_sum += math.log(lam_i)

        # Compensator (integral of conditional intensity)
        integral: float = mu * T_days

        kappas: npt.NDArray[Any] = K * np.exp(alpha * (mags - mc))
        remaining: npt.NDArray[Any] = T_days - times_days + c
        remaining = np.maximum(remaining, 1e-12)

        if abs(p - 1.0) < 1e-6:
            # p ≈ 1: integral of (t+c)^(-1) from 0 to T-t_j is log((T-t_j+c)/c)
            integrals_j: npt.NDArray[Any] = np.log(remaining / c)
        else:
            # general: [c^(1-p) - (T-t_j+c)^(1-p)] / (p-1)
            integrals_j = (c ** (1.0 - p) - remaining ** (1.0 - p)) / (p - 1.0)

        integral += float(np.sum(kappas * integrals_j))

        return -(log_sum - integral)

    except Exception:
        return 1e12


# ---------------------------------------------------------------------------
# Parameter fitting
# ---------------------------------------------------------------------------


def fit_etas(
    times_days: npt.NDArray[Any],
    mags: npt.NDArray[Any],
    mc: float,
    T_days: float,
) -> EtasParams:
    """Fit ETAS parameters via L-BFGS-B MLE.

    Falls back to *_DEFAULT_PARAMS* on insufficient data or optimisation
    failure.
    """
    if len(times_days) < _N_MIN_FIT:
        return _DEFAULT_PARAMS

    x0: list[float] = [
        math.log(0.05),   # log_mu
        math.log(0.1),    # log_K
        1.5,              # alpha
        math.log(0.01),   # log_c
        math.log(0.1),    # log_p_minus1  → p = 1 + exp(0.1) ≈ 1.1
    ]
    bounds: list[tuple[float, float]] = [
        (-8.0, 3.0),   # log_mu
        (-8.0, 3.0),   # log_K
        (0.1, 3.5),    # alpha
        (-10.0, 2.0),  # log_c
        (-4.0, 3.0),   # log_p_minus1
    ]

    try:
        result = minimize(
            _neg_log_likelihood,
            x0,
            args=(times_days, mags, mc, T_days),
            method="L-BFGS-B",
            bounds=bounds,
            options={"maxiter": 300, "ftol": 1e-9},
        )
        if result.success or result.fun < 1e11:
            xf: npt.NDArray[Any] = result.x
            return EtasParams(
                mu=math.exp(float(xf[0])),
                K=math.exp(float(xf[1])),
                alpha=float(xf[2]),
                c=math.exp(float(xf[3])),
                p=1.0 + math.exp(float(xf[4])),
            )
    except Exception:
        pass

    return _DEFAULT_PARAMS


# ---------------------------------------------------------------------------
# Omori integral helper
# ---------------------------------------------------------------------------


def _omori_integral(
    offset_days: float,
    horizon_days: float,
    c: float,
    p: float,
) -> float:
    """Compute ∫_0^{horizon} (s + offset + c)^{-p} ds.

    = [(horizon+offset+c)^(1-p) - (offset+c)^(1-p)] / (1-p)  for p ≠ 1
    = log((horizon+offset+c) / (offset+c))                     for p ≈ 1
    """
    a: float = offset_days + c
    b: float = horizon_days + a
    a = max(a, 1e-12)
    b = max(b, 1e-12)

    if abs(p - 1.0) < 1e-6:
        return math.log(b / a)
    return (b ** (1.0 - p) - a ** (1.0 - p)) / (1.0 - p)


# ---------------------------------------------------------------------------
# Main forecast function
# ---------------------------------------------------------------------------


def forecast(
    mainshock_time: datetime,
    mainshock_mag: float,
    catalog_times: Sequence[datetime],
    catalog_mags: Sequence[float],
    mc: float = 1.5,
    horizon_days: int = 30,
    min_forecast_mag: float = 1.0,
) -> EtasResult:
    """Generate an ETAS aftershock forecast for *mainshock_time*.

    Parameters
    ----------
    mainshock_time:
        Origin time of the triggering mainshock (timezone-aware UTC).
    mainshock_mag:
        Mainshock magnitude.
    catalog_times:
        Sequence of catalogue event times (timezone-aware UTC).
    catalog_mags:
        Corresponding magnitudes.
    mc:
        Magnitude of completeness for the catalogue.
    horizon_days:
        Forecast horizon in days.
    min_forecast_mag:
        Minimum magnitude threshold for the forecast output.
    """
    ms_time: datetime = mainshock_time
    if ms_time.tzinfo is None:
        ms_time = ms_time.replace(tzinfo=timezone.utc)

    # Convert to days relative to mainshock
    raw_pairs: list[tuple[float, float]] = []
    for ct, cm in zip(catalog_times, catalog_mags):
        t: datetime = ct
        if t.tzinfo is None:
            t = t.replace(tzinfo=timezone.utc)
        dt_days: float = (t - ms_time).total_seconds() / 86_400.0
        if cm >= mc:
            raw_pairs.append((dt_days, cm))

    # Sort by time
    raw_pairs.sort(key=lambda tup: tup[0])

    # Limit to most recent _N_MAX_CATALOG events
    if len(raw_pairs) > _N_MAX_CATALOG:
        raw_pairs = raw_pairs[-_N_MAX_CATALOG:]

    if raw_pairs:
        times_arr: npt.NDArray[Any] = np.array([p[0] for p in raw_pairs], dtype=np.float64)
        mags_arr: npt.NDArray[Any] = np.array([p[1] for p in raw_pairs], dtype=np.float64)
        T_days: float = float(times_arr[-1])
        if T_days <= 0.0:
            T_days = 1.0
    else:
        times_arr = np.array([], dtype=np.float64)
        mags_arr = np.array([], dtype=np.float64)
        T_days = 1.0

    n_catalog: int = len(times_arr)

    # Fit ETAS parameters on the observed catalogue
    params: EtasParams = fit_etas(times_arr, mags_arr, mc, T_days)

    # -----------------------------------------------------------------
    # Forecast Λ over [T, T + horizon_days]
    # -----------------------------------------------------------------
    b: float = 1.0  # assumed b-value for magnitude scaling

    # Scale factor: expected count at min_forecast_mag relative to mc
    mag_scale: float = 10.0 ** (b * (mc - min_forecast_mag))

    # Background rate contribution
    lambda_mu: float = params.mu * horizon_days

    # Aftershock contribution: sum over all catalogue events
    lambda_as: float = 0.0
    kappas: npt.NDArray[Any] = (
        params.K * np.exp(params.alpha * (mags_arr - mc))
        if n_catalog > 0
        else np.array([], dtype=np.float64)
    )
    for j in range(n_catalog):
        offset: float = T_days - float(times_arr[j])
        lambda_as += float(kappas[j]) * _omori_integral(offset, float(horizon_days), params.c, params.p)

    expected_count_at_mc: float = max(0.0, lambda_mu + lambda_as)
    expected_count: float = expected_count_at_mc * mag_scale

    p_at_least_one: float = 1.0 - math.exp(-max(0.0, expected_count))

    # Daily rates
    daily_rates: list[float] = []
    for day in range(horizon_days):
        d_lambda_mu: float = params.mu
        d_lambda_as: float = 0.0
        for j in range(n_catalog):
            offset_d: float = T_days + float(day) - float(times_arr[j])
            d_lambda_as += float(kappas[j]) * _omori_integral(offset_d, 1.0, params.c, params.p)
        daily_rate: float = max(0.0, (d_lambda_mu + d_lambda_as) * mag_scale)
        daily_rates.append(daily_rate)

    # p_exceedance for threshold magnitudes
    p_exceedance: dict[str, float] = {}
    for m_thresh in (2.0, 3.0, 4.0, 5.0):
        scale_thresh: float = 10.0 ** (b * (mc - m_thresh))
        exp_at_thresh: float = expected_count_at_mc * scale_thresh
        p_exceedance[str(m_thresh)] = 1.0 - math.exp(-max(0.0, exp_at_thresh))

    return EtasResult(
        expected_count=expected_count,
        p_at_least_one=p_at_least_one,
        p_exceedance=p_exceedance,
        daily_rates=daily_rates,
        params=params,
        n_catalog=n_catalog,
        model_version=MODEL_VERSION,
    )
