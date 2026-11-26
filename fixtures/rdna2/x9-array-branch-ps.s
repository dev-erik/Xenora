// SPDX-License-Identifier: GPL-3.0-or-later
// Original runtime branch: all reachable array layer requirements are validated.
.text
v_interp_p1_f32 v2, v0, attr0.x
v_interp_p2_f32 v2, v1, attr0.x
v_interp_p1_f32 v3, v0, attr0.y
v_interp_p2_f32 v3, v1, attr0.y
s_cmp_eq_u32 s25, 0
s_cbranch_scc0 second
v_mov_b32_e32 v4, s24
image_sample_lz v[8:11], v[2:4], s[0:7], s[8:11] dmask:0xf dim:SQ_RSRC_IMG_2D_ARRAY
s_branch done
second:
v_mov_b32_e32 v4, s31
image_sample_lz v[8:11], v[2:4], s[0:7], s[8:11] dmask:0xf dim:SQ_RSRC_IMG_2D_ARRAY
done:
exp mrt0, v8, v9, v10, v11 done vm
s_endpgm
