"""Sentence punctuation scored at aligned normalized-word boundaries, not just in sequence."""
import re
from collections import defaultdict

from summarize import distance, normalize_english


def marks_by_boundary(text):
    marks = defaultdict(list)
    for match in re.finditer(r'[.,!?;:]', text):
        index = match.start()
        before = text[index - 1] if index else ''
        after = text[index + 1] if index + 1 < len(text) else ''
        # Dots inside names/decimals, numeric grouping and clock separators are lexical formatting.
        if match[0] == '.' and before.isalnum() and after.isalnum():
            continue
        if match[0] in ',:' and before.isdigit() and after.isdigit():
            continue
        boundary = len(normalize_english(text[:index]).split())
        marks[boundary].append(match[0])
    return marks


def boundary_mapping(reference, hypothesis):
    """Map hypothesis gaps to reference gaps using a deterministic minimum-edit word alignment."""
    n, m = len(reference), len(hypothesis)
    costs = [[0] * (m + 1) for _ in range(n + 1)]
    for i in range(n + 1):
        costs[i][0] = i
    for j in range(m + 1):
        costs[0][j] = j
    for i in range(1, n + 1):
        for j in range(1, m + 1):
            costs[i][j] = min(costs[i - 1][j - 1] + (reference[i - 1] != hypothesis[j - 1]),
                              costs[i - 1][j] + 1, costs[i][j - 1] + 1)
    i, j, steps = n, m, []
    while i or j:
        if i and j and costs[i][j] == costs[i - 1][j - 1] + (reference[i - 1] != hypothesis[j - 1]):
            steps.append((1, 1))
            i, j = i - 1, j - 1
        elif i and costs[i][j] == costs[i - 1][j] + 1:
            steps.append((1, 0))
            i -= 1
        else:
            steps.append((0, 1))
            j -= 1
    i = j = 0
    mapping = {0: 0}
    for di, dj in reversed(steps):
        i, j = i + di, j + dj
        mapping[j] = i
    return mapping


def score(reference, hypothesis):
    expected = marks_by_boundary(reference)
    actual = marks_by_boundary(hypothesis)
    mapping = boundary_mapping(normalize_english(reference).split(), normalize_english(hypothesis).split())
    aligned = defaultdict(list)
    for boundary, marks in actual.items():
        # Prefix normalization can differ from whole-string normalization; clamp to its final gap.
        aligned[mapping[min(boundary, max(mapping))]].extend(marks)
    errors = sum(distance(expected[k], aligned[k]) for k in set(expected) | set(aligned))
    return {'errors': errors, 'reference_marks': sum(map(len, expected.values()))}
