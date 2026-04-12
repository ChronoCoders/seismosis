"""Unit tests for ETAS aftershock detection (analysis.aftershock)."""
from __future__ import annotations

from datetime import datetime, timedelta, timezone
from unittest.mock import MagicMock

import pytest

from analysis.aftershock import (
    classify_aftershock,
    find_mainshock,
    gardner_knopoff_window,
)
from analysis.models import MainshockInfo


# ─── Gardner-Knopoff window ────────────────────────────────────────────────────

class TestGardnerKnopoffWindow:
    def test_returns_positive_values(self):
        days, km = gardner_knopoff_window(5.0)
        assert days > 0.0
        assert km > 0.0

    def test_window_increases_with_magnitude(self):
        for m in [3.0, 4.0, 5.0, 6.0]:
            d_a, r_a = gardner_knopoff_window(m)
            d_b, r_b = gardner_knopoff_window(m + 1.0)
            assert d_b > d_a, f"time window did not increase at M={m}"
            assert r_b > r_a, f"spatial window did not increase at M={m}"

    def test_formula_switch_at_6_5(self):
        # Formula B (≥6.5) grows slower in time than formula A extrapolated
        days_a, _ = gardner_knopoff_window(6.49)
        days_b, _ = gardner_knopoff_window(6.51)
        # Window should increase continuously across the boundary
        assert days_b > days_a * 0.5  # not a dramatic drop

    def test_m7_window_plausible(self):
        days, km = gardner_knopoff_window(7.0)
        assert days > 30     # months
        assert km > 100      # hundreds of km

    def test_m3_window_small(self):
        days, km = gardner_knopoff_window(3.0)
        assert days < 5      # days
        assert km < 30       # tens of km


# ─── classify_aftershock ──────────────────────────────────────────────────────

class TestClassifyAftershock:
    def _make_mainshock(
        self, mag: float = 6.0, hours_ago: float = 10.0, dist_km: float = 30.0
    ) -> MainshockInfo:
        return MainshockInfo(
            source_id="USGS:mainshock",
            magnitude=mag,
            event_time=datetime.now(tz=timezone.utc) - timedelta(hours=hours_ago),
            distance_km=dist_km,
        )

    def test_within_both_windows_is_aftershock(self):
        mainshock = self._make_mainshock(mag=6.5, hours_ago=24.0, dist_km=50.0)
        event_time = mainshock.event_time + timedelta(hours=24)
        is_as, delta_h, dist = classify_aftershock(4.0, event_time, mainshock)
        assert is_as is True
        assert delta_h == pytest.approx(24.0, abs=0.1)
        assert dist == pytest.approx(50.0)

    def test_outside_time_window_not_aftershock(self):
        # 5000 hours (≈208 days) after a M5 event — beyond G-K time window
        mainshock = self._make_mainshock(mag=5.0, hours_ago=5000.0, dist_km=10.0)
        event_time = mainshock.event_time + timedelta(hours=5000)
        is_as, _, _ = classify_aftershock(3.0, event_time, mainshock)
        assert is_as is False

    def test_outside_distance_not_aftershock(self):
        # 3000 km from a M5 — outside G-K spatial window
        mainshock = self._make_mainshock(mag=5.0, hours_ago=1.0, dist_km=3000.0)
        event_time = mainshock.event_time + timedelta(hours=1)
        is_as, _, _ = classify_aftershock(3.0, event_time, mainshock)
        assert is_as is False

    def test_exactly_at_boundary_is_aftershock(self):
        # At exactly the G-K time boundary the event qualifies
        mag = 5.5
        gk_days, gk_km = gardner_knopoff_window(mag)
        mainshock = self._make_mainshock(mag=mag, hours_ago=0.1, dist_km=gk_km * 0.99)
        event_time = mainshock.event_time + timedelta(days=gk_days)
        is_as, _, _ = classify_aftershock(3.0, event_time, mainshock)
        assert is_as is True


# ─── find_mainshock (mocked DB) ────────────────────────────────────────────────

class TestFindMainshock:
    def _make_mock_conn(self, row):
        """Build a minimal mock psycopg2 connection that returns *row*."""
        mock_conn = MagicMock()
        mock_cursor = MagicMock()
        mock_cursor.__enter__ = MagicMock(return_value=mock_cursor)
        mock_cursor.__exit__ = MagicMock(return_value=False)
        mock_cursor.fetchone.return_value = row
        mock_conn.cursor.return_value = mock_cursor
        return mock_conn

    def test_no_db_row_returns_none(self):
        conn = self._make_mock_conn(None)
        result = find_mainshock(
            conn,
            magnitude=3.5,
            latitude=39.0,
            longitude=28.0,
            event_time=datetime.now(tz=timezone.utc),
        )
        assert result is None

    def test_db_row_returns_mainshock_info(self):
        et = datetime(2024, 1, 1, 10, 0, tzinfo=timezone.utc)
        conn = self._make_mock_conn(
            {
                "source_id": "USGS:us7000test",
                "magnitude": 5.8,
                "event_time": et,
                "distance_km": 42.3,
            }
        )
        result = find_mainshock(
            conn,
            magnitude=3.0,
            latitude=39.0,
            longitude=28.0,
            event_time=datetime(2024, 1, 2, 8, 0, tzinfo=timezone.utc),
        )
        assert result is not None
        assert result.source_id == "USGS:us7000test"
        assert result.magnitude == pytest.approx(5.8)
        assert result.distance_km == pytest.approx(42.3)
        assert result.event_time.tzinfo == timezone.utc

    def test_naive_db_datetime_gets_utc_tz(self):
        """Rows from psycopg2 may lack timezone info — must be coerced to UTC."""
        et_naive = datetime(2024, 1, 1, 10, 0)  # no tzinfo
        conn = self._make_mock_conn(
            {
                "source_id": "EMSC:2024test",
                "magnitude": 4.5,
                "event_time": et_naive,
                "distance_km": 20.0,
            }
        )
        result = find_mainshock(
            conn,
            magnitude=2.0,
            latitude=38.0,
            longitude=26.0,
            event_time=datetime.now(tz=timezone.utc),
        )
        assert result is not None
        assert result.event_time.tzinfo == timezone.utc

    def test_window_caps_are_applied(self):
        """Verify query is issued — window cap logic doesn't raise exceptions."""
        conn = self._make_mock_conn(None)
        # Should not raise even with tiny caps
        find_mainshock(
            conn,
            magnitude=3.0,
            latitude=39.0,
            longitude=28.0,
            event_time=datetime.now(tz=timezone.utc),
            max_window_days=1,
            max_window_km=5.0,
        )
        assert conn.cursor.called
