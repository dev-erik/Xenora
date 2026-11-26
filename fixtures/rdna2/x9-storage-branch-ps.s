// SPDX-License-Identifier: GPL-3.0-or-later
// Original bounded storage fixture. Synthetic v32/v33 are uint pixel x/y bits.
// s24==0 preserves prior storage; otherwise stores exact RGBA(.25,.5,.75,1).
// MRT0 stays literal red, independently of the side effect.
.text
v_mov_b32_e32 v8, lit(0x3e800000)
v_mov_b32_e32 v9, lit(0x3f000000)
v_mov_b32_e32 v10, lit(0x3f400000)
v_mov_b32_e32 v11, 1.0
s_cmp_lg_u32 s24, 0
s_cbranch_scc0 no_store
image_store v[8:11], v[32:33], s[0:7] dmask:0xf dim:SQ_RSRC_IMG_2D unorm
no_store:
v_mov_b32_e32 v16, 1.0
v_mov_b32_e32 v17, 0
exp mrt0, v16, v17, v17, v16 done vm
s_endpgm
