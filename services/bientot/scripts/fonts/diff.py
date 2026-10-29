"""Compares two folders of screenshots (browse.ts --shots) pixel by pixel. For each pair it prints
the number of pixels that differ, the largest channel difference and the boxes around the changes,
and writes an image of the changes in red over the "before" shot into the output folder.
Usage: python3 scripts/fonts/diff.py <before dir> <after dir> <output dir>
"""

import sys
from pathlib import Path

import numpy as np
from PIL import Image


def boxes(mask: np.ndarray, gap: int = 12) -> list[tuple[int, int, int, int]]:
    """Rough boxes (x0, y0, x1, y1) around the changed pixels, merging rows closer than gap."""
    rows = np.where(mask.any(axis=1))[0]
    out = []
    start = prev = None
    for r in list(rows) + [None]:
        if r is not None and prev is not None and r - prev <= gap:
            prev = r
            continue
        if start is not None:
            cols = np.where(mask[start : prev + 1].any(axis=0))[0]
            out.append((int(cols[0]), int(start), int(cols[-1]), int(prev)))
        start = prev = r
    return out


def main() -> None:
    before, after, out = (Path(a) for a in sys.argv[1:4])
    out.mkdir(parents=True, exist_ok=True)
    same = 0
    for path in sorted(before.glob("*.png")):
        a = np.asarray(Image.open(path).convert("RGB")).astype(int)
        other = after / path.name
        if not other.exists():
            print(f"{path.name}: missing after")
            continue
        b = np.asarray(Image.open(other).convert("RGB")).astype(int)
        if a.shape != b.shape:
            print(f"{path.name}: size {a.shape[1]}x{a.shape[0]} -> {b.shape[1]}x{b.shape[0]}")
            continue
        delta = np.abs(a - b).max(axis=2)
        mask = delta > 0
        if not mask.any():
            same += 1
            continue
        print(f"{path.name}: {int(mask.sum())} pixels, max delta {int(delta.max())}, boxes {boxes(mask)}")
        marked = a.copy()
        marked[mask] = [255, 0, 0]
        Image.fromarray(marked.astype(np.uint8)).save(out / path.name)
    print(f"{same} of {len(list(before.glob('*.png')))} identical")


if __name__ == "__main__":
    main()
