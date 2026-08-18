"""In-process throughput stats for the classifier (windows processed, avg latency)."""

import threading
import time


class ThroughputTracker:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._calls = 0
        self._total_ms = 0.0

    def record(self, elapsed: float) -> None:
        """Record one classifier call of `elapsed` seconds."""
        with self._lock:
            self._calls += 1
            self._total_ms += elapsed * 1000.0

    def snapshot(self) -> dict:
        with self._lock:
            if self._calls == 0:
                return {"windows": 0, "avg_latency_ms": 0.0}
            return {
                "windows": self._calls,
                "avg_latency_ms": round(self._total_ms / self._calls, 1),
            }


class RetrievalTracker:
    """In-process retrieval latency stats (B33) — how long the last vector
    retrieval path took, so the System tab can surface vec0 vs scan health."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._calls = 0
        self._total_ms = 0.0
        self._vec0_calls = 0

    def record(self, elapsed: float, backend: str) -> None:
        """Record one retrieval call of `elapsed` seconds from `backend`
        ('vec0' or 'scan')."""
        with self._lock:
            self._calls += 1
            self._total_ms += elapsed * 1000.0
            if backend == "vec0":
                self._vec0_calls += 1

    def snapshot(self) -> dict:
        with self._lock:
            if self._calls == 0:
                return {"calls": 0, "vec0_calls": 0, "avg_latency_ms": 0.0, "backend": "none"}
            backend = "vec0" if self._vec0_calls >= self._calls / 2 else "scan"
            return {
                "calls": self._calls,
                "vec0_calls": self._vec0_calls,
                "avg_latency_ms": round(self._total_ms / self._calls, 1),
                "backend": backend,
            }


throughput = ThroughputTracker()
retrieval = RetrievalTracker()
