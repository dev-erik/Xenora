// SPDX-License-Identifier: GPL-3.0-or-later
// Original full-Wave32 fixture; public AMD RDNA2 ISA sections 12.12/12.13.
// Encoding checked with native LLVM 23.1.6, gfx1030, +wavefrontsize32.
// Launch complete 32-lane waves; v0 is the global input/output word index.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v3, 0
// Low nibble serves lane0: reverse each row independently.
s_mov_b32 s8, 0x89abcdef
s_mov_b32 s9, 0x01234567
v_permlane16_b32 v3, v1, s8, s9
buffer_store_dword v3, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
