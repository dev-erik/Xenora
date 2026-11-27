// SPDX-License-Identifier: GPL-3.0-or-later
// Original bounded VMCNT wait: release load before reading, preserve store wait.
.text
s_waitcnt_vmcnt null, 0
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt_vmcnt null, 0
s_waitcnt_vmcnt null, 0
v_not_b32_e32 v2, v1
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vmcnt null, 0
s_waitcnt_vscnt null, 0
s_endpgm
