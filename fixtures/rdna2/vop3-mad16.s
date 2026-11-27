// SPDX-License-Identifier: GPL-3.0-or-later
// Original mixed-width fixture: both products contribute to the full-word output.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0x8000ffff
v_mov_b32_e32 v3, 0x7fffffff
v_mad_u32_u16 v4, v1, v2, v3
v_mad_i32_i16 v5, v1, v2, v3
v_add_nc_u32_e64 v6, v4, v5
buffer_store_dword v6, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
