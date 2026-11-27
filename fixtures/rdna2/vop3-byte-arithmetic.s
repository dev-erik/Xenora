// SPDX-License-Identifier: GPL-3.0-or-later
// Original three-source integer fixture; every result reaches the output.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0x0100ff00
v_mov_b32_e32 v3, 0x01010101
v_xad_u32 v4, v1, v2, v3
v_lerp_u8 v5, v1, v2, v3
v_msad_u8 v6, v1, v2, v3
v_add3_u32 v7, v4, v5, v6
buffer_store_dword v7, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
