// SPDX-License-Identifier: GPL-3.0-or-later
// Original scalar bit/conditional program: output is input XOR 0x005edcbb.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
s_mov_b32 s8, 0x7fa12345
s_bitset1_b32 s8, 31
s_bitset0_b32 s8, 0
s_bitcmp0_b32 s8, 0
s_cmov_b32 s9, s8
s_bitcmp1_b32 s8, 31
s_cmovk_i32 s10, -1
v_xor_b32_e32 v2, s9, v1
v_xor_b32_e32 v2, s10, v2
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
