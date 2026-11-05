#!/usr/bin/env python3
"""Interactive macOS bundle measurements; persist numeric allowlisted data only."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import plistlib
import re
import statistics
import subprocess
import threading


PR4 = {
    "source": "docs/CORE_OPTIMIZATION_0.4.md (PR #4)",
    "offline_proxy_median_ms": 58.63,
    "offline_proxy_p95_ms": 97.96,
    "installed_030_total_mem_mib_approx": 174,
    "installed_030_total_cpu_percent": 0.0,
    "installed_030_total_power_indicator": 0.0,
    "live_latency": None,
    "energy": None,
}


def number(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def timing(line):
    """Return only an enum and numeric value; never propagate source text."""
    if "pipeline_timing " not in line:
        return None
    try:
        row = json.loads(line.split("pipeline_timing ", 1)[1])
        if not isinstance(row, dict) or row.get("version") != 1:
            return "invalid", None
        if row.get("outcome") != "inserted":
            return "not_inserted", None
        stages = row.get("stages_ms")
        value = stages.get("release_to_inserted") if isinstance(stages, dict) else None
        return ("inserted", value) if number(value) else ("invalid", None)
    except (ValueError, TypeError):
        return "invalid", None


def stats(values):
    if not values:
        return None
    return {"n": len(values), "median_ms": statistics.median(values),
            "p95_ms": sorted(values)[math.ceil(len(values) * .95) - 1]}


def top_rows(text, pids):
    rows = {pid: [] for pid in pids}
    for line in text.splitlines():
        fields = line.split()
        if len(fields) != 4 or not fields[0].isdigit():
            continue
        pid = int(fields[0])
        if pid not in rows:
            continue
        mem = re.fullmatch(r"([0-9.]+)([BKMGT])[-+]?", fields[2])
        try:
            cpu, power = float(fields[1]), float(fields[3])
            if mem and number(cpu) and number(power):
                mib = float(mem[1]) * 1024 ** ("BKMGT".index(mem[2]) - 2)
                if number(mib):
                    rows[pid].append({"cpu_percent": cpu, "mem_mib": mib,
                                      "power_indicator": power})
        except ValueError:
            continue
    # top's first sample is not an interval measurement.
    return {pid: values[1:] for pid, values in rows.items()}


def energy_rows(text, expected=None):
    result = {}
    for component in ("CPU", "GPU", "ANE"):
        values = [float(v) for v in re.findall(
            rf"^{component} Power: ([0-9.]+) mW\s*$", text, re.MULTILINE)]
        if values and (expected is None or len(values) == expected) and all(number(v) for v in values):
            result[component.lower() + "_mean_mw"] = statistics.mean(values)
    return result


def run(command, timeout):
    try:
        result = subprocess.run(command, capture_output=True, text=True,
                                errors="replace", timeout=timeout, check=False)
        return result.stdout if result.returncode == 0 else ""
    except (OSError, subprocess.TimeoutExpired):
        return ""


def child_pid(parent):
    text = run(["/bin/ps", "-axo", "pid=,ppid=,comm="], 5)
    found = []
    for line in text.splitlines():
        fields = line.split(None, 2)
        if len(fields) == 3 and fields[1] == str(parent):
            if "parakeet" in Path(fields[2]).name.lower():
                found.append(int(fields[0]))
    if len(found) != 1:
        raise ValueError("Expected exactly one direct Parakeet helper; warm the default engine first.")
    return found[0]


def summary(report, baseline=None):
    lines = []
    failed = False
    incomplete = False
    latency = report["latency"]
    if report["invalid"] or report["not_inserted"]:
        lines.append("FAIL: invalid or unsuccessful pipeline events in the dictation window.")
        failed = True
    if not latency or latency["n"] < report["minimum_samples"]:
        lines.append("INCOMPLETE: too few successful live insertions.")
        incomplete = True
    elif baseline is None:
        lines.append("INCOMPLETE: PR #4 has no live latency baseline (offline proxy is not comparable).")
        incomplete = True
    else:
        before = baseline.get("latency")
        if not isinstance(before, dict) or baseline.get("invalid", 0) or baseline.get("not_inserted", 0) or not all(number(before.get(k)) for k in ("median_ms", "p95_ms", "n")) or before["n"] < report["minimum_samples"]:
            lines.append("INCOMPLETE: supplied live baseline has insufficient timings.")
            incomplete = True
        else:
            passed = all(latency[k] <= before[k] for k in ("median_ms", "p95_ms"))
            lines.append(f"{'PASS' if passed else 'FAIL'}: live median/p95 "
                         f"{latency['median_ms']:.3f}/{latency['p95_ms']:.3f} ms vs "
                         f"{before['median_ms']:.3f}/{before['p95_ms']:.3f} ms supplied matched baseline.")
            failed |= not passed
    resources = report["idle"]
    if not resources:
        lines.append("INCOMPLETE: app/helper idle interval unavailable or processes changed.")
        incomplete = True
    else:
        for key, limit in (("mem_mib", 174), ("cpu_percent", 0), ("power_indicator", 0)):
            passed = resources["total_mean"][key] <= limit
            lines.append(f"{'PASS' if passed else 'FAIL'} reference only: {key} "
                         f"{resources['total_mean'][key]:.3f} vs PR #4 installed 0.3.0 {limit}.")
        prior = baseline.get("idle") if baseline else None
        if not isinstance(prior, dict) or not isinstance(prior.get("total_mean"), dict) or not all(
                number(prior["total_mean"].get(k)) for k in resources["total_mean"]):
            lines.append("INCOMPLETE: no matched live resource baseline; PR #4 references are not a gate.")
            incomplete = True
        else:
            passed = all(resources["total_mean"][k] <= prior["total_mean"][k]
                         for k in resources["total_mean"])
            lines.append(f"{'PASS' if passed else 'FAIL'}: idle resources versus supplied matched baseline.")
            failed |= not passed
    lines.append("CAPTURED: system CPU/GPU/ANE estimated power; not app joules."
                 if report["energy"] else "INCOMPLETE: privileged system power unavailable.")
    prior_energy = baseline.get("energy") if baseline else None
    if report["energy"] and isinstance(prior_energy, dict) and all(
            number(prior_energy.get(k)) for k in report["energy"]):
        passed = all(v <= prior_energy[k] for k, v in report["energy"].items())
        lines.append(f"{'PASS' if passed else 'FAIL'}: system power versus supplied matched baseline.")
        failed |= not passed
    else:
        lines.append("INCOMPLETE: no comparable energy pair (PR #4 has no privileged energy baseline).")
        incomplete = True
    incomplete |= not bool(report["energy"])
    status = "FAIL" if failed else "INCOMPLETE" if incomplete else "PASS"
    return status, lines


def capture(args):
    bundle = args.app.resolve()
    with (bundle / "Contents/Info.plist").open("rb") as stream:
        info = plistlib.load(stream)
    executable = (bundle / "Contents/MacOS" / info["CFBundleExecutable"]).resolve()
    if bundle not in executable.parents or not executable.is_file():
        raise ValueError("Invalid bundle executable.")
    baseline = json.loads(args.baseline.read_text()) if args.baseline else None
    if baseline is not None and (not isinstance(baseline, dict) or baseline.get("schema") != 1):
        raise ValueError("Expected a release-check report baseline.")
    if os.environ.get("GRETCHEN_PARAKEET_HELPER"):
        raise ValueError("Unset the helper override to measure the bundled helper.")
    # Existing app instances would contaminate shortcut/delivery measurements.
    existing = run(["/bin/ps", "-axo", "comm="], 5)
    if any(Path(line.strip()).name == executable.name for line in existing.splitlines()):
        raise ValueError("Quit existing instances of this app before running the check.")
    report = {"schema": 1, "pr4_reference": PR4, "minimum_samples": args.minimum_samples,
              "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
              "latency": None, "invalid": 0, "not_inserted": 0, "idle": None,
              "energy": None, "release_to_inserted_ms": []}
    lock = threading.Lock()
    phase = "warmup"
    idle_events = 0
    env = dict(os.environ, RUST_LOG="info")
    app = subprocess.Popen([str(executable)], stdout=subprocess.DEVNULL,
                           stderr=subprocess.PIPE, env=env)

    def drain():
        nonlocal idle_events
        # Bound reads even if a dependency emits an enormous line. Discard fragments.
        fragment = False
        while True:
            data = app.stderr.readline(65536)
            if not data:
                break
            complete = data.endswith(b"\n")
            event = timing(data.decode("utf-8", "replace")) if complete and not fragment else None
            fragment = not complete
            if event:
                with lock:
                    # Enter does not cancel an in-flight dictation. Keep
                    # its eventual result while waiting to start idle sampling.
                    if phase in ("dictation", "pause"):
                        outcome, value = event
                        if outcome == "inserted":
                            report["release_to_inserted_ms"].append(value)
                        else:
                            report[outcome] += 1
                    elif phase == "idle":
                        idle_events += 1

    reader = threading.Thread(target=drain, daemon=True)
    reader.start()
    try:
        input("Wait for readiness, perform one warm-up dictation, then press Enter here. ")
        if app.poll() is not None:
            raise ValueError("Bundle exited before measurement.")
        with lock:
            phase = "dictation"
        input("Dictate into a disposable native text field. Verify each insertion visually. "
              "When all insertions finish, return here and press Enter. ")
        with lock:
            phase = "pause"
        input("Leave the app idle; stop other busy work. Press Enter to sample idle resources. ")
        helper = child_pid(app.pid)
        with lock:
            phase = "idle"
        energy = {}

        def power():
            energy.update(energy_rows(run([
                "/usr/bin/sudo", "-n", "/usr/bin/powermetrics", "--samplers",
                "cpu_power,gpu_power,ane_power", "-i", "1000", "-n", str(args.idle_seconds)],
                args.idle_seconds + 15), args.idle_seconds))

        worker = threading.Thread(target=power)
        worker.start()
        raw = run(["/usr/bin/top", "-l", str(args.idle_seconds + 1), "-s", "1",
                   "-pid", str(app.pid), "-pid", str(helper), "-stats", "pid,cpu,mem,power"],
                  args.idle_seconds + 15)
        worker.join()
        with lock:
            phase = "done"
        rows = top_rows(raw, (app.pid, helper))
        stable = app.poll() is None and child_pid(app.pid) == helper
        if stable and not idle_events and all(len(v) == args.idle_seconds for v in rows.values()):
            means = {role: {key: statistics.mean(r[key] for r in rows[pid])
                            for key in ("mem_mib", "cpu_percent", "power_indicator")}
                     for role, pid in (("app", app.pid), ("helper", helper))}
            report["idle"] = {"samples": args.idle_seconds, "interval_seconds": 1,
                              "means": means, "samples_by_role": {
                                  "app": rows[app.pid], "helper": rows[helper]},
                              "total_mean": {k: means["app"][k] + means["helper"][k]
                                             for k in means["app"]}}
        report["energy"] = energy if stable and not idle_events and len(energy) == 3 else None
        report["latency"] = stats(report["release_to_inserted_ms"])
        status, lines = summary(report, baseline)
        report["status"] = status
        report["summary"] = lines
        # Exclusive creation avoids destroying prior evidence; no raw logs are written.
        with os.fdopen(os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600),
                       "w", encoding="utf-8") as stream:
            json.dump(report, stream, indent=2, allow_nan=False)
            stream.write("\n")
        print(status + "\n" + "\n".join(lines))
        if report["latency"]:
            print("Live release-to-inserted:", json.dumps(report["latency"]))
        return {"PASS": 0, "FAIL": 1, "INCOMPLETE": 2}[status]
    finally:
        if app.poll() is None:
            app.terminate()
            try:
                app.wait(timeout=10)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait()
        reader.join(timeout=2)
        app.stderr.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, help="Matched live baseline report from this tool")
    parser.add_argument("--idle-seconds", type=int, default=30)
    parser.add_argument("--minimum-samples", type=int, default=20)
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("Live collection requires macOS.")
    if args.idle_seconds < 5 or args.minimum_samples < 1:
        parser.error("Use at least 5 idle seconds and a positive minimum sample count.")
    if args.output.exists():
        parser.error("Output already exists; choose a new file.")
    try:
        return capture(args)
    except (OSError, ValueError, KeyError, TypeError, EOFError, KeyboardInterrupt):
        # Exception strings can contain untrusted log/plist/path content.
        print("Collection failed or cancelled; no complete evidence report. Check bundle, helper, and output paths.")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
