// SPDX-License-Identifier: GPL-3.0-or-later
// Original byte conversion and signed low-half arithmetic fixture.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0x8001ffff
v_mov_b32_e32 v3, 0x13579bdf
v_add_nc_i16 v3, v1, v2
v_sub_nc_i16 v3, v1, v2
v_cvt_f32_ubyte0_e32 v3, v1
v_cvt_f32_ubyte0_e64 v3, v1
v_cvt_f32_ubyte1_e32 v3, v1
v_cvt_f32_ubyte1_e64 v3, v1
v_cvt_f32_ubyte2_e32 v3, v1
v_cvt_f32_ubyte2_e64 v3, v1
v_cvt_f32_ubyte3_e32 v3, v1
v_cvt_f32_ubyte3_e64 v3, v1
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
