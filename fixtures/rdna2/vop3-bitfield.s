// SPDX-License-Identifier: GPL-3.0-or-later
// Original lane-local bitfield/count fixture. Every new result feeds the output.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_bfe_u32 v2, v1, 4, 8
v_bfm_b32 v3, 8, 4
v_alignbit_b32 v4, v1, v2, 8
v_bcnt_u32_b32 v5, v1, v2
v_or3_b32 v6, v3, v4, v5
v_xor3_b32 v7, v6, v1, v2
buffer_store_dword v7, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
