"""
Seismicity event classifier: tectonic vs induced vs volcanic.

Uses scikit-learn HistGradientBoostingClassifier.
Model is loaded from disk on first call (lazy init, thread-safe).
"""
from __future__ import annotations

import threading
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import joblib
import numpy as np
import numpy.typing as npt

CLASSES = ["tectonic", "induced", "volcanic"]
MODEL_VERSION = "classifier-v1"

_lock = threading.Lock()
_model: object | None = None

# Path where a trained model is expected at runtime.  Configurable via the
# CLASSIFIER_MODEL_PATH environment variable if callers wrap this module.
_DEFAULT_MODEL_PATH = Path(__file__).parent / "artifacts" / "classifier.joblib"


@dataclass
class ClassificationResult:
    event_class: str
    confidence: float
    probabilities: dict[str, float]
    model_version: str


# ---------------------------------------------------------------------------
# Feature engineering
# ---------------------------------------------------------------------------

def _build_features(
    magnitude: float,
    depth_km: float | None,
    b_value_local: float | None,
    magnitude_type: str,
) -> npt.NDArray[Any]:
    """
    Build feature vector for a single event. Returns shape (1, 5).

    Features
    --------
    0  magnitude
    1  depth_km         (default 10.0 when None)
    2  depth_mag_ratio  (depth / max(magnitude, 0.1))
    3  b_value_local    (default 1.0 when None)
    4  focal_depth_class  0=shallow (<15 km), 1=crustal (<70 km), 2=deep
    """
    depth = depth_km if depth_km is not None else 10.0
    b_val = b_value_local if b_value_local is not None else 1.0
    depth_mag_ratio = depth / max(magnitude, 0.1)
    focal_depth_class = 0 if depth < 15 else (1 if depth < 70 else 2)
    return np.array([[magnitude, depth, depth_mag_ratio, b_val, focal_depth_class]])


# ---------------------------------------------------------------------------
# Rule-based fallback
# ---------------------------------------------------------------------------

def _rule_based_classify(
    magnitude: float,
    depth_km: float | None,
) -> ClassificationResult:
    """
    Fallback classification when no trained model is available on disk.

    Rules:
    - depth < 5 km AND magnitude < 3.5 → induced (confidence 0.55)
    - otherwise → tectonic (confidence 0.70)
    """
    depth = depth_km if depth_km is not None else 10.0

    if depth < 5.0 and magnitude < 3.5:
        cls = "induced"
        conf = 0.55
        probs = {"tectonic": 0.25, "induced": 0.55, "volcanic": 0.20}
    else:
        cls = "tectonic"
        conf = 0.70
        probs = {"tectonic": 0.70, "induced": 0.20, "volcanic": 0.10}

    return ClassificationResult(
        event_class=cls,
        confidence=conf,
        probabilities=probs,
        model_version=f"{MODEL_VERSION}-rules",
    )


# ---------------------------------------------------------------------------
# Model loading
# ---------------------------------------------------------------------------

def _load_model(model_path: Path = _DEFAULT_MODEL_PATH) -> object | None:
    """
    Load the model from disk. Returns None if the file does not exist.
    Thread-safe via module-level _lock (caller must hold the lock).
    """
    if not model_path.exists():
        return None
    loaded: object = joblib.load(model_path)
    return loaded


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

def classify_event(
    magnitude: float,
    depth_km: float | None,
    b_value_local: float | None,
    magnitude_type: str,
    model_path: Path = _DEFAULT_MODEL_PATH,
) -> ClassificationResult:
    """
    Run inference on a single event.

    If no trained model exists on disk, returns a rule-based fallback:
    - depth < 5 km AND magnitude < 3.5 → induced (confidence 0.55)
    - otherwise → tectonic (confidence 0.70)

    The model is loaded lazily on first call and cached for subsequent calls.

    Parameters
    ----------
    magnitude:
        Reported (or calibrated) magnitude value.
    depth_km:
        Hypocentral depth in km. Uses 10.0 when None.
    b_value_local:
        Regional b-value estimate. Uses 1.0 when None.
    magnitude_type:
        Magnitude scale string (e.g. "ML", "Mw"). Currently reserved for
        future feature engineering.
    model_path:
        Path to the serialised joblib model file.

    Returns
    -------
    ClassificationResult with event_class, confidence, probabilities, model_version.
    """
    global _model

    with _lock:
        if _model is None:
            _model = _load_model(model_path)

    if _model is None:
        return _rule_based_classify(magnitude, depth_km)

    features = _build_features(magnitude, depth_km, b_value_local, magnitude_type)

    # The stored model must expose predict_proba (sklearn convention).
    model_with_proba: Any = _model
    proba: npt.NDArray[Any] = model_with_proba.predict_proba(features)[0]

    best_idx = int(np.argmax(proba))
    event_class = CLASSES[best_idx]
    confidence = float(proba[best_idx])
    probabilities = {cls: float(p) for cls, p in zip(CLASSES, proba)}

    return ClassificationResult(
        event_class=event_class,
        confidence=confidence,
        probabilities=probabilities,
        model_version=MODEL_VERSION,
    )


def train_classifier(
    magnitudes: list[float],
    depths: list[float],
    b_values: list[float],
    labels: list[str],
    artifact_path: Path,
) -> dict[str, float]:
    """
    Train a HistGradientBoostingClassifier and save it with joblib.

    Uses stratified 5-fold cross-validation to compute precision, recall,
    and F1 metrics. The model is saved to artifact_path after training on
    the full dataset.

    Only call this function from a training/offline script — not from the
    hot processing path.

    Parameters
    ----------
    magnitudes:
        Training magnitudes.
    depths:
        Training depths (km). Must be same length as magnitudes.
    b_values:
        Local b-value estimates. Must be same length as magnitudes.
    labels:
        Ground-truth class labels from CLASSES. Must be same length as magnitudes.
    artifact_path:
        Filesystem path to write the serialised model.

    Returns
    -------
    Dict with keys "precision", "recall", "f1" (macro-averaged).

    Raises
    ------
    ValueError
        If any label is not in CLASSES, or input lists differ in length.
    ImportError
        Propagated from sklearn if scikit-learn is not installed.
    """
    from sklearn.ensemble import HistGradientBoostingClassifier
    from sklearn.model_selection import StratifiedKFold, cross_validate
    from sklearn.preprocessing import LabelEncoder

    n = len(magnitudes)
    if not (n == len(depths) == len(b_values) == len(labels)):
        raise ValueError("All input lists must have the same length.")

    invalid = [lbl for lbl in labels if lbl not in CLASSES]
    if invalid:
        raise ValueError(f"Unknown labels found: {set(invalid)}. Valid: {CLASSES}")

    X: npt.NDArray[Any] = np.array(
        [
            [
                magnitudes[i],
                depths[i],
                depths[i] / max(magnitudes[i], 0.1),
                b_values[i],
                0 if depths[i] < 15 else (1 if depths[i] < 70 else 2),
            ]
            for i in range(n)
        ]
    )

    le = LabelEncoder()
    le.fit(CLASSES)
    y: npt.NDArray[Any] = le.transform(labels)

    clf = HistGradientBoostingClassifier(
        max_iter=200,
        learning_rate=0.05,
        max_depth=4,
        random_state=42,
    )

    cv = StratifiedKFold(n_splits=5, shuffle=True, random_state=42)
    cv_results: dict[str, Any] = cross_validate(
        clf,
        X,
        y,
        cv=cv,
        scoring=["precision_macro", "recall_macro", "f1_macro"],
    )

    precision = float(np.mean(cv_results["test_precision_macro"]))
    recall = float(np.mean(cv_results["test_recall_macro"]))
    f1 = float(np.mean(cv_results["test_f1_macro"]))

    # Fit on full data and persist.
    clf.fit(X, y)
    artifact_path.parent.mkdir(parents=True, exist_ok=True)
    joblib.dump(clf, artifact_path)

    return {"precision": precision, "recall": recall, "f1": f1}
