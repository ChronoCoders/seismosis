"""Seismicity classifier: tectonic vs. induced using HistGradientBoostingClassifier.

Falls back to a deterministic rule-based approach when insufficient training
data is available or before the first training run.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Any, Optional

import numpy as np
import numpy.typing as npt
from sklearn.ensemble import HistGradientBoostingClassifier  # type: ignore[import-untyped]
from sklearn.metrics import f1_score  # type: ignore[import-untyped]
from sklearn.model_selection import StratifiedKFold  # type: ignore[import-untyped]
from sklearn.pipeline import Pipeline  # type: ignore[import-untyped]
from sklearn.preprocessing import OrdinalEncoder  # type: ignore[import-untyped]

import structlog

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

MODEL_VERSION: str = "hgb-classifier-v1"
RULE_VERSION: str = "rule-based-v1"
_N_MIN_TRAIN: int = 50
_N_MIN_CV: int = 100
_F1_WARN_THRESHOLD: float = 0.5
_CLASSES: list[str] = ["tectonic", "induced", "volcanic"]

_log = structlog.get_logger("forecast.classifier")

# ---------------------------------------------------------------------------
# Result and feature dataclasses
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ClassificationResult:
    source_id: str
    event_class: str
    confidence: float
    probabilities: dict[str, float]


@dataclass
class FeatureVector:
    source_id: str
    magnitude: float
    depth_km: float
    depth_mag_ratio: float
    b_value_local: float          # NaN if unknown
    inter_event_time_s: float     # NaN if unknown
    nearest_event_dist_km: float  # NaN if unknown
    focal_depth_class: int        # 0=shallow, 1=intermediate, 2=deep
    geology_type: int             # 0-4
    hour_of_day: int              # 0-23
    magnitude_type_enc: int       # 0=ML, 1=Mw, 2=mb, 3=other


# ---------------------------------------------------------------------------
# Static helper functions (exported for use in main.py)
# ---------------------------------------------------------------------------


def _focal_depth_class(depth_km: float) -> int:
    """Encode focal depth into 0=shallow (<15 km), 1=intermediate (15-70 km), 2=deep (>70 km)."""
    if depth_km < 15.0:
        return 0
    if depth_km <= 70.0:
        return 1
    return 2


def _geology_type(lat: float, lon: float) -> int:
    """Static geology type lookup for Turkey region.

    0=fold_thrust: eastern Anatolian thrust belt
    1=graben: western Anatolian grabens / Aegean extensional
    2=volcanic: Central Anatolian volcanic province
    3=platform: stable Anatolian platform
    4=ophiolite: default / everything else
    """
    # Order matters: check more specific regions first
    if 38.0 <= lat <= 39.0 and 34.0 <= lon <= 37.0:
        return 2  # volcanic
    if 38.0 <= lat <= 40.0 and 26.0 <= lon <= 30.0:
        return 1  # graben
    if 36.0 <= lat <= 42.0 and 36.0 <= lon <= 44.0:
        return 0  # fold_thrust
    if 36.0 <= lat <= 38.0 and 30.0 <= lon <= 36.0:
        return 3  # platform
    return 4  # ophiolite / default


def _mag_type_enc(magnitude_type: str) -> int:
    """Ordinal-encode magnitude type: ML=0, Mw=1, mb=2, other=3."""
    mt = magnitude_type.strip().lower()
    if mt == "ml":
        return 0
    if mt in ("mw", "mww", "mwr"):
        return 1
    if mt == "mb":
        return 2
    return 3


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
# Feature matrix builder
# ---------------------------------------------------------------------------


def _to_feature_matrix(feature_vectors: list[FeatureVector]) -> npt.NDArray[Any]:
    """Stack FeatureVector objects into an (N, 10) numpy array.

    Column order:
      0  magnitude
      1  depth_km
      2  depth_mag_ratio
      3  b_value_local
      4  inter_event_time_s
      5  nearest_event_dist_km
      6  focal_depth_class
      7  geology_type
      8  hour_of_day
      9  magnitude_type_enc
    """
    rows: list[list[float]] = []
    for fv in feature_vectors:
        rows.append([
            fv.magnitude,
            fv.depth_km,
            fv.depth_mag_ratio,
            fv.b_value_local,
            fv.inter_event_time_s,
            fv.nearest_event_dist_km,
            float(fv.focal_depth_class),
            float(fv.geology_type),
            float(fv.hour_of_day),
            float(fv.magnitude_type_enc),
        ])
    arr: npt.NDArray[Any] = np.array(rows, dtype=np.float64)
    return arr


# ---------------------------------------------------------------------------
# Classifier class
# ---------------------------------------------------------------------------


def _make_pipeline() -> Pipeline:
    """Build the canonical classifier pipeline: OrdinalEncoder → HistGradientBoosting."""
    return Pipeline([
        (
            "encoder",
            OrdinalEncoder(
                handle_unknown="use_encoded_value",
                unknown_value=-1,
                encoded_missing_value=np.nan,
            ),
        ),
        (
            "clf",
            HistGradientBoostingClassifier(
                max_iter=500,
                learning_rate=0.05,
                max_depth=6,
                min_samples_leaf=20,
                class_weight="balanced",
                random_state=42,
            ),
        ),
    ])


class SeismicClassifier:
    """Gradient-boosting seismicity classifier with a rule-based fallback."""

    def __init__(self) -> None:
        self._model: Optional[Pipeline] = None
        self._is_trained: bool = False

    # ------------------------------------------------------------------
    # Training
    # ------------------------------------------------------------------

    def train(
        self,
        feature_vectors: list[FeatureVector],
        labels: list[str],
    ) -> dict[str, float]:
        """Train on the provided feature vectors with the given labels.

        Runs stratified 5-fold cross-validation if N >= 100 and returns
        metrics dict with macro_f1, precision, recall, n_train.
        Returns empty dict if insufficient data.
        """
        n: int = len(feature_vectors)
        if n < _N_MIN_TRAIN:
            self._is_trained = False
            return {}

        X: npt.NDArray[Any] = _to_feature_matrix(feature_vectors)
        y: npt.NDArray[Any] = np.array(labels)

        # Drop volcanic rows if class is not present (avoids single-class issues)
        present_classes: set[str] = set(labels)
        if "volcanic" not in present_classes:
            mask: npt.NDArray[Any] = y != "volcanic"
            X = X[mask]
            y = y[mask]

        n_train: int = int(len(X))
        if n_train < _N_MIN_TRAIN:
            self._is_trained = False
            return {}

        # Cross-validation for macro-F1 (only when enough data)
        macro_f1: float = float("nan")
        cv_precision: float = float("nan")
        cv_recall: float = float("nan")

        if n_train >= _N_MIN_CV:
            skf = StratifiedKFold(n_splits=5, shuffle=True, random_state=42)
            fold_f1: list[float] = []
            fold_prec: list[float] = []
            fold_rec: list[float] = []

            for train_idx, val_idx in skf.split(X, y):
                X_tr: npt.NDArray[Any] = X[train_idx]
                y_tr: npt.NDArray[Any] = y[train_idx]
                X_val: npt.NDArray[Any] = X[val_idx]
                y_val: npt.NDArray[Any] = y[val_idx]

                fold_model = _make_pipeline()
                fold_model.fit(X_tr, y_tr)
                y_hat: npt.NDArray[Any] = fold_model.predict(X_val)

                unique_labels: list[str] = sorted(set(y_val.tolist()) | set(y_hat.tolist()))
                fold_f1.append(
                    float(f1_score(y_val, y_hat, labels=unique_labels, average="macro", zero_division=0))
                )
                fold_prec.append(
                    float(f1_score(y_val, y_hat, labels=unique_labels, average="macro", zero_division=0))
                )
                fold_rec.append(
                    float(f1_score(y_val, y_hat, labels=unique_labels, average="macro", zero_division=0))
                )

            macro_f1 = float(np.mean(fold_f1))
            cv_precision = float(np.mean(fold_prec))
            cv_recall = float(np.mean(fold_rec))

            if macro_f1 < _F1_WARN_THRESHOLD:
                _log.warning(
                    "classifier.low_f1",
                    macro_f1=macro_f1,
                    threshold=_F1_WARN_THRESHOLD,
                    n_train=n_train,
                )

        # Final model trained on full dataset
        self._model = _make_pipeline()
        self._model.fit(X, y)
        self._is_trained = True

        metrics: dict[str, float] = {
            "n_train": float(n_train),
        }
        if not math.isnan(macro_f1):
            metrics["macro_f1"] = macro_f1
            metrics["precision"] = cv_precision
            metrics["recall"] = cv_recall

        return metrics

    # ------------------------------------------------------------------
    # Prediction
    # ------------------------------------------------------------------

    def predict(
        self,
        feature_vectors: list[FeatureVector],
    ) -> list[ClassificationResult]:
        """Classify events using the trained model.

        Falls back to rule-based prediction if the model has not been trained.
        """
        if not self._is_trained or self._model is None:
            return self._predict_rule_based(feature_vectors)

        X: npt.NDArray[Any] = _to_feature_matrix(feature_vectors)
        y_pred: npt.NDArray[Any] = self._model.predict(X)
        y_proba: npt.NDArray[Any] = self._model.predict_proba(X)
        classes: list[str] = list(self._model.named_steps["clf"].classes_)

        results: list[ClassificationResult] = []
        for i, fv in enumerate(feature_vectors):
            event_class: str = str(y_pred[i])
            proba_row: npt.NDArray[Any] = y_proba[i]
            proba_map: dict[str, float] = {c: float(proba_row[j]) for j, c in enumerate(classes)}
            confidence: float = float(np.max(proba_row))
            results.append(ClassificationResult(
                source_id=fv.source_id,
                event_class=event_class,
                confidence=confidence,
                probabilities=proba_map,
            ))
        return results

    def _predict_rule_based(
        self,
        feature_vectors: list[FeatureVector],
    ) -> list[ClassificationResult]:
        """Rule-based classification with a fixed confidence of 0.6."""
        results: list[ClassificationResult] = []
        for fv in feature_vectors:
            event_class: str = _rule_label(fv.depth_km, fv.magnitude)
            # Assign the rule confidence to the predicted class; spread remainder
            # uniformly across the other two classes.
            other_p: float = (1.0 - 0.6) / (len(_CLASSES) - 1)
            proba_map: dict[str, float] = {
                c: 0.6 if c == event_class else other_p for c in _CLASSES
            }
            results.append(ClassificationResult(
                source_id=fv.source_id,
                event_class=event_class,
                confidence=0.6,
                probabilities=proba_map,
            ))
        return results
