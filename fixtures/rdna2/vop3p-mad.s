// SPDX-License-Identifier: GPL-3.0-or-later
// Original packed multiply-add fixture, default half selection and wrapping results.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0x8001ffff
v_mov_b32_e32 v3, 0xffff8000
v_pk_mad_i16 v4, v1, v2, v3
v_pk_mad_u16 v4, v1, v2, v3
buffer_store_dword v4, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
