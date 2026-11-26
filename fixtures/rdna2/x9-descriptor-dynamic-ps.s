// SPDX-License-Identifier: GPL-3.0-or-later
// Original four-slot descriptor ABI fixture; no console descriptor layout.
.text
v_interp_p1_f32_e32 v2, v0, attr0.x
v_interp_p2_f32_e32 v2, v1, attr0.x
v_interp_p1_f32_e32 v3, v0, attr0.y
v_interp_p2_f32_e32 v3, v1, attr0.y
// s24 selects texture; s25 independently selects sampler. Both are uint indices.
s_mov_b32 s60, s24
s_mov_b32 s68, s25
image_sample_lz v[4:7], v[2:3], s[60:67], s[68:71] dmask:0xf dim:SQ_RSRC_IMG_2D
exp mrt0, v4, v5, v6, v7 done vm
s_endpgm
