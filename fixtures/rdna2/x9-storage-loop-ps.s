// SPDX-License-Identifier: GPL-3.0-or-later
// Guest MIN explicitly bounds count. Each iteration overwrites its own pixel.
// n=0 preserves poison; n=1 writes red64; n=3 writes red191; n>=4 red255.
// Other stored channels are always [128,191,255]. MRT0 always red.
.text
s_min_u32 s32, s24, 4
v_mov_b32_e32 v8, 0
v_mov_b32_e32 v9, lit(0x3f000000)
v_mov_b32_e32 v10, lit(0x3f400000)
v_mov_b32_e32 v11, 1.0
v_mov_b32_e32 v12, lit(0x3e800000)
loop_header:
s_cmp_lg_u32 s32, 0
s_cbranch_scc0 loop_exit
v_add_f32_e32 v8, v12, v8
image_store v[8:11], v[32:33], s[0:7] dmask:0xf dim:SQ_RSRC_IMG_2D unorm
s_sub_u32 s32, s32, 1
s_branch loop_header
loop_exit:
v_mov_b32_e32 v16, 1.0
v_mov_b32_e32 v17, 0
exp mrt0, v16, v17, v17, v16 done vm
s_endpgm
