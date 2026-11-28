// SPDX-License-Identifier: GPL-3.0-or-later
// Original fixed-mode rounding and offset conversion fixture.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_cvt_off_f32_i4_e32 v3, v1
v_cvt_off_f32_i4_e64 v3, v1
v_trunc_f32_e32 v3, v1
v_trunc_f32_e64 v3, v1
v_ceil_f32_e32 v3, v1
v_ceil_f32_e64 v3, v1
v_rndne_f32_e32 v3, v1
v_rndne_f32_e64 v3, v1
v_floor_f32_e32 v3, v1
v_floor_f32_e64 v3, v1
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
