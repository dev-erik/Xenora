# Canonical integer e64 aliases

Both independent reviewers accepted this bounded implementation. It does not
mark S3.1 complete.

Eighteen binary and five unary operations use an alternate VOP3 encoding of
existing arithmetic. The binaries are signed/unsigned low/high 24-bit products,
signed/unsigned min/max, XNOR, ADD_NC/SUB_NC/SUBREV_NC, three reverse shifts and
AND/OR/XOR. Unary forms are NOT, BFREV, FFBH_U32, FFBL and FFBH_I32. ADD_NC e64
already existed, so this adds22 identities and expands one existing source domain.

All active inputs use checked nine-bit selectors: SGPR0–105, VCC_LO106, inline
integer/raw floating-named words, one shared literal, or VGPR0–255. Binary SRC1
now admits the same source classes as SRC0. Binary unused SRC2 must be inline
zero128; unary SRC1 and SRC2 must both be128. Unused fields do not consume
literals or scalar input slots. Canonical padding is a subset restriction;
alternative padding disassembled by LLVM is not claimed hardware-invalid.

Lengths are8 bytes, or12 when any active selector is255; repeated literal
selectors share a single trailing word. Distinct scalar identities are checked
before immediate conversion. First-word bits8–15 and second-word bits27–31
must be zero. Five LLVM-accepted clamp variants remain explicit modifier debt.
Other special sources, DPP/SDWA and floating-point arithmetic remain excluded.

Reverse shifts take count in SRC0 and data in SRC1; lowering reverses them once.
Existing arithmetic rules, including signed24 interpretation, five-bit shift
counts, wraparound and scan sentinels, are reused. Both sources are captured
before any destination write. Flags, inactive lanes and initialization rules
retain their existing contracts.

Old ADD_NC VGPR/VGPR inputs retain their original decoded representation and
runtime-VCC-region behavior. Newly admitted ADD_NC source combinations and all
other aliases remain excluded from those runtime regions. Full-stream decoding
still validates malformed encodings before empty EXEC can skip operand reads.
Graphics retains its scalar ABI, uniform-mask accounting, reserved destinations,
integer provenance and forbidden float/metadata uses at every source position.

Primary encoding and semantics sources are the public [AMD RDNA2 ISA guide](https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content),
sections6.2,12.7,12.8 and13.3.4, and pinned local ISA metadata. Native ARM64 LLVM
probe outputs are recorded in `artifacts/s31-vop3-alias-evidence.json`; assembly
agreement is not arithmetic execution evidence. Final tests and independent
acceptance are recorded in the [local graphics checkpoint](../testing/local-graphics-checkpoint.md).
