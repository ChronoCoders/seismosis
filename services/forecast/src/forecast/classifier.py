"""Seismicity classifier: tectonic vs. induced using HistGradientBoostingClassifier.

Falls back to a deterministic rule-based approach when insufficient training
data is available or before the first training run.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Optional, Sequence

import numpy as np
import numpy.typing as npt
from sklearn.ensemble import HistGradientBoostingClassifier  # type: ignore[import-untyped]

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

MODEL_VERSION: str = "hgb-classifier-v1"
RULE_VERSION: str = "rule-based-v1"
_N_MIN_TRAIN: int = 50
_CLASSES: list[str] = ["tectonic", "induced", "volcanic"]


# ---------------------------------------------------------------------------
# Result dataclass
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ClassificationResult:
    source_id: str
    event_class: str
    confidence: float


# ---------------------------------------------------------------------------
# Rule-based label
# ---------------------------------------------------------------------------


def _rule_label(depth_km: float, magnitude: float) -> str:
    """Simple deterministic seismicity label for a single event."""
    if depth_km > 70.0:
        return "tectonic"
    if depth_km < 10.0 and magnitude < 2.5:
        return "induced"
    return "tectonic"


# ---------------------------------------------------------------------------
# Classifier class
# ---------------------------------------------------------------------------


class SeismicClassifier:
    """Gradient-boosting seismicity classifier with a rule-based fallback."""

    def __init__(self) -> None:
        self._model: Optional[HistGradientBoostingClassifier] = None
        self._is_trained: bool = False

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _build_features(
        self,
        depths: Sequence[float],
        mags: Sequence[float],
        lats: Sequence[float],
        lons: Sequence[float],
    ) -> npt.NDArray[Any]:
        """Stack per-event scalars into an (N, 4) feature matrix.

        NaN values are intentionally preserved — HGB handles missing data
        natively.
        """
        arr: npt.NDArray[Any] = np.column_stack([
            np.array(depths, dtype=np.float64),
            np.array(mags, dtype=np.float64),
            np.array(lats, dtype=np.float64),
            np.array(lons, dtype=np.float64),
        ])
        return arr

    # ------------------------------------------------------------------
    # Training
    # ------------------------------------------------------------------

    def train(
        self,
        depths: Sequence[float],
        mags: Sequence[float],
        lats: Sequence[float],
        lons: Sequence[float],
    ) -> int:
        """Train on the provided catalogue using rule-based pseudo-labels.

        Returns the number of training samples used, or 0 if training was
        skipped due to insufficient data.
        """
        n: int = len(depths)
        if n < _N_MIN_TRAIN:
            self._is_trained = False
            return 0

        # Generate rule-based pseudo-labels
        labels: list[str] = [
            _rule_label(float(d), float(m))
            for d, m in zip(depths, mags)
        ]

        X: npt.NDArray[Any] = self._build_features(depths, mags, lats, lons)
        y: npt.NDArray[Any] = np.array(labels)

        # Keep only tectonic and induced (drop volcanic if none present)
        present_classes: set[str] = set(labels)
        if "volcanic" not in present_classes:
            mask: npt.NDArray[Any] = y != "volcanic"
            X = X[mask]
            y = y[mask]

        n_train: int = len(X)
        if n_train < _N_MIN_TRAIN:
            self._is_trained = False
            return 0

        self._model = HistGradientBoostingClassifier(max_iter=100, random_state=42)
        self._model.fit(X, y)
        self._is_trained = True
        return n_train

    # ------------------------------------------------------------------
    # Prediction
    # ------------------------------------------------------------------

    def predict(
        self,
        source_ids: Sequence[str],
        depths: Sequence[float],
        mags: Sequence[float],
        lats: Sequence[float],
        lons: Sequence[float],
    ) -> list[ClassificationResult]:
        """Classify events using the trained model.

        Falls back to rule-based prediction if the model has not been trained.
        """
        if not self._is_trained or self._model is None:
            return self.predict_rule_based(source_ids, depths, mags)

        X: npt.NDArray[Any] = self._build_features(depths, mags, lats, lons)
        y_pred: npt.NDArray[Any] = self._model.predict(X)
        y_proba: npt.NDArray[Any] = self._model.predict_proba(X)

        results: list[ClassificationResult] = []
        for i, sid in enumerate(source_ids):
            event_class: str = str(y_pred[i])
            confidence: float = float(np.max(y_proba[i]))
            results.append(ClassificationResult(
                source_id=sid,
                event_class=event_class,
                confidence=confidence,
            ))
        return results

    def predict_rule_based(
        self,
        source_ids: Sequence[str],
        depths: Sequence[float],
        mags: Sequence[float],
    ) -> list[ClassificationResult]:
        """Rule-based classification with a fixed confidence of 0.6."""
        results: list[ClassificationResult] = []
        for sid, d, m in zip(source_ids, depths, mags):
            event_class: str = _rule_label(float(d), float(m))
            results.append(ClassificationResult(
                source_id=sid,
                event_class=event_class,
                confidence=0.6,
            ))
        return results
