// SPDX-License-Identifier: GPL-3.0-or-later
// Original arithmetic and saturating packing fixture.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0x8001ffff
v_mov_b32_e32 v3, 0x13579bdf
v_mad_u16 v3, v1, v2, v3
v_min3_i16 v3, v1, v2, v3
v_min3_u16 v3, v1, v2, v3
v_max3_i16 v3, v1, v2, v3
v_max3_u16 v3, v1, v2, v3
v_med3_i16 v3, v1, v2, v3
v_med3_u16 v3, v1, v2, v3
v_mad_i16 v3, v1, v2, v3
v_cvt_pk_u16_u32 v3, v1, v2
v_cvt_pk_i16_i32 v3, v1, v2
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
