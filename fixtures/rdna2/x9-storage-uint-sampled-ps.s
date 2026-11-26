// SPDX-License-Identifier: GPL-3.0-or-later
// Slot1/sampler1 sampled Uint bits feed slot0 Uint storage, without float ALU.
// Metadata: slot0StorageUint + slot1Uint + uintMRT0 = pipeline word0x145.
// s24==0 preserves prior storage; nonzero stores sampled R32Uint bits exactly.
.text
v_interp_p1_f32 v2, v0, attr0.x
v_interp_p2_f32 v2, v1, attr0.x
v_interp_p1_f32 v3, v0, attr0.y
v_interp_p2_f32 v3, v1, attr0.y
image_sample_lz v[8:11], v[2:3], s[12:19], s[20:23] dmask:0xf dim:SQ_RSRC_IMG_2D
s_cmp_lg_u32 s24, 0
s_cbranch_scc0 no_store
image_store v8, v[32:33], s[0:7] dmask:0x1 dim:SQ_RSRC_IMG_2D unorm
no_store:
exp mrt0, v8, v9, v10, v11 done vm
s_endpgm
