// SPDX-License-Identifier: GPL-3.0-or-later
// Original Wave32 EXECNZ and lane-derived VCCNZ comparison-join fixture.
.text
s_mov_b32 s8, 7
s_cbranch_execnz .Lload
s_mov_b32 s8, 11
.Lload:
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, s8
v_cmp_eq_u32_e32 vcc_lo, 0, v1
s_cbranch_vccnz .Ljoin
v_mov_b32_e32 v2, 9
.Ljoin:
v_cmp_eq_u32_e32 vcc_lo, 7, v2
v_mov_b32_e32 v3, vcc_lo
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
