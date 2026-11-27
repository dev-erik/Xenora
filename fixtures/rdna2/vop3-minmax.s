// SPDX-License-Identifier: GPL-3.0-or-later
// Original runtime order-statistic fixture; sum all six results modulo 2^32.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0x80000001
v_mov_b32_e32 v3, 0x7ffffffe
v_min3_i32 v4, v1, v2, v3
v_min3_u32 v5, v1, v2, v3
v_max3_i32 v6, v1, v2, v3
v_max3_u32 v7, v1, v2, v3
v_med3_i32 v8, v1, v2, 1
v_med3_u32 v9, v1, v3, -1
v_add3_u32 v10, v4, v5, v6
v_add3_u32 v11, v7, v8, v9
v_add_nc_u32_e64 v12, v10, v11
buffer_store_dword v12, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
