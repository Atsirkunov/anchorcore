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


throughput = ThroughputTracker()
