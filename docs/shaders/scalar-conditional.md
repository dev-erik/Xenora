# Scalar bit operations and conditional writes

The ordinary 32-bit gfx1030 subset includes `S_BITSET0_B32`, `S_BITSET1_B32`,
`S_BITCMP0_B32`, `S_BITCMP1_B32`, `S_CMOV_B32` and `S_CMOVK_I32`.
Bit updates clear/set one bit of the old destination; tests replace SCC with
whether a source bit is zero/one. Bit indices use the low five bits. Updates
and conditional moves preserve SCC. All scalar effects execute regardless of EXEC.

Conditional moves write only when SCC is true. CMOVK sign-extends its 16-bit
immediate. The verifier requires SCC to be defined. A known-false move leaves
the destination exactly as it was, including undefined, and does not resolve an
ordinary source value. A true move can initialize a fresh destination but must
read a defined source. Runtime graphics predicates require both alternatives to
be defined. These are explicit dataflow rules; they do not claim anything about
physical hardware operand fetching. Full-stream decode and destination/ABI
restrictions apply even to untaken moves. False uniform reads do not contribute
to the graphics uniform mask.

Normal scalar selectors include SGPRs, integer/float-named inline words and one
literal. Two simultaneous literals, special registers, B64 neighbors and relative
addressing remain excluded. Compute protects descriptor destinations; graphics
writes only its permitted local scalar registers and reads its ordinary uniform
or local domain. Extended descriptor words retain their separate reservation.

Graphics records possible writes explicitly in bounded-loop analysis. Bit
updates read the carried old value. Conditional writes preserve false-path
state, and possibly zero-trip loops cannot create a definite scalar value from
an entry-undefined local. Any new write to a loop counter is rejected; changed
locals cannot retain stale bounds for an inner loop. Bit tests may set body SCC,
while loop headers retain their existing adjacent compare/exit requirement.

Original fixture: `fixtures/rdna2/scalar-conditional.s`. Decoder/LLVM, independent
reference and native compute tests accompany graphics dataflow and native exact
R32Uint readback tests. Full S3.1 remains open beyond this bounded instruction set.
