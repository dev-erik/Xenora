// SPDX-License-Identifier: GPL-3.0-or-later
// Original CMPK fixture: every relation uses the same 0xffff immediate.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
s_mov_b32 s8, 0xffffffff
s_cmpk_eq_i32 s8, 0xffff
s_cmpk_lg_i32 s8, 0xffff
s_cmpk_gt_i32 s8, 0xffff
s_cmpk_ge_i32 s8, 0xffff
s_cmpk_lt_i32 s8, 0xffff
s_cmpk_le_i32 s8, 0xffff
s_cmpk_eq_u32 s8, 0xffff
s_cmpk_lg_u32 s8, 0xffff
s_cmpk_gt_u32 s8, 0xffff
s_cmpk_ge_u32 s8, 0xffff
s_cmpk_lt_u32 s8, 0xffff
s_cmpk_le_u32 s8, 0xffff
v_mov_b32_e32 v2, s8
s_cbranch_scc0 done
v_xor_b32_e32 v2, v1, v2
done:
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
