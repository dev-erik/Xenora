// SPDX-License-Identifier: GPL-3.0-or-later
// Original synthetic pixel fixture: (r*0.5+0.25, g*0.5, b+0.25, a).
.text
v_interp_p1_f32_e32 v2, v0, attr0.x
v_interp_p2_f32_e32 v2, v1, attr0.x
v_interp_p1_f32_e32 v3, v0, attr0.y
v_interp_p2_f32_e32 v3, v1, attr0.y
v_interp_p1_f32_e32 v4, v0, attr0.z
v_interp_p2_f32_e32 v4, v1, attr0.z
v_interp_p1_f32_e32 v5, v0, attr0.w
v_interp_p2_f32_e32 v5, v1, attr0.w
v_mov_b32_e32 v6, lit(0x3f000000)
v_mov_b32_e32 v7, 0x3e800000
v_mul_f32_e64 v8, v2, v6
v_add_f32_e64 v8, v8, v7
v_mul_f32_e64 v9, v3, v6
v_add_f32_e64 v10, v4, v7
exp mrt0, v8, v9, v10, v5 done vm
s_endpgm
