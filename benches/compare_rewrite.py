#!/usr/bin/env python3
"""Compare two release builds of the shipping APIs, with alternating order.

Build rewrite_bench at the baseline commit, preserve its executable, then build
it at the candidate commit. Both executables must use the same Cargo.lock,
compiler and release profile. Timings exclude fixture/reset and output capture.
"""
import argparse
import hashlib
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
    parser.add_argument("--samples", type=int, default=31)
    parser.add_argument("--crop-min-reduction", type=float, default=10,
                        help="secondary crop target; primary grayscale target remains 25%%")
    parser.add_argument("--cases", nargs="+", default=[
        "fit-rgb.jpg", "fit-gray.jpg", "fit-rgb.png",
        "crop-rotated.png", "crop-upright.png",
    ])
    args = parser.parse_args()
    if args.samples < 21:
        parser.error("use at least 21 samples for the p95 comparison")
    rows = []
    failures = []
    with tempfile.TemporaryDirectory(prefix="manzar-compare-") as temporary:
        for case in args.cases:
            samples = {"baseline": [], "candidate": []}
            hashes = {}
            # Two warmups per variant, then alternate which variant runs first.
            for iteration in range(args.samples + 2):
                variants = [("baseline", args.baseline), ("candidate", args.candidate)]
                if iteration % 2:
                    variants.reverse()
                for name, executable in variants:
                    output = pathlib.Path(temporary) / name
                    run = subprocess.run([
                        str(executable.resolve()), case, str(args.corpus.resolve()), str(output),
                    ], check=True, capture_output=True, text=True)
                    milliseconds = float(run.stdout.strip())
                    if not math.isfinite(milliseconds) or milliseconds <= 0:
                        raise ValueError("invalid elapsed time")
                    if iteration >= 2:
                        samples[name].append(milliseconds)
                    hashes[name] = hashlib.sha256(output.read_bytes()).hexdigest()
                    if name == "baseline":
                        reference = output.read_bytes()
                    else:
                        candidate = output.read_bytes()
                # Check EVERY sample, not only the last hash. The baseline is
                # an independent frozen shipping implementation.
                if candidate != reference:
                    raise AssertionError(f"{case}: output differs from shipping baseline")
            row = {"case": case, "samples_ms": samples, "output_sha256": hashes}
            for name in samples:
                row[name] = {
                    "median_ms": statistics.median(samples[name]),
                    "p95_ms": sorted(samples[name])[math.ceil(args.samples * .95) - 1],
                }
            row["median_reduction_percent"] = 100 * (1 - row["candidate"]["median_ms"] / row["baseline"]["median_ms"])
            row["p95_reduction_percent"] = 100 * (1 - row["candidate"]["p95_ms"] / row["baseline"]["p95_ms"])
            rows.append(row)
            print(f"{case}: {row['baseline']['median_ms']:.2f} -> {row['candidate']['median_ms']:.2f} ms; "
                  f"median {row['median_reduction_percent']:.1f}%, p95 {row['p95_reduction_percent']:.1f}% reduction", flush=True)
            if row["median_reduction_percent"] < -10:
                failures.append(f"{case}: median regressed by more than 10%")
            # The pre-agreed target for the first slice is the grayscale fit.
            if case == "fit-gray.jpg" and min(row["median_reduction_percent"], row["p95_reduction_percent"]) < 25:
                failures.append("grayscale fit: requires >=25% median AND p95 reduction")
            if case == "crop-rotated.png" and row["median_reduction_percent"] < args.crop_min_reduction:
                failures.append(f"rotated crop: requires >={args.crop_min_reduction}% median reduction")
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps({
        "samples_per_variant": args.samples,
        "warmups_per_variant": 2,
        "order": "alternating paired processes, warm page cache",
        "scope": "complete Rust operations; excludes webview and fixture/reset/output capture",
        "rows": rows, "failures": failures,
    }, indent=2) + "\n")
    if failures:
        raise SystemExit("FAIL: " + "; ".join(failures))
    print("PASS: byte-identical output, performance target reached, no material median regression")


if __name__ == "__main__":
    main()
