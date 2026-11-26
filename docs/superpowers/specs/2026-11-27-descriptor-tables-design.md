# S2.5 descriptor tables and shader bindings

Status: implemented and validated; see the linked implementation checklist and checkpoint.

The authorized S2.5 work adds a finite typed descriptor-table model, four simultaneous
texture and sampler slots, and bounded uniform shader descriptor indexing. Existing
two-slot public APIs and shader ABIs retain their current semantics. This remains
an original synthetic graphics ABI, not a console descriptor-memory format.

## Table contract

A table has eight typed entries: texture entries0–3 and sampler entries4–7.
Creation initializes all entries null. An update replaces a checked contiguous
range atomically; validate every record before changing any entry or revision.
Textures reference ordinary/selected-slice read-only views or array-range views.
Sampler entries copy immutable sampler values. Normalized descriptors retain the
backing independently of public view/sampler lifetime. Storage remains the existing
separate slot0 binding and must not alias sampled or color roots.

Each successful update publishes a monotonically increasing immutable revision.
Recording a bind captures that revision. Later update/destruction does not alter
recorded commands; rebinding explicitly captures a newer revision. Superseded
recorded binds retain ownership until reset. Pending receipts retain ownership after
reset, completion does not imply receipt release, and final release refunds charges.
Resource contents resolve at submission using existing Host/exact Produced rules;
a descriptor snapshot is not an early pixel snapshot. Live tables share the object
cap. A separate cap of256 live revisions includes recorded and pending revisions,
so repeated table updates cannot evade retained-metadata bounds. Failed updates,
invalid result spans and failed admission must not consume a revision or budget.

Null/unused entries are legal. Every reachable required texture/sampler must be
present and type/dimension/LOD compatible. Only required entries become sampled
accesses or uploads; all retained entries still retain their backing lifetime.
Legacy binding commands leave descriptor mode explicitly, and backend replacement
revokes descriptor support. No legacy callback receives truncated new bindings.

## Four-slot compiler and native contract

Add GraphicsPipelineTypes4 and CompiledGraphics4 with a u16 pair mask
(bit texture*4+sampler), four array-layer lists and uniform index requirements.
A shared compiler core restricts legacy entry points to their original bases and
maps pair bits back explicitly; it does not truncate the four-slot mask.
Four-slot texture bases are0,12,32,44; sampler bases8,20,40,52. Uniforms remain
s24–31. The extended ABI reserves descriptor spans; scalar scratch uses s56 onward
outside selector spans. Legacy loop fixtures retain their existing scratch rules.
Native code validates up to16 pairs and binds or clears all four texture and sampler
slots every draw. Exact producer dependencies and full-root leases apply to slots2/3.

Bounded shader indexing selects among these four bindings at GPU execution time,
using direct integer-uniform selectors and a generated switch. Selector provenance,
register identity and homogeneous candidate types/dimensions are checked by the
compiler. The loader validates every reachable selector is less than4 before any
GPU admission. Required bindings are the union of all reachable candidates, even
when a runtime condition chooses only one. Computed/divergent descriptor selection
and unrestricted bindless heaps explicitly reject; they are not silently lowered to
host-selected resources. Selector texture window s60–67 and sampler window s68–71 use their first word
as a synthetic index, established only by a direct s_mov_b32 from s24–31. Reserved
tail writes reject. Provenance is captured at each sample and conflicting joins
reject. Metadata descriptor_index_uniform_mask records all required index uniforms.
The same uniform selecting both bindings requires diagonal pairs; independent
uniforms require their Cartesian product. Texture indexing cannot include storage0.

## Additive guest operations

70 creates an empty table(device). 71 updates(table, first entry, count, records),
returning the new revision. Each record is two u64 words(kind,handle): kind0 null,
kind1 ordinary/selected-slice view, kind2 array range, kind3 sampler. A null requires
handle0; typed positions reject wrong kinds. 72 destroys(table). 73 binds(list,table)
with table0 leaving descriptor mode. 74 creates a four-slot typed pipeline using
(device,VS address/length,PS address/length,types address); types are twelve u64 words,
four texture kinds then eight export kinds. Texture kinds0float,1uint,2depth,
3float-array,4uint-array,5storage-float,6storage-uint; storage only at texture0.
Exports0float/1uint. All unused descriptor words and outputs retain ABI preflight.

## Acceptance

Original guest shaders must sample four distinguishable textures with cross-paired
samplers in one invocation. Literal outputs must distinguish revisions, slots,
mips, typed uint bits and actual runtime descriptor indices. Tests cover A/B/A,
updates after recording/submission, nulls, invalid final update records, typed ranges,
parent destruction, pending exact versions, storage feedback, legacy transitions,
shared caps and revision charge/refund. Native tests use explicit producer gates and
full-root lease differentials. Fault mutations must expose latest-revision lookup,
truncated slots, missing waits and missing ownership. Finish with all nine repository
gates, two independent component reviews, a signed-off commit and push to the fork.
This does not close S2.6/S2.7 or broader Stage3/game execution.
