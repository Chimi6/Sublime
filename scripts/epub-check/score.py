#!/usr/bin/env python3
"""Scores conversions against a reference: one line per file.

    score.py NAME REF OURS PANDOC

Each argument is a file epubtext.py reads (EPUB or Word). Recall is the
share of the reference's words (as a multiset) a conversion keeps;
precision the share of its words that are the reference's. Counts are
notes, pictures, tables, and headings, reference first. An empty or missing
conversion scores zero.
"""

import collections
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import epubtext  # noqa: E402

WORD = re.compile(r"\w+", re.UNICODE)


def words(text):
    return collections.Counter(word.lower() for word in WORD.findall(text))


def overlap(ref, got):
    common = sum((ref & got).values())
    recall = common / max(1, sum(ref.values()))
    precision = common / max(1, sum(got.values()))
    return recall, precision


def main():
    name, ref, *candidates = sys.argv[1:]
    ref_words = words(epubtext.text(ref))
    ref_counts = epubtext.counts(ref)
    parts = [name]
    for label, path in zip(("ours", "pandoc"), candidates):
        if not Path(path).exists() or Path(path).stat().st_size == 0:
            parts.append(f"{label} recall=0.000 precision=0.000 counts=missing")
            continue
        recall, precision = overlap(ref_words, words(epubtext.text(path)))
        got = epubtext.counts(path)
        shown = ",".join(f"{got[k]}/{ref_counts[k]}" for k in ("notes", "images", "tables", "headings"))
        parts.append(f"{label} recall={recall:.3f} precision={precision:.3f} counts={shown}")
    print(" ".join(parts))


if __name__ == "__main__":
    main()
