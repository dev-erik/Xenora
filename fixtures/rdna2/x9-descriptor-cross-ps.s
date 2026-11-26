// SPDX-License-Identifier: GPL-3.0-or-later
// Original four-slot descriptor ABI fixture; no console descriptor layout.
.text
v_interp_p1_f32_e32 v2, v0, attr0.x
v_interp_p2_f32_e32 v2, v1, attr0.x
v_interp_p1_f32_e32 v3, v0, attr0.y
v_interp_p2_f32_e32 v3, v1, attr0.y
image_sample_lz v[4:7], v[2:3], s[0:7], s[52:55] dmask:0xf dim:SQ_RSRC_IMG_2D
image_sample_lz v[8:11], v[2:3], s[12:19], s[40:43] dmask:0xf dim:SQ_RSRC_IMG_2D
image_sample_lz v[12:15], v[2:3], s[32:39], s[20:23] dmask:0xf dim:SQ_RSRC_IMG_2D
image_sample_lz v[16:19], v[2:3], s[44:51], s[8:11] dmask:0xf dim:SQ_RSRC_IMG_2D
exp mrt0, v4, v8, v12, v16 done vm
s_endpgm
