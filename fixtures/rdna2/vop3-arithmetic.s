// SPDX-License-Identifier: GPL-3.0-or-later
// Original runtime chain; all four arithmetic results contribute to the output.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_bfe_i32 v2, v1, 4, 8
v_lshl_add_u32 v3, v1, 1, v2
v_add_lshl_u32 v4, v3, v2, 3
v_lshl_or_b32 v5, v4, 1, v2
buffer_store_dword v5, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
