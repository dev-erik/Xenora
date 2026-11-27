#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later

"""Render the checked-in status tables and progress graphic."""

import argparse
from html import escape
from pathlib import Path
import sys
import tomllib


def table(headers, rows):
    def line(cells):
        return "| " + " | ".join(str(cell).replace("|", r"\|") for cell in cells) + " |"

    return "\n".join([line(headers), line(["---"] * len(headers)), *(line(row) for row in rows)])


def replace_block(path, name, body, check):
    original = path.read_text()
    start = f"<!-- status:{name}:start -->\n"
    stop = f"\n<!-- status:{name}:end -->"
    head, _, tail = original.partition(start)
    if not tail or stop not in tail:
        raise ValueError(f"{path}: missing status markers")
    updated = head + start + body + stop + tail.split(stop, 1)[1]
    if updated == original:
        return True
    if check:
        print(f"{path}: status table differs from docs/status.toml", file=sys.stderr)
        return False
    path.write_text(updated)
    return True


def render_progress(status):
    forms = status["instruction"]
    milestones = status["guest_milestones"]
    verified_gpu = {"fixture verified", "fixture verified on Apple M5"}
    local_gpu = "local Apple M5 readback passed; acceptance pending"
    if not forms or len({item["form"] for item in forms}) != len(forms):
        raise ValueError("expected a nonempty inventory of unique instruction forms")
    for item in forms:
        if any(type(item[key]) is not bool for key in ("recognized", "decoded")):
            raise ValueError(f"{item['form']}: expected boolean decoder status")
        if item["decoded"] and not item["recognized"]:
            raise ValueError(f"{item['form']}: decoded form must be recognized")
        if item["gpu"] in verified_gpu and (
            not item["decoded"] or not item["lowering"] or item["cpu"] != "fixture verified"
        ):
            raise ValueError(f"{item['form']}: GPU verification requires a verified CPU path")
        if item["gpu"] == local_gpu and (
            not item["decoded"] or not item["lowering"] or item["cpu"] != "local known-answer test passed"
        ):
            raise ValueError(f"{item['form']}: local GPU evidence requires the local CPU result")
        if item["gpu"] not in verified_gpu | {"acceptance pending", local_gpu}:
            raise ValueError(f"{item['form']}: unknown GPU evidence status")
    if set(milestones) != {"X1", "X2", "X3", "X4"} or any(
        value not in {"accepted", "pending"} for value in milestones.values()
    ):
        raise ValueError("expected acceptance status for each of X1–X4")

    rows = [
        ("Decoded gfx1030 forms", sum(item["decoded"] for item in forms), len(forms)),
        ("Accepted Metal forms", sum(item["gpu"] in verified_gpu for item in forms), len(forms)),
        ("Accepted guest milestones · X1–X4", list(milestones.values()).count("accepted"), len(milestones)),
    ]
    svg = [
        '<svg xmlns="http://www.w3.org/2000/svg" width="530" height="185" viewBox="0 0 530 185" role="img" aria-labelledby="title description">',
        '<title id="title">Xenora · recorded progress</title>',
        '<desc id="description">' + escape(". ".join(f"{label}: {count} of {total}" for label, count, total in rows)) + '</desc>',
        '<rect x="1" y="1" width="528" height="183" rx="10" fill="#18212a" stroke="#516171"/>',
        '<g font-family="system-ui,sans-serif" fill="#eaf0f5">',
        '<text x="20" y="29" font-size="13" font-weight="700">RECORDED FIXTURE PROGRESS</text>',
    ]
    for y, (label, count, total) in zip((58, 96, 134), rows, strict=True):
        svg.extend([
            f'<text x="20" y="{y}" font-size="13">{escape(label)}</text>',
            f'<text x="510" y="{y}" font-size="13" text-anchor="end">{count} / {total}</text>',
            f'<rect x="20" y="{y + 7}" width="490" height="4" rx="2" fill="#3c4a58"/>',
            f'<rect x="20" y="{y + 7}" width="{490 * count / total:.2f}" height="4" rx="2" fill="#88c6bd"/>',
        ])
    svg.extend([
        '<text x="20" y="168" font-size="11" fill="#b4c2d0">Scope: tracked instruction forms and four synthetic guest milestones.</text>',
        '</g>',
        '</svg>',
    ])
    return "\n".join(svg) + "\n"


def check_asset(path, svg, check):
    if path.exists() and path.read_text() == svg:
        return True
    if check:
        print(f"{path}: progress graphic differs from docs/status.toml", file=sys.stderr)
        return False
    path.write_text(svg)
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    check = parser.parse_args().check
    root = Path(__file__).resolve().parents[2]
    docs = root / "docs"
    status = tomllib.loads((docs / "status.toml").read_text())
    claims = tomllib.loads((docs / "claims.toml").read_text())
    classes = {claim["id"]: claim["class"] for claim in claims["claim"]}

    features = table(
        ("Feature", "Target", "Implementation", "Correctness", "Hardware", "Evidence", "Limits"),
        [
            (
                feature["id"],
                feature["architecture"],
                feature["implementation"],
                feature["correctness"],
                feature["hardware_verification"] + (
                    f" on {feature['hardware_target']}" if "hardware_target" in feature else ""
                ),
                ", ".join(sorted({classes[claim] for claim in feature["claims"]})),
                feature["limitations"],
            )
            for feature in status["feature"]
        ],
    )
    instructions = table(
        ("Form", "Operands", "Size or word", "Recognized", "Decoded", "LLVM",
         "Lowering", "Reference", "CPU", "GPU"),
        [
            (
                f"`{item['form']}`",
                item["encoding"],
                item["representation"],
                "yes" if item["recognized"] else "no",
                "yes" if item["decoded"] else "no",
                item["llvm"],
                item["lowering"],
                item["reference"],
                item["cpu"],
                item["gpu"],
            )
            for item in status["instruction"]
        ],
    )
    svg = render_progress(status)
    checks = [
        replace_block(docs / "progress.md", "features", features, check),
        check_asset(docs / "assets/xenora-progress.svg", svg, check),
        replace_block(docs / "shaders/instruction-coverage.md", "instructions", instructions, check),
    ]
    return 0 if all(checks) else 1


if __name__ == "__main__":
    sys.exit(main())
