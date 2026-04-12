"""Unit tests for ML magnitude refinement (analysis.magnitude)."""
from __future__ import annotations

import pytest

from analysis.magnitude import magnitude_uncertainty, refine_to_ml


class TestRefineToMl:
    # ── ML input ───────────────────────────────────────────────────────────

    def test_ml_at_surface_gets_negative_correction(self):
        ml, src = refine_to_ml(4.0, "ML", 0.0)
        assert src == "ml_depth_corrected"
        assert ml < 4.0  # shallow correction is negative

    def test_ml_below_30km_no_correction(self):
        ml, src = refine_to_ml(4.0, "ML", 50.0)
        assert src == "ml_depth_corrected"
        assert ml == pytest.approx(4.0, abs=0.01)

    def test_ml_none_depth_defaults_to_10km(self):
        ml_none, _ = refine_to_ml(3.5, "ML", None)
        ml_10, _ = refine_to_ml(3.5, "ML", 10.0)
        assert ml_none == ml_10

    # ── Mw conversions ─────────────────────────────────────────────────────

    def test_mw_small_uses_low_regime(self):
        # Grünthal: 0.835 + 1.062 * 2.0 = 2.959
        ml, src = refine_to_ml(2.0, "Mw", 10.0)
        assert src == "converted_from_mw"
        assert ml == pytest.approx(2.96, abs=0.1)

    def test_mw_medium_uses_mid_regime(self):
        ml, src = refine_to_ml(5.0, "MW", 10.0)
        assert src == "converted_from_mw"
        assert 4.0 <= ml <= 7.0

    def test_mw_large_unity_mapping(self):
        ml, src = refine_to_ml(7.5, "MWC", 15.0)
        assert src == "converted_from_mw"
        assert ml == pytest.approx(7.55, abs=0.05)

    def test_mww_and_mwb_treated_as_mw(self):
        ml_mww, _ = refine_to_ml(4.5, "MWW", 10.0)
        ml_mwb, _ = refine_to_ml(4.5, "MWB", 10.0)
        assert ml_mww == ml_mwb

    # ── mb and Ms conversions ──────────────────────────────────────────────

    def test_mb_offset(self):
        ml, src = refine_to_ml(4.8, "mb", 20.0)
        assert src == "converted_from_mb"
        assert ml == pytest.approx(4.9, abs=0.01)

    def test_mb_lg_treated_as_mb(self):
        ml, src = refine_to_ml(4.0, "MB_LG", 10.0)
        assert src == "converted_from_mb"

    def test_ms_offset_negative(self):
        ml, src = refine_to_ml(6.0, "Ms", 10.0)
        assert src == "converted_from_ms"
        assert ml == pytest.approx(5.7, abs=0.01)

    def test_ms_20_treated_as_ms(self):
        ml, src = refine_to_ml(5.5, "MS_20", 10.0)
        assert src == "converted_from_ms"

    # ── Unknown / passthrough ──────────────────────────────────────────────

    def test_unknown_type_passthrough(self):
        ml, src = refine_to_ml(3.1, "MD", 10.0)
        assert src == "passthrough"
        assert ml == pytest.approx(3.1, abs=0.01)

    def test_case_insensitive(self):
        ml_lower, _ = refine_to_ml(4.0, "mw", 10.0)
        ml_upper, _ = refine_to_ml(4.0, "MW", 10.0)
        assert ml_lower == ml_upper

    # ── Roundtrip precision ────────────────────────────────────────────────

    def test_result_rounded_to_two_decimals(self):
        ml, _ = refine_to_ml(3.456789, "ML", 10.0)
        assert ml == round(ml, 2)


class TestMagnitudeUncertainty:
    def test_ml_corrected_low_uncertainty(self):
        assert magnitude_uncertainty(3.0, "ml_depth_corrected") == pytest.approx(0.05)

    def test_mw_small_sigma(self):
        assert magnitude_uncertainty(3.5, "converted_from_mw") == pytest.approx(0.2)

    def test_mw_large_sigma(self):
        assert magnitude_uncertainty(6.0, "converted_from_mw") == pytest.approx(0.3)

    def test_mb_sigma(self):
        assert magnitude_uncertainty(4.0, "converted_from_mb") == pytest.approx(0.3)

    def test_ms_sigma(self):
        assert magnitude_uncertainty(5.0, "converted_from_ms") == pytest.approx(0.25)

    def test_passthrough_sigma(self):
        assert magnitude_uncertainty(3.0, "passthrough") == pytest.approx(0.1)
