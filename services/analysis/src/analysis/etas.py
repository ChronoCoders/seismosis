"""ETAS aftershock sequence probability forecasting (Ogata 1988)."""
from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Any

import numpy as np
import numpy.typing as npt
from scipy.optimize import minimize


@dataclass
class ETASParams:
    mu: float           # background rate (events/day)
    K: float            # aftershock productivity
    alpha: float        # magnitude sensitivity
    c: float            # Omori-Utsu time offset (days)
    p: float            # Omori-Utsu decay exponent
    mc: float           # magnitude of completeness
    zone: str
    n_events: int
    log_likelihood: float


@dataclass
class ETASForecast:
    mainshock_source_id: str
    expected_count: float           # expected M≥min_mag aftershocks in horizon
    p_at_least_one: float           # P(N≥1)
    daily_rates: list[float]        # per-day expected rate
    p_exceedance: dict[float, float]  # {magnitude: P(max_mag > M)}
    params_zone: str
    params_snapshot: dict[str, float]
    model_version: str


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------

_MIN_EVENTS_FIT = 30
_EXCEEDANCE_MAGNITUDES = [3.0, 4.0, 5.0, 6.0, 7.0]
_MODEL_VERSION = "etas-v1.0"
_N_RESTARTS = 5
_MC_SHIFT = 1e-9   # numerical guard against t_i == t


def _etas_lambda(
    t: float,
    event_times: npt.NDArray[Any],
    magnitudes: npt.NDArray[Any],
    mc: float,
    mu: float,
    K: float,
    alpha: float,
    c: float,
    p: float,
) -> float:
    """Compute ETAS conditional intensity at time t (scalar)."""
    rate = mu
    mask = event_times < t
    if mask.any():
        dt = t - event_times[mask] + c
        mag_factor = np.exp(alpha * (magnitudes[mask] - mc))
        rate += float(np.sum(K * mag_factor / dt ** p))
    return max(rate, 1e-300)


def _log_likelihood(
    params: npt.NDArray[Any],
    event_times: npt.NDArray[Any],
    magnitudes: npt.NDArray[Any],
    mc: float,
    t_end: float,
) -> float:
    """
    Negative log-likelihood for ETAS model.

    L = sum_i log(lambda(t_i)) - integral_0^T lambda(t) dt

    The integral is approximated via a Riemann sum over n_steps bins.
    """
    mu, K, alpha, c, p = float(params[0]), float(params[1]), float(params[2]), float(params[3]), float(params[4])

    # --- sum of log(lambda(t_i)) ---
    log_sum = 0.0
    for i, ti in enumerate(event_times):
        lam = _etas_lambda(ti, event_times[:i], magnitudes[:i], mc, mu, K, alpha, c, p)
        log_val = math.log(lam)
        if not math.isfinite(log_val):
            return 1e15
        log_sum += log_val

    # --- integral of lambda over [0, t_end] via trapezoid (100 points) ---
    n_steps = 100
    ts = np.linspace(0.0, t_end, n_steps)
    rates = np.array([
        _etas_lambda(float(ti), event_times, magnitudes, mc, mu, K, alpha, c, p)
        for ti in ts
    ])
    integral = float(np.trapz(rates, ts))

    nll = -(log_sum - integral)
    return nll if math.isfinite(nll) else 1e15


def fit_etas(
    event_times: npt.NDArray[Any],
    magnitudes: npt.NDArray[Any],
    mc: float,
    zone: str,
) -> ETASParams:
    """
    Maximum likelihood estimation of ETAS parameters.

    Returns ETASParams. Raises ValueError if fewer than 30 events above mc.
    Uses L-BFGS-B optimizer with multiple random restarts (n=5).
    """
    mask = magnitudes >= mc
    t_filtered: npt.NDArray[Any] = event_times[mask]
    m_filtered: npt.NDArray[Any] = magnitudes[mask]

    n_events = int(t_filtered.size)
    if n_events < _MIN_EVENTS_FIT:
        raise ValueError(
            f"Insufficient events above mc={mc:.2f} for ETAS fit: "
            f"got {n_events}, need at least {_MIN_EVENTS_FIT}."
        )

    t_end = float(t_filtered[-1]) if t_filtered.size > 0 else 1.0
    if t_end <= 0.0:
        t_end = 1.0

    # Parameter bounds: mu, K, alpha, c, p
    bounds = [
        (1e-6, None),   # mu  > 0
        (1e-6, None),   # K   > 0
        (1e-6, 3.0),    # alpha in (0, 3)
        (1e-6, 1.0),    # c in (0, 1)
        (1.0, 2.0),     # p in (1, 2)
    ]

    rng = np.random.default_rng(42)
    best_nll = float("inf")
    best_result = None

    for _ in range(_N_RESTARTS):
        x0 = np.array([
            rng.uniform(0.1, 1.0),   # mu
            rng.uniform(0.01, 0.2),  # K
            rng.uniform(0.5, 2.0),   # alpha
            rng.uniform(0.001, 0.1), # c
            rng.uniform(1.0, 1.5),   # p
        ])
        result = minimize(
            _log_likelihood,
            x0,
            args=(t_filtered, m_filtered, mc, t_end),
            method="L-BFGS-B",
            bounds=bounds,
            options={"maxiter": 500, "ftol": 1e-9},
        )
        if result.fun < best_nll:
            best_nll = float(result.fun)
            best_result = result

    if best_result is None:
        raise ValueError("ETAS optimiser failed to converge on any restart.")

    mu, K, alpha, c, p = (float(v) for v in best_result.x)
    return ETASParams(
        mu=mu,
        K=K,
        alpha=alpha,
        c=c,
        p=p,
        mc=mc,
        zone=zone,
        n_events=n_events,
        log_likelihood=-best_nll,
    )


def forecast_aftershock(
    mainshock_magnitude: float,
    mainshock_time_days: float,
    params: ETASParams,
    horizon_days: int = 30,
    min_magnitude: float = 3.0,
) -> ETASForecast:
    """
    Poisson-approximation ETAS forecast for aftershock activity.

    Integrates lambda(t) numerically for each day in the horizon to obtain
    daily rates, then uses Poisson statistics for exceedance probabilities.

    P(at_least_one) = 1 - exp(-expected_count)
    p_exceedance uses Gutenberg-Richter scaling from the fitted alpha parameter
    to estimate P(max_magnitude > M) within the forecast horizon.
    """
    mu = params.mu
    K = params.K
    alpha = params.alpha
    c = params.c
    p = params.p
    mc = params.mc

    # Contribution from the mainshock only (simplified: ignore prior catalog).
    # lambda(t) = mu + K * exp(alpha*(M_main - mc)) / (t - t_main + c)^p
    mag_factor = math.exp(alpha * (mainshock_magnitude - mc))

    # Daily rates: integrate over each 1-day interval
    daily_rates: list[float] = []
    n_points = 50  # integration points per day
    for day in range(horizon_days):
        t_lo = day + _MC_SHIFT
        t_hi = day + 1.0
        ts = np.linspace(t_lo, t_hi, n_points)
        rates = mu + K * mag_factor / (ts + c) ** p
        rate_day = float(np.trapz(rates, ts))
        daily_rates.append(max(rate_day, 0.0))

    # Scale to min_magnitude (GR: N(≥m) = N_total * 10^(-b*(m-mc)), b≈1)
    # alpha = b * ln(10) => b = alpha / ln(10)
    b_approx = alpha / math.log(10.0)
    gr_scale = 10.0 ** (-b_approx * max(min_magnitude - mc, 0.0))

    scaled_rates = [r * gr_scale for r in daily_rates]
    expected_count = sum(scaled_rates)
    p_at_least_one = 1.0 - math.exp(-expected_count)

    # P(exceedance) for fixed magnitude thresholds
    p_exceedance: dict[float, float] = {}
    for mag in _EXCEEDANCE_MAGNITUDES:
        rate_above = expected_count * 10.0 ** (-b_approx * max(mag - min_magnitude, 0.0))
        p_exceedance[mag] = 1.0 - math.exp(-max(rate_above, 0.0))

    params_snapshot: dict[str, float] = {
        "mu": params.mu,
        "K": params.K,
        "alpha": params.alpha,
        "c": params.c,
        "p": params.p,
        "mc": params.mc,
    }

    return ETASForecast(
        mainshock_source_id="",  # caller fills in after construction if needed
        expected_count=expected_count,
        p_at_least_one=p_at_least_one,
        daily_rates=daily_rates,
        p_exceedance=p_exceedance,
        params_zone=params.zone,
        params_snapshot=params_snapshot,
        model_version=_MODEL_VERSION,
    )


DEFAULT_PARAMS = ETASParams(
    mu=0.5,
    K=0.08,
    alpha=1.0,
    c=0.01,
    p=1.1,
    mc=2.0,
    zone="default",
    n_events=0,
    log_likelihood=0.0,
)
