// SPDX-License-Identifier: GPL-3.0-or-later
// Original Wave32 WRITELANE: captured input survives source overwrite;
// index32 wraps to lane0, EXEC0 does not suppress the write, others retain 7.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_readfirstlane_b32 s8, v1
v_mov_b32_e32 v1, 9
v_mov_b32_e32 v2, 7
s_mov_b32 s9, 32
s_mov_b32 exec_lo, 0
v_writelane_b32 v2, s8, s9
s_mov_b32 exec_lo, -1
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
