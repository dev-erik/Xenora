# WRITELANE constant-data increment

Continue S3.4 after the complete-wave Wave64 checkpoint. AMD RDNA2 document
70648 §12.12 p184 explicitly permits constants as WRITELANE source data.

## Contract

Admit ordinary SGPR data, inline integer selectors128–208, inline float-bit
selectors240–248 and literal selector255. A literal is the third dword; the
instruction consumes12 bytes rather than8. Keep the lane selector an ordinary
SGPR0–105 with its existing compile-time-known requirement. Unused SRC2 remains
inline zero, modifiers remain rejected, and special register data stays outside
this increment. WRITELANE ignores EXEC and preserves every unselected lane.
Existing initialized-destination, pending-load, replay and graphics restrictions
remain. Wave32 uses five selector bits; complete Wave64 uses six.

## Work and acceptance

- [x] Extend the typed ISA and decoder, with exact LLVM encoding checks.
- [x] Extend compute lowering and independently implemented reference evaluation.
- [x] Independently author expected words, malformed input and preservation tests.
- [x] Review semantics and implementation independently.
- [x] Run focused CPU, native Metal, regression and lint checks.
- [x] Update coverage and checkpoint, then sign off and push the tested commit.

No new instruction identity is added. S3.4 remains open for broader selectors,
mask execution, graphics/helper lanes and unresolved public ISA semantics.
