// SPDX-License-Identifier: GPL-3.0-or-later
// Original Wave32 comparison cone; expected masks are independently derived in
// tests/validation/tests/vop3_mbcnt_independent.rs from AMD RDNA2 ISA 12.12.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mbcnt_lo_u32_b32 v2, v1, 0
v_mbcnt_hi_u32_b32 v2, -1, v2
v_cmp_eq_u32_e32 vcc_lo, 1, v2
v_mov_b32_e32 v3, vcc_lo
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
