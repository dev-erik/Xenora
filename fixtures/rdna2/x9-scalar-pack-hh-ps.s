// SPDX-License-Identifier: GPL-3.0-or-later
// Original AMD RDNA2 12.1 scalar runtime case. s24=A,s25=B,s26=SCC-in,s27=expected.
// Red=result equality, green=SCC-out. MOV to s105 must preserve SCC and all bits.
.text
s_mov_b32 s32, s24
s_mov_b32 s33, s25
s_cmp_lg_u32 s26, 0
s_pack_hh_b32_b16 s32, s32, s33
s_mov_b32 s105, s32
v_mov_b32_e32 v8, 0
v_mov_b32_e32 v9, 0
v_mov_b32_e32 v10, 0
v_mov_b32_e32 v11, 1.0
s_cbranch_scc0 flag_done
v_mov_b32_e32 v9, 1.0
flag_done:
s_cmp_eq_u32 s105, s27
s_cbranch_scc0 exports
v_mov_b32_e32 v8, 1.0
exports:
exp mrt0, v8, v9, v10, v11 done vm
s_endpgm
