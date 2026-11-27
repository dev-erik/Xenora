// SPDX-License-Identifier: GPL-3.0-or-later
// Original spec-derived fixture: AMD RDNA2 XML opcode386 and Table82.
// LLVM does not implement this READFIRSTLANE encoding. The .long directive
// preserves reviewed bytes; assembling data is NOT an LLVM instruction oracle.
// Zero unused fields are a project-selected subset, not AMD canonical padding.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
// Force the literal EXEC-zero spelling used by the independent native corpus.
.long 0xbefe03ff, 0x00000000 // spec-instruction: S_MOV_B32
// v_readfirstlane_b32 s8, v1 in the spec-derived VOP3 subset.
.long 0xd5820008, 0x00000101 // spec-instruction: V_READFIRSTLANE_B32
s_mov_b32 exec_lo, -1
v_mov_b32_e32 v1, 0
v_mov_b32_e32 v3, s8
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
