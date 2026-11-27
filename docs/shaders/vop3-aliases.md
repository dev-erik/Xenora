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
must be zero except for the five integer clamp modes below.
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

## Integer clamp modes

Bit15 enables saturation for MUL_I32_I24, MUL_U32_U24, ADD_NC_U32, SUB_NC_U32
and SUBREV_NC_U32. These are five operand modes of existing encodings, not five
new instruction identities. All other modifier combinations remain rejected;
ALIGNBIT clamp remains unresolved and unsupported.

Addition saturates its mathematical unsigned result at0xffffffff. Subtraction
saturates below zero, with reversed operands for SUBREV. Unsigned multiplication
uses the low24 bits of each input and saturates the product at0xffffffff. Signed
multiplication sign-extends bit23 first and saturates at signed32 limits before
narrowing. Arbitrary upper source bytes do not affect either multiply. The
multiplication rules are derived from the AMD integer-clamp rule and declared
operand/result types; no AMD hardware validation is claimed.

The implementation avoids native signed overflow and Metal64-bit dependencies.
Unsigned products overflow when their high word is nonzero. A signed product
fits when its high word equals sign extension of its low word; otherwise the
operand signs select the signed32 bound. Flags, source/lane initialization and
all graphics provenance rules are retained. Clamped ADD never enters the old
unclamped ADD runtime-VCC-region path.

Primary basis: AMD RDNA2 section6.5 and12.7. LLVM unsigned saturation selection is
independently visible in [VOP2 definitions](https://github.com/llvm/llvm-project/blob/5b10d2d74929eae78912771222c9855af97496f4/llvm/lib/Target/AMDGPU/VOP2Instructions.td).
The original `fixtures/rdna2/vop3-clamp.s` feeds ten runtime/literal results into
a rolling checksum. Both independent reviewers accepted the five modes after
full and focused validation recorded in the local graphics checkpoint.
