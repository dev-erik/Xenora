// SPDX-License-Identifier: GPL-3.0-or-later
.text
v_mov_b32_e32 v1, 1.0
v_mov_b32_e32 v2, 2.0
v_add_f32_e64 v3, v1, v2
v_mul_f32_e64 v3, v3, v2
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
