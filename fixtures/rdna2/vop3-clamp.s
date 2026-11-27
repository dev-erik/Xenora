// SPDX-License-Identifier: GPL-3.0-or-later
// Original clamp operand-mode fixture; rolling checksum observes every result.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v30, 0
v_mul_i32_i24_e64 v3, v1, 0x7fffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mul_i32_i24_e64 v3, 0x7fffff, 0x7fffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mul_u32_u24_e64 v3, v1, 0xffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mul_u32_u24_e64 v3, 0xffffff, 0xffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_add_nc_u32_e64 v3, v1, 0xffffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_add_nc_u32_e64 v3, 0xffffffff, 1 clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_sub_nc_u32_e64 v3, v1, 0xffffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_sub_nc_u32_e64 v3, 0, 1 clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_subrev_nc_u32_e64 v3, v1, 0 clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_subrev_nc_u32_e64 v3, 1, 0 clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
buffer_store_dword v30, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
