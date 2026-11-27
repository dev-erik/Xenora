#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Render the Xbox scope map from explicit statuses in the master document."""
import argparse
from collections import Counter
from html import escape
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
TARGET = ROOT / 'docs/assets/xbox-compatibility-map.svg'
COLORS = {'Foundation': '#69baff', 'Contract gap': '#efb666', 'Open': '#a0aab9'}
GROUPS = [
    ('01', 'CPU & launch', 'Guest execution and target profiles', 1, 11),
    ('02', 'Graphics boundary & resources', 'Xbox interfaces and GPU ownership', 12, 23),
    ('03', 'Shader translation', 'Instruction behavior and execution state', 24, 32),
    ('04', 'Rendering & Metal', 'Pipeline features and host mappings', 33, 40),
    ('05', 'Runtime & platform', 'Services required around the renderer', 41, 52),
    ('06', 'Integration & acceptance', 'Evidence that the whole system works', 53, 58),
]


def generate():
    rows = {}
    for line in (ROOT / 'docs/xbox-compatibility.md').read_text().splitlines():
        if not line.startswith('| XBOX-'):
            continue
        columns = [c.strip() for c in line.strip('|').split('|')]
        if len(columns) != 5:
            raise ValueError(f'Expected five columns: {line}')
        ident, area, requirements, basis, status = columns
        number = int(ident.removeprefix('XBOX-'))
        if number in rows or status not in COLORS:
            raise ValueError(f'Duplicate ID or unknown map status: {line}')
        rows[number] = (ident, area, requirements, basis, status)
    expected = {n for _, _, _, lo, hi in GROUPS for n in range(lo, hi+1)}
    if set(rows) != expected:
        raise ValueError('Feature IDs changed: update visual grouping to cover the complete inventory')
    counts = Counter(row[4] for row in rows.values())
    svg = ['<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="1200" viewBox="0 0 1200 1200" role="img" aria-labelledby="title desc">',
           '<title id="title">Xbox Series X and S compatibility on Apple Silicon and Metal</title>',
           '<desc id="desc">All 58 tracked areas grouped into six subsystems. Foundation means bounded synthetic work; contract gap means unresolved Xbox requirements; open means no full acceptance. No area is certified complete for Xbox compatibility. The master document contains the accessible table.</desc>',
           '<rect width="1200" height="1200" fill="#0b111b"/>',
           '<g font-family="system-ui,-apple-system,BlinkMacSystemFont,sans-serif">',
           '<text x="26" y="32" fill="#69baff" font-size="12" font-weight="700" letter-spacing="2">XENORA / SYSTEM COMPATIBILITY</text>',
           '<text x="24" y="72" fill="#f2f6fc" font-size="32" font-weight="700">Xbox Series X|S → Apple Silicon + Metal</text>',
           f'<text x="26" y="100" fill="#aab8cc" font-size="15">{len(rows)} tracked areas · comprehensive target · no full Xbox acceptance claimed</text>']
    for x, status, phrase in [(26,'Foundation','bounded work'),(410,'Contract gap','unresolved contract'),(830,'Open','acceptance outstanding')]:
        svg += [f'<circle cx="{x+5}" cy="125" r="5" fill="{COLORS[status]}"/>',
                f'<text x="{x+18}" y="130" fill="{COLORS[status]}" font-size="13">{status} {counts[status]} · {phrase}</text>']
    for i, (index, title, subtitle, lo, hi) in enumerate(GROUPS):
        x, y = 24+(i%2)*588, 152+(i//2)*326
        svg += [f'<rect x="{x}" y="{y}" width="564" height="308" rx="12" fill="#111c2b" stroke="#273548"/>',
                f'<text x="{x+18}" y="{y+28}" fill="#657d9a" font-size="14" font-weight="700">{index}</text>',
                f'<text x="{x+51}" y="{y+28}" fill="#eef4fc" font-size="19" font-weight="650">{escape(title)}</text>',
                f'<text x="{x+18}" y="{y+49}" fill="#91a5bf" font-size="12">{escape(subtitle)}</text>']
        for offset, number in enumerate(range(lo, hi+1)):
            ident, area, requirements, basis, status = rows[number]
            baseline = y+76+offset*19
            details = re.sub(r'\[([^\]]+)\]\([^)]+\)',r'\1',f'{ident}: {area}. {status}. {basis}. Required: {requirements}')
            svg += [f'<g data-id="{ident}"><title>{escape(details)}</title>',
                    f'<text x="{x+18}" y="{baseline}" fill="#6f86a2" font-size="11">{number:02}</text>',
                    f'<text x="{x+44}" y="{baseline}" fill="#dce6f4" font-size="13">{escape(area)}</text>',
                    f'<text x="{x+546}" y="{baseline}" text-anchor="end" fill="{COLORS[status]}" font-size="11">{escape(status)}</text>', '</g>']
    svg += ['<text x="26" y="1147" fill="#dce6f4" font-size="14">Foundation ≠ console compatibility. Every area still needs Xbox-specific acceptance.</text>',
            '<text x="26" y="1172" fill="#91a5bf" font-size="12">Source: docs/xbox-compatibility.md · Desktop RDNA2 instruction coverage is a separate, narrower inventory.</text>',
            '</g>', '</svg>']
    return '\n'.join(svg)+'\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    content = generate()
    if args.check:
        if not TARGET.exists() or TARGET.read_text() != content:
            print('Stale or missing: docs/assets/xbox-compatibility-map.svg')
            return 1
    else:
        TARGET.write_text(content)
        print('Wrote docs/assets/xbox-compatibility-map.svg')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
