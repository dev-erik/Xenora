// SPDX-License-Identifier: GPL-3.0-or-later
// Original full-Wave32 fixture; public AMD RDNA2 ISA sections 12.12/12.13.
// Encoding checked with native LLVM 23.1.6, gfx1030, +wavefrontsize32.
// Launch complete 32-lane waves; v0 is the global input/output word index.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v3, 0
// Derive the lane index without reading reserved memory-index VGPR v0.
v_mbcnt_lo_u32_b32 v2, -1, 0
v_lshlrev_b32_e32 v2, 2, v2
v_xor_b32_e32 v2, 4, v2
ds_bpermute_b32 v3, v2, v1
s_waitcnt lgkmcnt(0)
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
