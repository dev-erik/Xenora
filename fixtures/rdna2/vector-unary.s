// SPDX-License-Identifier: GPL-3.0-or-later
// Original VOP1 integer program. Combine independently observable unary words;
// individual literal/masked cases live in tests/validation/tests/vop1_integer.rs.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_not_b32_e32 v2, v1
v_bfrev_b32_e32 v3, v1
v_ffbh_u32_e32 v4, v1
v_ffbl_b32_e32 v5, v1
v_ffbh_i32_e32 v6, v1
v_xor_b32_e32 v2, v2, v3
v_xor_b32_e32 v2, v2, v4
v_xor_b32_e32 v2, v2, v5
v_xor_b32_e32 v2, v2, v6
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
