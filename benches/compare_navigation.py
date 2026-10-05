#!/usr/bin/env python3
"""Compare frozen and candidate navigation_bench executables, alternating order.

Both builds use the same fixtures, Cargo.lock and release profile. This measures
the shipping image-serving API, not IPC, webview decoding or screen presentation.
The frozen baseline is expected to fail the example's frame-budget assertion.
"""
import argparse
import json
import math
import pathlib
import statistics
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=pathlib.Path)
    parser.add_argument("candidate", type=pathlib.Path)
    parser.add_argument("corpus", type=pathlib.Path)
    parser.add_argument("report", type=pathlib.Path)
    parser.add_argument("--pairs", type=int, default=21)
    args = parser.parse_args()
    reports = {"baseline": [], "candidate": []}
    with tempfile.TemporaryDirectory() as temporary:
        output = pathlib.Path(temporary) / "measurement.json"
        for iteration in range(args.pairs):
            variants = [("baseline", args.baseline), ("candidate", args.candidate)]
            if iteration % 2:
                variants.reverse()
            for name, executable in variants:
                output.unlink(missing_ok=True)
                run = subprocess.run([str(executable.resolve()), str(args.corpus.resolve()), str(output)], capture_output=True, text=True)
                if not output.exists() or (name == "candidate" and run.returncode):
                    raise RuntimeError(run.stdout + run.stderr)
                reports[name].append(json.loads(output.read_text()))
            print(f"pair {iteration + 1}/{args.pairs}", flush=True)
    summary = {}
    for name, runs in reports.items():
        samples = sorted(value for run in runs for value in run["warm_ms"])
        summary[name] = {
            "warm_median_ms": statistics.median(samples),
            "warm_p95_ms": samples[math.ceil(len(samples) * .95) - 1],
            "first_render_median_ms": statistics.median(value for run in runs for value in run["cold_ms"]),
        }
    reduction = 100 * (1 - summary["candidate"]["warm_p95_ms"] / summary["baseline"]["warm_p95_ms"])
    args.report.write_text(json.dumps({
        "scope": "six-image working set, 12MP RGB JPEG/PNG at 1280x768; Rust serving only",
        "order": "alternating paired release processes, warm filesystem page cache",
        "summary": summary, "p95_reduction_percent": reduction, "runs": reports,
    }, indent=2) + "\n")
    assert reduction >= 50, f"requires >=50% p95 reduction, got {reduction:.1f}%"
    assert summary["candidate"]["warm_p95_ms"] < 25
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
