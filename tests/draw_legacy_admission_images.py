"""Verify preserved compact data, automatic retry, and deletion in both native views."""

from pathlib import Path
import sys

from draw_cpu_lifecycle_images import verify


STAGES = ["initial", "denied", "recovered", "namespace-cleared", "deleted"]
COLORS = ["red+blue", "red+blue", "magenta+blue", "blue", "none"]


if __name__ == "__main__":
    verify(Path(sys.argv[1]), STAGES, COLORS)
