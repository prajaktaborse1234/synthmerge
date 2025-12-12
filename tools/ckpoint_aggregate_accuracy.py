#!/usr/bin/env python3
#
# SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
# Copyright (C) 2025  Red Hat, Inc.

import csv
import sys
import re
from typing import List, Dict, Tuple


def read_csv(filename: str, model_filter: re.Pattern) -> List[dict]:
    """Read CSV file and return list of dictionaries,
    filtering by model name regex"""
    data = []
    with open(filename, "r", newline="", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        for row in reader:
            if model_filter.match(row["model"]):
                data.append(row)
    return data


def calculate_aggregate_stats(rows: List[dict]) -> Tuple[float, float, float]:
    """Calculate aggregate accuracy, accuracy_aligned and
    accuracy_stripped from list of rows"""
    # Group rows by entry_index
    entries: Dict[str, List[dict]] = {}
    for row in rows:
        entry_index = row["entry_index"]
        if entry_index not in entries:
            entries[entry_index] = []
        entries[entry_index].append(row)

    total = len(entries)
    if total == 0:
        return 0.0, 0.0, 0.0

    correct = sum(
        1
        for entry_rows in entries.values()
        if any(row["correct"] == "true" for row in entry_rows)
    )
    correct_aligned = sum(
        1
        for entry_rows in entries.values()
        if any(row["correct_aligned"] == "true" for row in entry_rows)
    )
    correct_stripped = sum(
        1
        for entry_rows in entries.values()
        if any(row["correct_stripped"] == "true" for row in entry_rows)
    )

    accuracy = correct / total
    accuracy_aligned = correct_aligned / total
    accuracy_stripped = correct_stripped / total

    return accuracy, accuracy_aligned, accuracy_stripped


def main():
    if len(sys.argv) < 3:
        print(
            "Usage: python aggregate_stats.py <model_regex> "
            "<file1.csv> [file2.csv] ..."
        )
        sys.exit(1)

    # Get model regex from first argument
    model_regex = sys.argv[1]
    try:
        model_filter = re.compile(model_regex)
    except re.error as e:
        print(f"Invalid regex: {e}", file=sys.stderr)
        sys.exit(1)

    # Increase CSV field size limit
    csv.field_size_limit(sys.maxsize)

    all_rows = []
    for filename in sys.argv[2:]:
        rows = read_csv(filename, model_filter)
        all_rows.extend(rows)

    accuracy, accuracy_aligned, \
        accuracy_stripped = calculate_aggregate_stats(all_rows)

    # Print results to stdout as CSV
    writer = csv.writer(sys.stdout)
    writer.writerow(["accuracy", "accuracy_aligned", "accuracy_stripped"])
    writer.writerow(
        [
            f"{accuracy*100:.6f}",
            f"{accuracy_aligned*100:.6f}",
            f"{accuracy_stripped*100:.6f}",
        ]
    )

    print(f"Total entries: {len(all_rows)}", file=sys.stderr)


if __name__ == "__main__":
    main()
