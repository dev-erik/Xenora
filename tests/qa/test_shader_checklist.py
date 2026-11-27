# SPDX-License-Identifier: GPL-3.0-or-later
"""Stage ownership must not hide branches and memory behind arithmetic filters."""
import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("shader_checklist", ROOT / "scripts/audit/shader_checklist.py")
checklist = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checklist)


class StageOwnership(unittest.TestCase):
    def test_control_flow(self):
        for family, name in (("SOPP", "S_CBRANCH_SCC0"), ("SOPP", "S_CBRANCH_SCC1"),
                             ("SOPP", "S_ENDPGM"), ("SOP1", "S_SETPC_B64")):
            self.assertEqual(checklist.workstream(family, name), "S3.2 Control flow")

    def test_memory_families(self):
        data = checklist.load_inventories(ROOT)
        families = {"FLAT", "GLOBAL", "SCRATCH", "SMEM", "MUBUF", "MTBUF", "MIMG", "EXP", "VINTRP"}
        for family, name in data["identified"]:
            if family in families:
                self.assertEqual(checklist.workstream(family, name), "S3.5 Memory / interfaces", name)

    def test_lane_operations_separate_from_lds(self):
        for name in ("DS_SWIZZLE_B32", "DS_PERMUTE_B32", "DS_BPERMUTE_B32"):
            self.assertEqual(checklist.workstream("DS", name), "S3.4 Wave / lane")
        for name in ("DS_READ_B32", "DS_ADD_RTN_U32"):
            self.assertEqual(checklist.workstream("DS", name), "S3.5 Memory / interfaces")

    def test_waits_and_barriers(self):
        for name in ("S_WAITCNT", "S_WAITCNT_DEPCTR", "S_BARRIER", "S_ICACHE_INV"):
            self.assertEqual(checklist.workstream("SOPP", name), "S3.5 Memory / interfaces")

    def test_every_encoding_counted_once(self):
        data = checklist.load_inventories(ROOT)
        groups = {}
        for form in data["identified"]:
            groups.setdefault(checklist.workstream(*form), set()).add(form)
        self.assertEqual(sum(map(len, groups.values())), len(data["identified"]))
        self.assertEqual(set().union(*groups.values()), set(data["identified"]))
