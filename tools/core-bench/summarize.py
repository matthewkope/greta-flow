"""Validate paired runs and report median / nearest-rank p95 in milliseconds."""
import json
import math
import statistics
import sys
from pathlib import Path


def stats(values):
    if not values or any(not math.isfinite(v) or v < 0 for v in values):
        raise ValueError("missing, negative or nonfinite timings")
    return {"n": len(values), "median_ms": statistics.median(values),
            "p95_ms": sorted(values)[math.ceil(len(values) * 0.95) - 1]}


def read(path, manifest, events, passes):
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    if len([r for r in rows if r["event"] == "load"]) != 1:
        raise ValueError(f"{path}: expected one load row")
    ids = [clip["id"] for clip in manifest]
    if not ids or len(ids) != len(set(ids)):
        raise ValueError("empty or duplicate manifest IDs")
    expected = {(event, clip, p) for event in events for clip in ids
                for p in range(1, passes + 1)}
    expected.add((events[0], ids[0], 0))
    result = {}
    for row in rows:
        if row["event"] == "load":
            continue
        key = row["event"], row["id"], row["pass"]
        if key in result:
            raise ValueError(f"{path}: duplicate {key}")
        result[key] = row
    if set(result) != expected:
        raise ValueError(f"{path}: incomplete or unexpected run")
    return result


def compare(before, after):
    if set(before) != set(after):
        raise ValueError("different paired clips")
    differences = sum(before[k]["text"] != after[k]["text"] for k in before)
    return {"paired_transcripts": len(before), "changed_transcripts": differences}


def summarize(directory, full_manifest):
    full = json.loads(full_manifest.read_text())
    proxy = json.loads((directory / "manifest.json").read_text())
    result = {"quantile": "nearest rank", "live_release_to_inserted": None}
    full_runs = {side: read(directory / f"{side}.jsonl", full,
                            ["batch", "segmented_offline"], 2)
                 for side in ("before", "after")}
    result["full_corpus"] = compare(full_runs["before"], full_runs["after"])
    for side, rows in full_runs.items():
        result["full_corpus"][side] = stats([
            (r["seconds"] + r["prepare_seconds"]) * 1000
            for (event, _, p), r in rows.items() if event == "batch" and p > 0])
    result["offline_release_proxy"] = {}
    combined = {"before": [], "after": []}
    for suffix in ("", "-repeat"):
        runs = {side: read(directory / f"proxy-{side}{suffix}.jsonl", proxy,
                           ["offline_sink"], 3) for side in combined}
        row = compare(runs["before"], runs["after"])
        for side, rows in runs.items():
            values = [r["stages_ms"]["offline_release_to_sink"]
                      for (_, _, p), r in rows.items() if p > 0]
            row[side] = stats(values)
            combined[side].extend(values)
        result["offline_release_proxy"][suffix or "first"] = row
    result["offline_release_proxy"]["combined"] = {
        side: stats(values) for side, values in combined.items()}
    return result


if __name__ == "__main__":
    print(json.dumps(summarize(Path(sys.argv[1]), Path(sys.argv[2])), indent=2))
