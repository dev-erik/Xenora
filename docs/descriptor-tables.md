# Descriptor tables and runtime indexing

The synthetic graphics service supports four texture slots and four independent
sampler slots. Tables capture immutable selections when bound into a command list;
resource contents and exact produced versions resolve when the list is submitted.
This interface does not describe Xbox descriptor memory or general bindless heaps.

## Guest operations

All calls use the versioned descriptor and result-span rules in
[graphics-call-v1](graphics-call-v1.md). Arguments below follow the version/opcode.

| Opcode | Arguments | Result |
| --- | --- | --- |
| 70 | device | Empty table handle |
| 71 | table, first entry, count, records address | New revision number |
| 72 | table | Destroy public handle |
| 73 | command list, table (zero leaves table mode) | Record immutable bind |
| 74 | device, vertex address, vertex length, pixel address, pixel length, types address | Four-slot pipeline |

A table contains texture entries 0–3 and sampler entries 4–7. Each update record
contains two little-endian u64 words: kind and handle. Kinds are 0=null (handle must
be zero), 1=ordinary or selected-slice texture view, 2=array-range view, 3=sampler.
The entry position determines its type. Empty updates, out-of-range spans and any
invalid record reject the entire update without advancing its revision.

Opcode74 reads twelve u64 words: four texture types followed by eight export types.
Texture types are 0=float, 1=uint, 2=raw depth, 3=float array, 4=uint array,
5=storage float and 6=storage uint. Storage is restricted to texture0 and uses the
separate existing writable storage binding. Exports are 0=float or 1=uint.

Creation consumes one live revision. The limit is 256 distinct live revisions,
including current tables, recorded binds and receipt-held revisions. Replacing an
otherwise unreferenced current revision does not increase this count. Shared
references count once. Tables also consume the shared guest object budget.

Destroying a view, sampler or table does not alter recorded bindings. Normalized
entries retain backing roots; even unused entries retain lifetime ownership.
Null entries are legal when unused; every shader-required entry must be present.
Only shader-required entries cause sampled accesses, uploads or producer waits.
Command-list reset releases recorded references; pending receipts retain theirs
through completion until release and dependency retirement. Legacy binding commands
leave table mode. Installing an older backend revokes descriptor support.

## Shader ABI

`compile_graphics_pipeline_typed4` returns a `CompiledGraphics4`. Its u16 pair mask
uses bit `texture * 4 + sampler`. The legacy two-slot compiler and its u8 mask
retain their existing behavior through explicit mask conversion.

| Binding | Static SGPR bases | Runtime selector window |
| --- | --- | --- |
| Texture slots 0–3 | 0, 12, 32, 44 | s60–67; index in s60 |
| Sampler slots 0–3 | 8, 20, 40, 52 | s68–71; index in s68 |

The extended ABI reserves descriptor windows. Uniforms remain s24–31; use scalar
scratch outside descriptor and selector windows, such as s56–59 or s72 onward.
Legacy programs retain their previous scratch-register rules.

A direct `s_mov_b32` from a uniform to s60 or s68 establishes an integer selector.
`IMAGE_SAMPLE_LZ` using that selector window becomes a GPU switch among the four
bindings. The host checks each captured index as raw u32 `< 4`, without float conversion.
Bit patterns 0–3 are valid; the bits for `1.0f` exceed that range and reject.
Out-of-range values, computed/divergent selectors and conflicting used joins reject.
Selector provenance is captured per sample, so later writes do not change earlier
samples. The same uniform selecting texture and sampler requires diagonal pairs;
independent uniforms require all sixteen pairs. Required bindings union all reachable
branches and must be valid even when a particular runtime selection does not use them.

Dynamic texture candidates must have homogeneous sample types and dimensions.
Array-layer bounds and sampler LOD checks apply to every possible candidate.
Unsigned samples preserve their bits. Storage0 excludes texture0 sampling and
therefore excludes dynamic selection across all four textures; a fixed nonzero
texture may still use a dynamic sampler. Storage feedback aliases reject by root.

## Validation

Run `./scripts/validate-graphics.sh quick` for focused checks or
`./scripts/validate-graphics.sh full` for all nine repository gates. Both run on
native Apple Silicon. The focused script includes descriptor compiler, loader,
native ownership/binding and original guest tests. GPU tests compare literal
outputs offscreen with Metal API and shader validation enabled.

The [design](superpowers/specs/2026-11-27-descriptor-tables-design.md) and
[implementation checklist](superpowers/plans/2026-11-27-descriptor-tables.md)
record the finite S2.5 scope. General guest-memory coherence, broader queue graphs,
unrestricted descriptor indexing and commercial game compatibility remain separate.
