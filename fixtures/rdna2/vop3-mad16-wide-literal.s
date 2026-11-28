// SPDX-License-Identifier: GPL-3.0-or-later
// Original shared literal: signed16 multiplicand and full32 accumulator.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mad_i32_i16 v2, -32768, v1, -32768
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
