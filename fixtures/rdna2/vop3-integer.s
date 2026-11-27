// SPDX-License-Identifier: GPL-3.0-or-later
// Original full-width products and ternary operations with runtime input words.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0x80000001
v_mov_b32_e32 v3, 0x0f0f0f0f
v_mul_lo_u32 v4, v1, v2
v_mul_hi_u32 v5, v1, v2
v_mul_hi_i32 v6, v1, v2
v_bfi_b32 v7, v3, v4, v5
v_add3_u32 v8, v7, v6, 1
v_and_or_b32 v9, v8, v3, v5
buffer_store_dword v9, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
