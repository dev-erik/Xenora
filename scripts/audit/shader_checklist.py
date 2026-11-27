#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate the complete pinned RDNA2 conversion checklist and progress map."""
from __future__ import annotations

import argparse
from collections import Counter
from html import escape
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from qa.rdna2_plan import load_inventories  # noqa: E402

STATES = {
    'verified': ('Converted subset; fixture accepted', '#238636'),
    'pending': ('Converted subset; acceptance pending', '#bf8700'),
    'unrecorded': ('Decoded; translation evidence unrecorded', '#1f6feb'),
    'missing': ('Not converted', '#6e7681'),
}
ARITHMETIC_FAMILIES = set('SOP1 SOP2 SOPC SOPK VOP1 VOP2 VOP3 VOP3P VOP3SDST VOPC'.split())


def workstream(family, name):
    """Published planning partition, not a completion or compatibility contract."""
    if family not in ARITHMETIC_FAMILIES:
        return 'Other shader work / assignment pending'
    if ((name.startswith('V_') and 'CMP' in name)
        or any(t in name for t in ('SAVEEXEC', 'WREXEC', 'WQM', 'QUADMASK'))
        or name in {'V_CNDMASK_B32', 'V_ADD_CO_CI_U32', 'V_SUB_CO_CI_U32',
                    'V_SUBREV_CO_CI_U32', 'V_ADD_CO_U32', 'V_SUB_CO_U32', 'V_SUBREV_CO_U32'}):
        return 'S3.3 Masks / comparisons'
    if any(t in name for t in ('READLANE', 'WRITELANE', 'READFIRSTLANE', 'PERMLANE', 'MBCNT')):
        return 'S3.4 Wave / lane'
    if any(t in name for t in ('GETPC', 'SETPC', 'SWAPPC', 'RFE', 'CALL', 'SUBVECTOR_LOOP')):
        return 'S3.2 Control flow'
    if any(t in name for t in ('WAITCNT', 'INTERP')) or name == 'V_PIPEFLUSH':
        return 'S3.5 Memory / interfaces'
    if name in {'S_GETREG_B32', 'S_SETREG_B32', 'S_SETREG_IMM32_B32', 'S_VERSION', 'V_CLREXCP', 'V_NOP'}:
        return 'State / assignment pending'
    return 'S3.1 Arithmetic'


def cell(value):
    return str(value).replace('|', '&#124;').replace('\n', ' ')


def generate():
    data = load_inventories(ROOT)
    identified, decoder = data['identified'], data['decoder']
    if decoder - identified.keys():
        raise ValueError('Decoded forms absent from the pinned inventory')
    evidence = {}
    for row in data['status']['instruction']:
        key = data['sc']['status_form'](row, identified)
        if key in evidence:
            raise ValueError(f'Duplicate status evidence: {key}')
        evidence[key] = row
    if evidence.keys() - decoder:
        raise ValueError('Status evidence without decoder support')
    rows = []
    for (family, name), opcode in sorted(identified.items(), key=lambda x: (x[0][0], x[1], x[0][1])):
        key = family, name
        record = evidence.get(key, {})
        if key not in decoder:
            state = 'missing'
        elif not record.get('lowering'):
            state = 'unrecorded'
        elif key in data['metal']:
            state = 'verified'
        else:
            state = 'pending'
        rows.append((family, name, opcode, state, record))
    counts = Counter(r[3] for r in rows)
    md = [
        '# Complete RDNA2 shader conversion checklist', '',
        'Generated from the pinned AMD RDNA2 inventory, the handwritten and generated decoder identities, '
        f'and [recorded evidence](../status.toml). Every one of the inventory’s **{len(rows):,} encoding forms** appears below, '
        'including unsupported forms. These are shader instructions, not Rust methods or runtime API functions. '
        'The same mnemonic in two encoding families is two forms.', '',
        '![Conversion map](../assets/shader-conversion-map.svg)', '',
        '## Status meanings', '',
        '| Status | Forms | Meaning |', '| --- | ---: | --- |',
    ]
    explanations = {
        'verified': 'Translation is recorded and the status registry records accepted native fixture evidence; only the documented operand subset is covered.',
        'pending': 'Translation is recorded, but the registry does not record accepted native fixture evidence. This may reflect pending acceptance or documentation lag.',
        'unrecorded': 'The decoder lists the form, but no translation evidence row is recorded. This is an evidence gap, not proof that lowering is absent.',
        'missing': 'No supported decoder identity exists. Opcode recognition from metadata alone does not implement execution.',
    }
    for state, (label, _) in STATES.items():
        md.append(f'| {label} | {counts[state]} | {explanations[state]} |')
    md += ['', f'**Decoder coverage: {len(decoder)}/{len(rows)}. Not yet decoded: {len(rows)-len(decoder)}.**', '',
        'No row means “all operand modes complete.” Sources, literals, modifiers, register widths, '
        'execution masks, graphics versus compute applicability, control-flow interactions and numerical behavior '
        'still require their own contracts and tests. This list covers the pinned inventory, not every future '
        'AMD instruction or every Xbox graphics method.', '',
        'The evidence columns reproduce `docs/status.toml` without turning local readback or decoder support '
        'into independent acceptance. Historical rows can lag the newer '
        '[local checkpoints](../testing/local-graphics-checkpoint.md); reconcile their evidence before changing status. '
        'Missing CPU/GPU records mean unrecorded, not a test failure.', '',
        '## Stage 3 planning map', '',
        'The partition below publishes the working arithmetic planning rules. It is provisional, '
        'not a claim that a stage is complete or that forms outside arithmetic are optional. '
        'S3.6 numerical correctness applies across floating-point operations and is not counted as a disjoint opcode bucket. '
        'State and other-family assignments remain explicit gaps.', '',
        '| Workstream | Total forms | Decoded | Not decoded |', '| --- | ---: | ---: | ---: |',
    ]
    groups = {}
    for family, name, _, state, _ in rows:
        groups.setdefault(workstream(family, name), Counter())[state] += 1
    for group, tally in sorted(groups.items()):
        total = sum(tally.values())
        md.append(f'| {group} | {total} | {total-tally["missing"]} | {tally["missing"]} |')
    arithmetic = groups['S3.1 Arithmetic']
    arithmetic_total = sum(arithmetic.values())
    arithmetic_decoded = arithmetic_total - arithmetic['missing']
    md += ['', f'**{arithmetic_decoded}/{arithmetic_total} arithmetic forms decoded is not {arithmetic_decoded} accepted or fully converted forms.** '
        'This distinction corrects the earlier shorthand used in progress updates.', '',
        '## Families', '', '| Family | Total | Fixture accepted | Acceptance pending | Evidence gap | Not converted |',
        '| --- | ---: | ---: | ---: | ---: | ---: |']
    families = sorted({r[0] for r in rows})
    for family in families:
        tally = Counter(r[3] for r in rows if r[0] == family)
        md.append(f'| [{family}](#{family.lower()}) | {sum(tally.values())} | {tally["verified"]} | {tally["pending"]} | {tally["unrecorded"]} | {tally["missing"]} |')
    md += ['', '## Regenerate and validate', '', 'From the repository root:', '', '```sh',
        'python3 scripts/audit/shader_checklist.py', 'python3 scripts/audit/shader_checklist.py --check', '```', '',
        'Update the implementation and its evidence in `docs/status.toml`, then regenerate. '
        'The check compares every generated row and the map against current sources. '
        'CI and the full graphics-validation status gate run this check automatically. '
        'Do not hand-edit generated rows. The narrower [evidence table](instruction-coverage.md) '
        'retains operand contracts and validation details. The source snapshot and attribution are in '
        '[rdna2.jsonl](../../crates/isa/data/rdna2.jsonl).', '']
    for family in families:
        md += [f'## {family}', '', '| OP | Instruction | Conversion status | Recorded lowering | CPU evidence | GPU evidence |',
               '| ---: | --- | --- | --- | --- | --- |']
        for fam, name, opcode, state, record in rows:
            if fam != family:
                continue
            values = [opcode, f'`{name}`', STATES[state][0], record.get('lowering', '—'),
                      record.get('cpu', '—'), record.get('gpu', '—')]
            md.append('| ' + ' | '.join(cell(v) for v in values) + ' |')
        md.append('')
    # Accessible per-form map: one square per encoding, grouped by family.
    width, columns, pitch = 1040, 64, 13
    height = 130 + sum(35 + ((sum(r[0] == f for r in rows)+columns-1)//columns)*pitch for f in families)
    svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-labelledby="title desc">',
           '<title id="title">All pinned RDNA2 instruction forms: conversion and evidence</title>',
           '<desc id="desc">One square per instruction encoding. Green: accepted fixture subset; amber: recorded translation with acceptance pending; blue: decoder with evidence gap; gray: not converted. Full accessible table in shader-checklist.md.</desc>',
           f'<rect width="{width}" height="{height}" fill="#0d1117"/>',
           '<g font-family="system-ui,sans-serif" fill="#e6edf3">',
           f'<text x="24" y="30" font-size="20">RDNA2 conversion checklist · {len(rows):,} forms</text>',
           f'<text x="24" y="55" font-size="14">{len(decoder)} decoded · {counts["missing"]} not converted · operand subsets only</text>']
    for i, (state, (label, color)) in enumerate(STATES.items()):
        x = 24 + (i % 2)*500
        y = 78 + (i//2)*22
        svg += [f'<rect x="{x}" y="{y-10}" width="10" height="10" fill="{color}"/>',
                f'<text x="{x+18}" y="{y}" font-size="12">{escape(label)}: {counts[state]}</text>']
    y = 125
    for family in families:
        members = [r for r in rows if r[0] == family]
        svg.append(f'<text x="24" y="{y}" font-size="13">{family} · {len(members)}</text>')
        y += 10
        for i, (_, name, opcode, state, _) in enumerate(members):
            label = f'{family} {name} OP {opcode}: {STATES[state][0]}'
            svg.append(f'<rect x="{24+(i%columns)*pitch}" y="{y+(i//columns)*pitch}" width="10" height="10" fill="{STATES[state][1]}"><title>{escape(label)}</title></rect>')
        y += ((len(members)+columns-1)//columns)*pitch + 25
    svg += ['</g>', '</svg>']
    return {'docs/shaders/shader-checklist.md': '\n'.join(md),
            'docs/assets/shader-conversion-map.svg': '\n'.join(svg)+'\n'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    failed = False
    for relative, content in generate().items():
        path = ROOT / relative
        if args.check:
            if not path.exists() or path.read_text() != content:
                print(f'Stale or missing: {relative}', file=sys.stderr)
                failed = True
        else:
            path.write_text(content)
            print(f'Wrote {relative}')
    return int(failed)


if __name__ == '__main__':
    raise SystemExit(main())
