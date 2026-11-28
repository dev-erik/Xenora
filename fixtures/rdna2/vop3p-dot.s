// SPDX-License-Identifier: GPL-3.0-or-later
// Original packed integer dot-product fixture, default half selection and wrapping results.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0x8001ffff
v_mov_b32_e32 v3, 0xffff8000
v_dot2_i32_i16 v4, v1, v2, v3
v_dot2_u32_u16 v4, v1, v2, v3
v_dot4_i32_i8 v4, v1, v2, v3
v_dot4_u32_u8 v4, v1, v2, v3
v_dot8_i32_i4 v4, v1, v2, v3
v_dot8_u32_u4 v4, v1, v2, v3
buffer_store_dword v4, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
