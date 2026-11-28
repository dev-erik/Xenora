// SPDX-License-Identifier: GPL-3.0-or-later
// Original byte-window fixture from public RDNA2 instruction semantics.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_alignbyte_b32 v2, v1, 0x807f81ff, 3
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
