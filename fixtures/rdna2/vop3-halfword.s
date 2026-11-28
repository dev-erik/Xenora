// SPDX-License-Identifier: GPL-3.0-or-later
// Original low-half arithmetic fixture; destination high half remains initialized.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0x8001ffff
v_mov_b32_e32 v3, 0x13579bdf
v_add_nc_u16 v3, v1, v2
v_sub_nc_u16 v3, v1, v2
v_mul_lo_u16 v3, v1, v2
v_lshrrev_b16 v3, v1, v2
v_ashrrev_i16 v3, v1, v2
v_max_u16 v3, v1, v2
v_max_i16 v3, v1, v2
v_min_u16 v3, v1, v2
v_min_i16 v3, v1, v2
v_lshlrev_b16 v3, v1, v2
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
