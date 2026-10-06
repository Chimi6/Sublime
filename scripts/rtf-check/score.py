#!/usr/bin/env python3
"""score.py <pdf dir> <name> <reference> <kind>...: page counts of each kind's
PDF and the share of the reference's words each kind's text carries."""
import collections, os, re, sys

base, name, reference, *kinds = sys.argv[1:]


def pages(kind):
    path = f"{base}/{kind}/{name}.pdf"
    if not os.path.exists(path):
        return 0
    return len(re.findall(rb"/Type\s*/Page[^s]", open(path, "rb").read()))


def words(kind):
    path = f"{base}/{kind}/{name}.txt"
    if not os.path.exists(path):
        return collections.Counter()
    text = open(path, encoding="utf-8", errors="replace").read()
    return collections.Counter(word.lower() for word in re.findall(r"\w+", text))


truth = words(reference)
total = sum(truth.values()) or 1
page_part = " ".join(f"{kind}={pages(kind)}" for kind in [reference, *kinds])
recall_part = " ".join(f"{kind}={sum((truth & words(kind)).values()) / total:.3f}" for kind in kinds)
print(f"{name} pages {page_part} recall {recall_part}")
