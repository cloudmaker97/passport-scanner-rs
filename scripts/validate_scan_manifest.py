#!/usr/bin/env python3
"""Offline survey validation; requires existing Pillow and NumPy installations.

Example: python scripts/validate_scan_manifest.py scans/final20/manifest.json
Exit 0 means technical checks passed, not document/colour/OCR certification.
No image content, OCR, metadata or personal fields are printed or extracted.
"""

import argparse
import hashlib
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageFile


def validate(manifest_path, expected_runs=20, ir_tolerance=0, max_white_clip=None):
    manifest_path = Path(manifest_path).resolve()
    root = manifest_path.parent
    manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
    errors, artifacts, runs, seen_paths = [], [], [], set()
    ImageFile.LOAD_TRUNCATED_IMAGES = False
    required = {(stage, source) for stage in ("raw", "processed")
                for source in ("white", "infrared")}
    records = manifest.get("runs", [])
    if manifest.get("status") != "completed":
        errors.append("Manifest is not completed")
    for key in ("attempted", "capture_succeeded", "processing_succeeded", "succeeded"):
        if manifest.get(key) != expected_runs:
            errors.append(f"Manifest {key} does not equal expected runs")
    if manifest.get("failed") != 0 or len(records) != expected_runs:
        errors.append("Failed count or run record count is incorrect")
    if [r.get("run") for r in records] != list(range(1, expected_runs + 1)):
        errors.append("Run numbers are not the expected consecutive sequence")
    for record in records:
        number = record.get("run")
        if not all(record.get(k) is True for k in
                   ("capture_succeeded", "processing_succeeded", "succeeded")) or record.get("errors"):
            errors.append(f"Run {number}: capture/processing reported failure")
        keys = [(d.get("stage"), d.get("source")) for d in record.get("artifacts", [])]
        if any(keys.count(k) != 1 for k in required):
            errors.append(f"Run {number}: expected exactly one raw/processed white/IR artifact")
        for declared in record.get("artifacts", []):
            name = declared.get("path", "")
            item = {"run": number, "path": name, "stage": declared.get("stage"),
                    "source": declared.get("source"), "decode_ok": False}
            artifacts.append(item)
            try:
                path = (root / name).resolve()
                if not name or not path.is_relative_to(root) or path == manifest_path:
                    raise ValueError("invalid artifact path")
                if path in seen_paths:
                    raise ValueError("artifact path reused")
                seen_paths.add(path)
                data = path.read_bytes()
                item.update(bytes=len(data), sha256=hashlib.sha256(data).hexdigest(),
                            jpeg_soi=data.startswith(b"\xff\xd8"),
                            jpeg_eoi_found=data.rfind(b"\xff\xd9") >= 0)
                if not item["jpeg_soi"] or not item["jpeg_eoi_found"]:
                    raise ValueError("missing JPEG start/end marker")
                item["bytes_after_eoi"] = len(data) - data.rfind(b"\xff\xd9") - 2
                with Image.open(path) as im:
                    im.verify()
                with Image.open(path) as im:
                    im.load()
                    item["decoded_size"] = list(im.size)
                    pixels = np.asarray(im.convert("RGB"), dtype=np.uint16)
                item["decode_ok"] = True
                if item["bytes"] != declared.get("bytes"):
                    raise ValueError("manifest byte count mismatch")
                if item["decoded_size"] != [declared.get("frame_width"), declared.get("frame_height")]:
                    raise ValueError("manifest dimensions mismatch")
                # Accumulate in uint32 to avoid overflow at full white.
                p = pixels.astype(np.uint32)
                luma = (77 * p[:, :, 0] + 150 * p[:, :, 1] + 29 * p[:, :, 2]) >> 8
                item["mean_luma"] = float(luma.mean())
                item["mean_rgb"] = [float(v) for v in p.mean(axis=(0, 1))]
                item["all_channels_ge250_fraction"] = float((p >= 250).all(axis=2).mean())
                item["any_channel_eq255_fraction"] = float((p == 255).any(axis=2).mean())
                if item["stage"] == "processed" and item["source"] == "infrared":
                    spread = pixels.max(axis=2) - pixels.min(axis=2)
                    item["maximum_channel_spread"] = int(spread.max())
                    item["fraction_above_grayscale_tolerance"] = float((spread > ir_tolerance).mean())
                    if item["maximum_channel_spread"] > ir_tolerance:
                        raise ValueError("processed IR is not grayscale within tolerance")
                if item["stage"] == "processed" and item["source"] == "white" and max_white_clip is not None:
                    if item["all_channels_ge250_fraction"] > max_white_clip:
                        raise ValueError("white clipping exceeds explicitly selected threshold")
            except (OSError, ValueError, KeyError, TypeError) as exc:
                # Only exception class is exposed for parser/I/O failures; no content.
                reason = str(exc) if type(exc) is ValueError else type(exc).__name__
                item["error"] = reason
                errors.append(f"Run {number} {item['stage']}/{item['source']}: {reason}")
        runs.append({"run": number, "document_presence": "requires_visual_review",
                     "full_front_and_corners": "requires_visual_review",
                     "text_and_portrait_readability": "requires_visual_review",
                     "mrz_on_front_expected": False, "ocr_success": None})
    groups = {}
    for stage, source in sorted(required):
        items = [a for a in artifacts if a["stage"] == stage and a["source"] == source and "mean_luma" in a]
        if items:
            groups[f"{stage}-{source}"] = {
                "count": len(items),
                "decoded_sizes": sorted({tuple(a["decoded_size"]) for a in items}),
                "unique_hashes": len({a["sha256"] for a in items}),
                "mean_luma_range": [min(a["mean_luma"] for a in items), max(a["mean_luma"] for a in items)],
                "all_channels_ge250_fraction_range": [min(a["all_channels_ge250_fraction"] for a in items), max(a["all_channels_ge250_fraction"] for a in items)],
            }
    return {"schema_version": 1, "manifest_sha256": hashlib.sha256(manifest_path.read_bytes()).hexdigest(),
            "technical_checks_pass": not errors, "quality_pass": None,
            "expected_runs": expected_runs, "ir_channel_spread_tolerance": ir_tolerance,
            "maximum_white_clip_fraction": max_white_clip, "errors": errors,
            "limitations": ["File integrity is separate from visual document quality and colour accuracy.",
                            "Review every processed front for complete corners, readable text and portrait, glare and crop errors.",
                            "No MRZ is required on the German ID front. No OCR was performed or certified.",
                            "Clipping and brightness ranges are descriptive unless an explicit clipping threshold is supplied.",
                            "Duplicate hashes are reported for review, not automatically treated as corruption."],
            "metric_method": "Full Pillow JPEG verify and decode; luma=(77R+150G+29B)>>8. Clipping measures entire image, not a detected document or portrait ROI.",
            "groups": groups, "runs": runs, "artifacts": artifacts}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--expected-runs", type=int, default=20)
    parser.add_argument("--ir-tolerance", type=int, default=0)
    parser.add_argument("--max-white-clip", type=float, default=None)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.expected_runs < 1 or not 0 <= args.ir_tolerance <= 255 or (args.max_white_clip is not None and not 0 <= args.max_white_clip <= 1):
        parser.error("invalid run count, IR tolerance or clipping fraction")
    output = args.output or args.manifest.parent / "offline-validation.json"
    if output.resolve() == args.manifest.resolve() or output.suffix.lower() != ".json" or output.exists():
        parser.error("output must be a new JSON file, separate from the manifest")
    try:
        result = validate(args.manifest, args.expected_runs, args.ir_tolerance, args.max_white_clip)
    except (OSError, ValueError, TypeError) as exc:
        print(f"Validation unavailable: {type(exc).__name__}", file=sys.stderr)
        return 2
    output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"technical_checks_pass": result["technical_checks_pass"],
                      "files_checked": len(result["artifacts"]), "error_count": len(result["errors"]),
                      "quality_pass": None}))
    return 0 if result["technical_checks_pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
