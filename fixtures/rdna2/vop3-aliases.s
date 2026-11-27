// SPDX-License-Identifier: GPL-3.0-or-later
// Original alias fixture: every result feeds a rolling multiply/add checksum.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0x80000001
v_mov_b32_e32 v30, 0
s_mov_b32 s32, 3
v_mul_i32_i24_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mul_hi_i32_i24_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mul_u32_u24_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mul_hi_u32_u24_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_min_i32_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_max_i32_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_min_u32_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_max_u32_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_xnor_b32_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_add_nc_u32_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_sub_nc_u32_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_subrev_nc_u32_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_and_b32_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_or_b32_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_xor_b32_e64 v3, v1, v2
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_lshlrev_b32_e64 v3, s32, v1
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_lshrrev_b32_e64 v3, s32, v1
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_ashrrev_i32_e64 v3, s32, v1
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_not_b32_e64 v3, v1
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_bfrev_b32_e64 v3, v1
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_ffbh_u32_e64 v3, v1
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_ffbl_b32_e64 v3, v1
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_ffbh_i32_e64 v3, v1
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_add_nc_u32_e64 v30, v30, s32
buffer_store_dword v30, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
