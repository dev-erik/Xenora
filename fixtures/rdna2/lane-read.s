// SPDX-License-Identifier: GPL-3.0-or-later
// Original guaranteed-lane-zero Wave32 fixture, including partial waves.
// READLANE masks scalar index32 to zero. Captured input survives overwrite.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_readfirstlane_b32 s8, v1
v_mbcnt_lo_u32_b32 v2, -1, 0
s_mov_b32 s9, 32
v_readlane_b32 s10, v2, s9
v_mov_b32_e32 v1, 0
v_mov_b32_e32 v3, s8
v_mov_b32_e32 v4, s10
v_xor_b32_e32 v3, v3, v4
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
