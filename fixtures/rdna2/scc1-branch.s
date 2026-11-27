// SPDX-License-Identifier: GPL-3.0-or-later
// Original SCC1 fixture: take the first branch, fall through the second.
// The observable result is 7; either inverted branch changes the result.
.text
v_mov_b32_e32 v1, 0
s_cmp_eq_u32 0, 0
s_cbranch_scc1 .Ltaken
v_mov_b32_e32 v1, 99
s_branch .Lstore
.Ltaken:
v_mov_b32_e32 v1, 7
s_cmp_eq_u32 0, 1
s_cbranch_scc1 .Lwrong
s_branch .Lstore
.Lwrong:
v_mov_b32_e32 v1, 55
.Lstore:
buffer_store_dword v1, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
