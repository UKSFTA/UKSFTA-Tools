#!/usr/bin/env python3
"""Measure investigate precision and coverage against a labelled store.

The identity store (`.uksfta/identities.json`, schema 2) records one
decision per PBO and the top ten candidates its queries ranked. A labels
file marks the expected Workshop id for any PBO a human has verified:

    {
      "pbo": {
        "ade_core.pbo": {"id": "1891978580", "title": "ADE - Advanced Diving Environment"},
        "some_private.pbo": {"id": null}
      }
    }

Usage:
    tools/calibrate.py --template        # write a blank labels file
    tools/calibrate.py                   # report metrics for labelled PBOs

A row is a hit at rank k when the expected id sits at or before rank k in
the stored candidates. Precision at 1, 3 and 10 is measured over every
verified row whatever its stored status, so an ambiguous row that ranks
the right id second still counts at 3. The exit code is 1 only when a
confirmed row names the wrong id.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path

DEFAULT_STORE = Path(".uksfta/identities.json")
DEFAULT_LABELS = Path(".uksfta/labels.json")
RANKS = (1, 3, 10)


def load_json(path: Path) -> dict:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def template(store: dict, path: Path) -> int:
    rows = {pbo: {"id": None, "title": None} for pbo in sorted(store.get("pbo", {}))}
    path.write_text(
        json.dumps({"pbo": rows}, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(f"wrote {len(rows)} blank labels to {path}")
    return 0


def candidate_ids(record: dict) -> list[str]:
    return [candidate.get("id", "") for candidate in record.get("candidates", [])]


def hit_rank(ids: list[str], expected: str) -> int | None:
    """The first rank holding the expected id, or None when it is absent."""
    for rank, candidate_id in enumerate(ids, start=1):
        if candidate_id == expected:
            return rank
    return None


def precision(ranks: list[int | None], k: int) -> float:
    if not ranks:
        return 0.0
    hits = sum(1 for rank in ranks if rank is not None and rank <= k)
    return hits / len(ranks)


def rank_table(rows: list[tuple[str, int | None]]) -> str:
    groups: dict[str, list[int | None]] = {}
    for status, rank in rows:
        groups.setdefault(status, []).append(rank)
    groups["all"] = [rank for _, rank in rows]

    lines = [f"{'status':<12}{'rows':>6}" + "".join(f"{f'p@{k}':>8}" for k in RANKS)]
    for status in sorted(groups, key=lambda name: (name == "all", name)):
        ranks = groups[status]
        cells = "".join(f"{precision(ranks, k):>8.3f}" for k in RANKS)
        lines.append(f"{status:<12}{len(ranks):>6}{cells}")
    return "\n".join(lines)


def review(store: dict, labels: dict) -> int:
    """Print the rows a human should verify, with their top candidates.

    Fill the expected id in `.uksfta/labels.json` for the rows you can
    check. Only verified labels count toward precision.
    """
    known = labels.get("pbo", {})
    print("PBO\tstatus\tM\tlabel\t1\t2\t3")
    for pbo, record in sorted(store.get("pbo", {}).items()):
        resolution = record["resolution"]
        label = known.get(pbo, {}).get("id") or ""
        candidates = record.get("candidates", [])[:3]
        cells = [
            f"{candidate['id']}:{candidate['title'][:60]}" for candidate in candidates
        ]
        while len(cells) < 3:
            cells.append("")
        print(
            "\t".join(
                [
                    pbo,
                    resolution["status"],
                    f"{resolution['m']:.2f}",
                    label,
                    *cells,
                ]
            )
        )
    print(
        f"\n{len(store.get('pbo', {}))} rows. Put a verified id in .uksfta/labels.json, "
        "then run without --review."
    )
    return 0


def summarise(store: dict, labels: dict) -> int:
    pbo_records = store.get("pbo", {})
    expected = labels.get("pbo", {})
    if not expected:
        print("no labels: run --template and fill in the ids you verified")
        return 0

    statuses = Counter(
        record["resolution"]["status"] for record in pbo_records.values()
    )
    verified = [
        (pbo, expected[pbo])
        for pbo in sorted(expected)
        if pbo in pbo_records and expected[pbo].get("id")
    ]
    blank = [
        pbo
        for pbo, label in expected.items()
        if pbo in pbo_records and not label.get("id")
    ]
    unlabelled = [pbo for pbo in pbo_records if pbo not in expected]

    rows: list[tuple[str, int | None]] = []
    confirmed = correct = wrong = 0
    wrong_rows = []
    for pbo, label in verified:
        record = pbo_records[pbo]
        resolution = record["resolution"]
        rows.append(
            (resolution["status"], hit_rank(candidate_ids(record), label["id"]))
        )
        if resolution["status"] != "confirmed":
            continue
        confirmed += 1
        if resolution.get("id") == label["id"]:
            correct += 1
        else:
            wrong += 1
            wrong_rows.append(
                (pbo, resolution.get("title"), resolution.get("id"), label["id"])
            )

    print(f"store PBOs         {len(pbo_records)}")
    print(f"verified labels    {len(verified)}")
    print(f"statuses           {dict(sorted(statuses.items()))}")
    print(f"labelled confirmed {confirmed}")
    if confirmed:
        print(f"precision          {correct / confirmed:.3f} ({correct}/{confirmed})")
        print(f"wrong              {wrong}")
    else:
        print("precision          not measurable: no labelled confirmed rows")
    print(
        f"coverage           named right {correct}, named wrong {wrong}, "
        f"not named {len(verified) - confirmed}"
    )
    print(f"unlabelled         {len(unlabelled)}")
    print(f"blank labels       {len(blank)}")

    print()
    print(rank_table(rows))

    for row in wrong_rows:
        print(f"  WRONG {row[0]}: got {row[1]!r} ({row[2]}), expected {row[3]}")

    return 0 if wrong == 0 else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--store", type=Path, default=DEFAULT_STORE)
    parser.add_argument("--labels", type=Path, default=DEFAULT_LABELS)
    parser.add_argument("--template", action="store_true")
    parser.add_argument("--review", action="store_true")
    args = parser.parse_args()

    if not args.store.exists():
        print(f"store not found: {args.store}", file=sys.stderr)
        return 2
    store = load_json(args.store)

    if args.template:
        return template(store, args.labels)
    if args.review:
        labels = load_json(args.labels) if args.labels.exists() else {"pbo": {}}
        return review(store, labels)
    if not args.labels.exists():
        print(f"labels not found: {args.labels}; run --template", file=sys.stderr)
        return 2
    return summarise(store, load_json(args.labels))


if __name__ == "__main__":
    raise SystemExit(main())
