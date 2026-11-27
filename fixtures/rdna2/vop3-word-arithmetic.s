// SPDX-License-Identifier: GPL-3.0-or-later
// Original fixture: high half of signed-saturated add, low half of wrapping add.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_add_nc_i32 v2, v1, 1 clamp
v_add_nc_i32 v3, v1, 1
v_perm_b32 v2, v2, v3, 0x07060100
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
