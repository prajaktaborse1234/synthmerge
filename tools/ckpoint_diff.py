#!/usr/bin/env python3
#
# SPDX-License-Identifier: GPL-3.0-or-later OR AGPL-3.0-or-later
# Copyright (C) 2025  Red Hat, Inc.

import csv
import sys
import re
from typing import List


def read_csv(filename: str) -> List[dict]:
    """Read CSV file and return list of dictionaries"""
    data = []
    with open(filename, "r", newline="", encoding="utf-8") as f:
        reader = csv.DictReader(f)
        for row in reader:
            data.append(row)
    return data


def find_differing_rows(
    rows1: List[dict], rows2: List[dict], fields: List[str]
) -> List[dict]:
    """Find rows that differ between two lists of dictionaries"""
    differing_rows = []
    # Group rows by entry_index for both lists
    from collections import defaultdict

    grouped_rows1 = defaultdict(list)
    grouped_rows2 = defaultdict(list)

    for row in rows1:
        grouped_rows1[row["entry_index"]].append(row)

    for row in rows2:
        grouped_rows2[row["entry_index"]].append(row)

    # Compare groups with same entry_index
    all_entry_indices = set(grouped_rows1.keys()) | set(grouped_rows2.keys())

    for entry_index in all_entry_indices:
        group1 = grouped_rows1.get(entry_index, [])
        group2 = grouped_rows2.get(entry_index, [])

        # If one group is empty and the other isn't, they differ
        if len(group1) != len(group2):
            # differing_rows.extend(group2)
            continue

        all_equal = True
        for group in group1, group2:
            first_row = group[0]
            for row in group[1:]:
                if not all(first_row[field] == row[field] for field in fields):
                    all_equal = False
                    break
            if not all_equal:
                break
        if not all_equal:
            continue

        # Compare each row in group1 with corresponding row in group2
        difference1 = True
        difference2 = True
        for row1, row2 in zip(group1, group2):
            # if not all(row1[field] != row2[field] for field in fields):
            if any(
                row1[field] != "false" and (1 or row2[field] == "true")
                for field in fields
            ):
                difference1 = False
                break
            if any(
                (1 or row1[field] == "false") and row2[field] != "true"
                for field in fields
            ):
                difference2 = False
                break
        if difference1 and difference2:
            differing_rows.append(row2)

    return differing_rows


def main():
    if len(sys.argv) < 5:
        print(
            "Usage: python diff_csv.py <test.csv> <file1.csv> "
            "<file2.csv> <field1> [field2] ..."
        )
        sys.exit(1)

    test_file = sys.argv[1]
    file1 = sys.argv[2]
    file2 = sys.argv[3]
    fields = sys.argv[4:]

    # Increase CSV field size limit
    csv.field_size_limit(sys.maxsize)

    rows1 = read_csv(file1)
    rows2 = read_csv(file2)
    test_rows = read_csv(test_file)

    # Get all unique field names from both files
    all_fields = set()
    for row in rows1 + rows2:
        all_fields.update(row.keys())

    # Check if specified fields exist
    missing_fields = set(fields) - all_fields
    if missing_fields:
        print(f"Error: Fields not found in CSV files: {missing_fields}")
        sys.exit(1)

    differing_rows = find_differing_rows(rows1, rows2, fields)

    count = 0
    # Print results to stdout as CSV
    if differing_rows:
        writer = csv.DictWriter(sys.stdout, fieldnames=test_rows[0].keys())
        writer.writeheader()
        for row in differing_rows:
            commit = row["patch_commit_hash"]
            for test_row in test_rows:
                desc = test_row["Description"]
                if re.search(re.escape(commit), desc):
                    writer.writerow(test_row)
                    count += 1
                    break
            else:
                writer.writerow(row)
            # if count > 10:
            #    break
    print(count, file=sys.stderr)


if __name__ == "__main__":
    main()
