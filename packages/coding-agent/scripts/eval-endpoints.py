#!/usr/bin/env python3
"""Score the endpoints embedder-strategy.md pre-registered, from run records.

Deliberately dumb: it reads the committed per-query numbers and applies the
tests named in the design note. No new metric is invented here, and no slice is
taken that was not declared in advance.

  python3 scripts/eval-endpoints.py "auto +rr" runs/a0-minilm.json runs/a1-bge-small.json ...
"""
import json
import sys
from itertools import combinations
from math import comb

CONFIG = sys.argv[1]
PATHS = sys.argv[2:]

# Declared in the design note before any arm ran.
SEMANTIC = {"conceptual", "cross-file", "boundary"}
GUARDRAIL = {"exact-symbol", "error-fragment"}


def load(path):
    d = json.load(open(path))
    rows = {}
    for q in d["perQuery"]:
        for r in q["results"]:
            if r["label"] == CONFIG:
                rows[q["id"]] = (q["class"], r)
    return d["provenance"]["embedder"].get("modelId", "?"), rows


def sign_test(pairs):
    """Two-sided paired sign test, ties discarded — the note's chosen test."""
    b = sum(1 for x, y in pairs if y > x)
    w = sum(1 for x, y in pairs if y < x)
    n = b + w
    if n == 0:
        return b, w, 1.0
    k = min(b, w)
    tail = sum(comb(n, i) for i in range(k + 1)) / (2 ** n)
    return b, w, min(1.0, 2 * tail)


def mean(xs):
    return sum(xs) / len(xs) if xs else 0.0


runs = [(p,) + load(p) for p in PATHS]

print(f'config: "{CONFIG}"\n')
print(f"{'arm':<28} {'model_id':<34} {'MRR':>6} {'R@10':>6} {'R@50':>6}  {'sem MRR':>8} {'sem R@10':>9}")
for path, model, rows in runs:
    sem = [r for c, r in rows.values() if c in SEMANTIC]
    print(
        f"{path.split('/')[-1]:<28} {model:<34} "
        f"{mean([r['mrr'] for _, r in rows.values()]):>6.3f} "
        f"{mean([r['recallAt10'] for _, r in rows.values()]):>6.3f} "
        f"{mean([r['recallAt50'] for _, r in rows.values()]):>6.3f}  "
        f"{mean([r['mrr'] for r in sem]):>8.3f} {mean([r['recallAt10'] for r in sem]):>9.3f}"
    )

for (pa, ma, ra), (pb, mb, rb) in combinations(runs, 2):
    ids = sorted(set(ra) & set(rb))
    print(f"\n{'=' * 78}\n{pa.split('/')[-1]}  ->  {pb.split('/')[-1]}   (n={len(ids)})\n{'=' * 78}")

    for name, subset in (
        ("PRIMARY   all queries, MRR", ids),
        ("SECONDARY semantic subgroup, MRR", [i for i in ids if ra[i][0] in SEMANTIC]),
        ("SECONDARY semantic subgroup, R@10", [i for i in ids if ra[i][0] in SEMANTIC]),
        ("REACH     all queries, R@50", ids),
    ):
        metric = "recallAt10" if name.endswith("R@10") else "recallAt50" if name.endswith("R@50") else "mrr"
        pairs = [(ra[i][1][metric], rb[i][1][metric]) for i in subset]
        b, w, p = sign_test(pairs)
        verdict = "SIGNIFICANT" if p <= 0.05 else "not significant"
        print(
            f"  {name:<36} {mean([x for x, _ in pairs]):.3f} -> {mean([y for _, y in pairs]):+.3f} "
            f"({mean([y for _, y in pairs]) - mean([x for x, _ in pairs]):+.3f})  "
            f"{b}+/{w}-  p={p:.3f}  {verdict}"
        )

    print("  GUARDRAIL (must not regress)")
    for cls in sorted(GUARDRAIL):
        subset = [i for i in ids if ra[i][0] == cls]
        for metric, label in (("recallAt10", "R@10"), ("mrr", "MRR")):
            before = mean([ra[i][1][metric] for i in subset])
            after = mean([rb[i][1][metric] for i in subset])
            flag = "  <-- REGRESSION" if after < before - 1e-9 else ""
            print(f"    {cls:<16} {label:<5} {before:.3f} -> {after:.3f} ({after - before:+.3f}){flag}")

    print("  Descriptive (underpowered by design — reported, not tested)")
    for cls in sorted({c for c, _ in ra.values()}):
        subset = [i for i in ids if ra[i][0] == cls]
        before = mean([ra[i][1]["mrr"] for i in subset])
        after = mean([rb[i][1]["mrr"] for i in subset])
        print(f"    {cls:<16} n={len(subset):<3} MRR {before:.3f} -> {after:.3f} ({after - before:+.3f})")
