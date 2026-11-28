// SPDX-License-Identifier: GPL-3.0-or-later
// Original packed binary fixture; all twelve encodings, with final unsigned-minimum output.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0x8001ffff
v_pk_mul_lo_u16 v3, v1, v2
v_pk_add_i16 v3, v1, v2
v_pk_sub_i16 v3, v1, v2
v_pk_lshlrev_b16 v3, v1, v2
v_pk_lshrrev_b16 v3, v1, v2
v_pk_ashrrev_i16 v3, v1, v2
v_pk_max_i16 v3, v1, v2
v_pk_min_i16 v3, v1, v2
v_pk_add_u16 v3, v1, v2
v_pk_sub_u16 v3, v1, v2
v_pk_max_u16 v3, v1, v2
v_pk_min_u16 v3, v1, v2
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
