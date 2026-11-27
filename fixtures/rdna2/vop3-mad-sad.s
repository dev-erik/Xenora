// SPDX-License-Identifier: GPL-3.0-or-later
// Original twelve-mode MAD/SAD fixture; all results feed a rolling checksum.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v30, 0
v_mad_i32_i24 v3, v1, 2, 0xffffffff
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0x800000
v_mad_i32_i24 v3, v2, 2, 0xffffffff
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mad_i32_i24 v3, v1, 2, 0xffffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0x800000
v_mad_i32_i24 v3, v2, 2, 0xffffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mad_u32_u24 v3, v1, 2, 0xffffffff
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0xffffff
v_mad_u32_u24 v3, v2, 0xffffff, 1
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mad_u32_u24 v3, v1, 2, 0xffffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0xffffff
v_mad_u32_u24 v3, v2, 0xffffff, 1 clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_sad_u8 v3, v1, 0, 0xffffffff
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0xffffffff
v_sad_u8 v3, v2, 0, 0xffffffff
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_sad_u8 v3, v1, 0, 0xffffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0xffffffff
v_sad_u8 v3, v2, 0, 0xffffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_sad_hi_u8 v3, v1, 0, 0xffffffff
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0xffffffff
v_sad_hi_u8 v3, v2, 0, 0xffffffff
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_sad_hi_u8 v3, v1, 0, 0xffffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0xffffffff
v_sad_hi_u8 v3, v2, 0, 0xffffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_sad_u16 v3, v1, 0, 0xffffffff
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0xffffffff
v_sad_u16 v3, v2, 0, 0xffffffff
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_sad_u16 v3, v1, 0, 0xffffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0xffffffff
v_sad_u16 v3, v2, 0, 0xffffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_sad_u32 v3, v1, 0, 0xffffffff
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0xffffffff
v_sad_u32 v3, v2, 0, 1
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_sad_u32 v3, v1, 0, 0xffffffff clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0xffffffff
v_sad_u32 v3, v2, 0, 1 clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0x8000
v_mad_i32_i24 v3, v2, 0x10000, -1
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mad_i32_i24 v3, v2, 0x10000, -1 clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mov_b32_e32 v2, 0xff7fff
v_mad_i32_i24 v3, v2, 0x10000, 0x10000
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
v_mad_i32_i24 v3, v2, 0x10000, 0x10000 clamp
v_mul_lo_u32 v30, v30, 33
v_add_nc_u32_e64 v30, v30, v3
buffer_store_dword v30, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
