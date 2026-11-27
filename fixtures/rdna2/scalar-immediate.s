// SPDX-License-Identifier: GPL-3.0-or-later
// Original signed-immediate chain: (-32768 + -1) * -1 = 32769.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
s_movk_i32 s8, 0x8000
s_addk_i32 s8, 0xffff
s_mulk_i32 s8, 0xffff
v_mov_b32_e32 v2, s8
v_xor_b32_e32 v2, v1, v2
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
