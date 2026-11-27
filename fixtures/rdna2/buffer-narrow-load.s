// SPDX-License-Identifier: GPL-3.0-or-later
// Original four-byte-stride narrow loads, each independently waited and XORed.
.text
buffer_load_ubyte v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
buffer_load_sbyte v2, v0, s[0:3], 0 idxen
s_waitcnt_vmcnt null, 0
buffer_load_ushort v3, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
buffer_load_sshort v4, v0, s[0:3], 0 idxen
s_waitcnt_vmcnt null, 0
v_xor_b32_e32 v5, v1, v2
v_xor_b32_e32 v5, v5, v3
v_xor_b32_e32 v5, v5, v4
buffer_store_dword v5, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
