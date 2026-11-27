// SPDX-License-Identifier: GPL-3.0-or-later
// Original RDNA2 scalar pack fixture; expected input XOR 0xef011234.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
s_mov_b32 s8, 0x1234abcd
s_mov_b32 s9, 0x5678ef01
s_cmp_eq_u32 0, 0
s_pack_ll_b32_b16 s10, s8, s9
s_pack_lh_b32_b16 s11, s8, s9
s_pack_hh_b32_b16 s12, s8, s9
s_cbranch_scc0 fail
v_mov_b32_e32 v2, s10
v_mov_b32_e32 v3, s11
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s12
v_xor_b32_e32 v2, v2, v3
v_xor_b32_e32 v2, v1, v2
s_branch done
fail:
v_mov_b32_e32 v2, 0
done:
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
