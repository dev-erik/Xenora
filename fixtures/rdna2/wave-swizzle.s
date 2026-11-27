// SPDX-License-Identifier: GPL-3.0-or-later
// Original full-Wave32 fixture; public AMD RDNA2 ISA sections 12.12/12.13.
// Encoding checked with native LLVM 23.1.6, gfx1030, +wavefrontsize32.
// Launch complete 32-lane waves; v0 is the global input/output word index.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v3, 0
// and=31, or=0, xor=16: exchange the two 16-lane rows.
ds_swizzle_b32 v3, v1 offset:0x401f
s_waitcnt lgkmcnt(0)
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
