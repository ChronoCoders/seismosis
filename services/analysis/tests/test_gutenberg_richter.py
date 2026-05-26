"""Tests for the Gutenberg-Richter b-value estimation module."""
from __future__ import annotations

import numpy as np
import pytest

from analysis.gutenberg_richter import estimate_b_value, estimate_mc


class TestEstimateMc:
    def test_estimate_mc_returns_plausible_value(self) -> None:
        """
        Synthetic GR catalog with peak frequency near M=2.0 should return
        an Mc close to 2.0 (within 0.3 magnitude units).
        """
        rng = np.random.default_rng(0)

        # Build a catalog: many small events clustered around M=2.0, fewer at higher M.
        # FMD peak is deliberately at M=2.0.
        low_mags = rng.uniform(1.9, 2.1, size=300)   # dominant bin
        high_mags = rng.uniform(2.1, 5.0, size=50)   # declining tail
        magnitudes = np.concatenate([low_mags, high_mags])

        mc = estimate_mc(magnitudes, bin_width=0.1)

        assert 1.7 <= mc <= 2.3, (
            f"Expected Mc near 2.0, got {mc:.2f}. "
            "Mc method should detect the frequency peak at ~M=2.0."
        )


class TestEstimateBValue:
    def test_estimate_b_value_known_result(self) -> None:
        """
        For a synthetic catalog where the Aki-Utsu formula yields b≈1.0,
        the returned b-value must be within 0.1 of 1.0.

        Aki-Utsu MLE: b = log10(e) / (mean_M - mc)
        For b=1.0: mean_M - mc = log10(e) ≈ 0.4343.
        Using mc=2.0, mean_M should be ~2.4343.
        We approximate this with an exponential distribution shifted by mc.
        """
        rng = np.random.default_rng(42)
        mc = 2.0
        # Exponential distribution with rate = b * ln(10) ≈ 2.3026 for b=1.0
        # gives mean excess = 1 / (b * ln(10)) ≈ 0.4343
        excess = rng.exponential(scale=1.0 / (1.0 * np.log(10)), size=500)
        magnitudes = excess + mc

        b_value, b_std, a_value, n_events = estimate_b_value(magnitudes, mc=mc)

        assert abs(b_value - 1.0) < 0.1, (
            f"Expected b≈1.0 for exponential catalog, got b={b_value:.4f}."
        )
        assert b_std > 0.0, "b_std must be positive."
        assert n_events == magnitudes.size, "All events are above mc in this dataset."
        assert a_value > 0.0, "a_value (log10 intercept) should be positive for large catalogs."

    def test_estimate_b_value_raises_on_insufficient_data(self) -> None:
        """
        Fewer than 20 events above mc must raise ValueError.
        """
        magnitudes = np.array([2.1, 2.5, 3.0, 3.5, 2.8])  # 5 events — well below 20
        with pytest.raises(ValueError, match="Insufficient events"):
            estimate_b_value(magnitudes, mc=2.0)
