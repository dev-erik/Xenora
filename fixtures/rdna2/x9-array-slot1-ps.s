// SPDX-License-Identifier: GPL-3.0-or-later
// Original DIM=5 sample; s24 contains IEEE754 f32 bits of a validated integral
// range-relative layer. No conversion instruction or numeric uint layer ABI.
// AMD RDNA2 sections 8.2.1-8.2.4 / 13.7: contiguous float u,v,slice.
.text
v_interp_p1_f32 v2, v0, attr0.x
v_interp_p2_f32 v2, v1, attr0.x
v_interp_p1_f32 v3, v0, attr0.y
v_interp_p2_f32 v3, v1, attr0.y
v_mov_b32_e32 v4, s24
image_sample_lz v[8:11], v[2:4], s[12:19], s[20:23] dmask:0xf dim:SQ_RSRC_IMG_2D_ARRAY
exp mrt0, v8, v9, v10, v11 done vm
s_endpgm
