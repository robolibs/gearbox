"""Verify concurrently uploaded typed/legacy neighbors and complete deletion."""

from pathlib import Path
import sys

from draw_cpu_lifecycle_images import verify


if __name__ == "__main__":
    verify(Path(sys.argv[1]), ["contended-ready", "streamed-ready", "cleared"], ["red+blue", "red+blue", "none"])
