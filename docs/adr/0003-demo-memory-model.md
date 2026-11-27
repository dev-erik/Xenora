# 0003. Use disjoint lane-indexed buffers for the first fixture

For the synthetic compute fixture, the resource beginning at SGPR 0 denotes the read-only input words and the one beginning at SGPR 4 denotes the output words. VGPR 0 starts at the lane index and must remain unchanged when used as a resource index. Each allocation has the same word count, and each dispatched invocation accesses only its own lane's word. The fixture ABI provides no raw resource descriptor bits or guest virtual addresses.

The compiler must reject descriptor forms, offsets, index changes and control flow outside this contract. Both engines must honor the load dependency and completion order. The host reads output after command completion. Supporting shared writable buffers will require a new ordering and aliasing contract.

AMD RDNA 2 specifies implicit outstanding memory waits at program termination (class B, CLAIM-RDNA2-TRANSFER-WAITS). This fixture policy requires explicit corresponding wait instructions before termination to keep the bounded execution model auditable. See the [RDNA 2 source notes](../research/rdna2.md).

The bounded narrow-load extension retains the same four-byte stride and bindings:
byte/halfword loads select the low subword of that invocation's little-endian
input element and extend it into one 32-bit destination. This is not packed
one-byte/two-byte array indexing. The existing single-pending-load, full-EXEC,
zero-offset and explicit-wait rules apply to all four narrow forms.


Pure lane-read replay may read an earlier immutable input value at the current
logical wave's first index. This extends internal input access beyond the
caller's own element, while preserving disjoint outputs and input bounds:
lane zero exists for every nonempty partial Wave32 group. It does not admit
shared writable memory, arbitrary addresses or values for absent lanes. The
[bounded lane-read interface](../interfaces.md#bounded-wave32-lane-read-scalar-values)
defines operand and consumer restrictions.
