# Typed resource formats and roles

This page defines the finite resource-format milestone for the synthetic graphics
ABI. It is not a claim of comprehensive Direct3D format support or Xbox compatibility.
The matrix below is implemented and locally validated with native Metal and original
guest tests. See the [validation checkpoint](testing/local-graphics-checkpoint.md)
for evidence and the limits of that validation.

| Storage format | Packed block | Same-format transfers | Shader interpretation | Attachment |
| --- | --- | --- | --- | --- |
| R8 UNORM | 1 byte/texel | Yes | float, missing G/B=0, A=1 | Color, blending |
| RG8 UNORM | 2 bytes/texel | Yes | float, missing B=0, A=1 | Color, blending |
| RGBA8 UNORM | 4 bytes/texel | Yes | float, optional sRGB view | Color, blending |
| BGRA8 UNORM | 4 bytes/texel | Yes | logical RGBA float channels, optional sRGB view | Color, blending |
| RGBA16 FLOAT | 8 bytes/texel | Yes | float from binary16 storage | Color, blending |
| R32 FLOAT | 4 bytes/texel | Yes | float, missing G/B=0, A=1 | Color, blending |
| R32 UINT | 4 bytes/texel | Yes | unsigned integer, nearest only | Integer color, no blending |
| Depth32 FLOAT | 4 bytes/texel | Yes | raw scalar depth | Depth only |
| BC1 | 8 bytes/4×4 block | Block-constrained | float, optional sRGB view | Rejected |
| BC3 | 16 bytes/4×4 block | Block-constrained | float, optional sRGB view | Rejected |

BC roles require runtime BC capability. Other roles require explicit backend
registration; copy capability does not imply sampling or attachment capability.
Invalid format/role combinations reject before dispatch. These requirements cover
full mip chains, explicit arrays including one-layer arrays, selected slice/mip
read-only views, and exact GPU-produced versions. Writable subresource views and
shader-selected array indexing remain separate workstreams. Attachments use ordinary
single-mip 2D surfaces, matching the existing attachment contract.

## Representation and transfers

Format, dimensions, mip count and array identity remain authoritative across host
snapshots, caches, queued inputs and completion. Equal byte lengths never establish
compatibility. Copies preserve storage bytes: they do not decode BC blocks, reorder
BGRA channels, normalize integers or numerically convert floating-point payloads.

Pitched buffer transfers derive alignment and row lengths from storage blocks.
Compressed origins must be block aligned; partial final blocks are permitted only
at the selected mip edge. Complete destination versions preserve untouched levels,
slices, texels and buffer gaps. Full backings and staging allocations participate
in admission and lifetime accounting. Both pending inputs retain exact producer
versions and dependencies. Publication is atomic on validated completion.

## Typed shader and attachment contract

Explicit pipeline types distinguish float, unsigned integer and raw depth texture
inputs and float versus unsigned integer exports. Existing pipeline APIs default
to float. Integer sampling transports raw VGPR bits through moves and joins;
implicit floating arithmetic, floating coordinates and float exports of sampled
integer bits reject. Integer exports use bit reinterpretation, never numeric float
conversion. Nearest sampling is required for integer textures.

Metal returns scalar raw depth. The synthetic descriptor maps that value to
`(depth, 0, 0, 1)`. This vector mapping is Xenora's contract, not a four-component
Metal depth return. Comparison sampling is not included.

Per-target attachment formats stay fixed during a pass. Integer clears retain all
32 bits and cannot enable blending. Float slots may use the draw's blend state
alongside unblended integer MRT slots. Readbacks preserve each format's packed
storage; R8 does not expand to RGBA8 and floating targets do not clamp through an
RGBA8 intermediate. Host and Produced Load must match exact format and extent.

## Explicit exclusions

This milestone does not include format conversion, arbitrary reinterpretation,
stencil, comparison-depth sampling, shader storage writes, additional BC families,
packed HDR, signed-normalized or multiplanar formats. These are additional scope,
not silently supported cells. Numerical equivalence beyond the demonstrated native
cases remains part of shader numerical validation.

## Primary references

- [Apple Metal feature tables](https://developer.apple.com/metal/Metal-Feature-Set-Tables.pdf).
- [Metal Shading Language specification](https://developer.apple.com/metal/Metal-Shading-Language-Specification.pdf), texture types and sampling (§6.13).
- [AMD RDNA2 ISA](https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content), image instructions and descriptor interpretation.

## Native selected-view color Load

`QueuedColorLoad::ProducedTypedView` initializes a fresh color attachment from an
exact GPU-produced selected view. It requires a single mip, matching dimensions,
a supported color format and a linear view matching that storage format. sRGB
reinterpretations reject: Load preserves packed values and does not decode colors.
The original producer point and complete backing allocation stay retained, even
when the public root/view handles are dropped. This uses a GPU-to-GPU copy before
rendering; no pending texture is reconstructed from host bytes.

The resulting ordinary 2D surface can be passed as a source to
`submit_typed_image_copy`, with the original full texture version as destination
input and the selected mip/slice as copy destination. This produces a fresh full
backing while preserving untouched bytes. These are separate explicit submissions:
callers must collect each point and obey each operation's admission/error rules.
They are not an atomic compound submission or direct mutation of the original view.

This is a native building block. Guest writable-view descriptors, writable depth
subresources, shader-indexed array ranges, shader storage writes and range-aware
write conflicts remain open. The native tests use original trusted Metal shaders;
they do not establish guest ABI support for this operation.

Metal views share their source allocation; fresh destinations preserve Xenora's
immutable produced-version contract. See [Apple's texture-view documentation](https://developer.apple.com/documentation/metal/mtltexture).
