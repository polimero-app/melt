# Model previews

Last-updated: 2026-11-03

Melt generates static previews locally. Embedded images are preferred; models
without one use the desktop CPU renderer. Preview support does not imply that
a file can be printed directly or opened by every configured slicer.

## Implementation sequence

Each chunk includes its own validation and documentation update and is committed
independently. Existing unrelated working-tree changes are outside this work.

1. **Preview plumbing and 3MF images:** share bounded image/archive helpers,
   follow thumbnail relationships, rank fallback images, decode JPEG to PNG.
2. **3MF geometry:** resolve model parts, build items, components, transforms,
   and Bambu/Orca production references with bounded XML parsing.
3. **Project and toolpath previews:** UFP, FreeCAD, Fusion F3D/F3Z, LightBurn,
   G-code/GCO and binary G-code embedded previews.
4. **Mesh formats:** AMF, OFF, PLY, GLB/glTF, Collada, VRML/X3D and 3DS.
5. **Point clouds:** XYZ/PTS/ASC, PLY without faces, PCD, LAS and E57, with
   bounded sampling and a point-rendering path.
6. **Native application previews:** Blender, Rhino, SketchUp, SolidWorks and
   Inventor stored previews, using Rust libraries for containers/codecs.
7. **Qualification:** complete format/limitation documentation, frontend/backend
   coverage checks, current screenshot gallery and before/after evidence.

No Swift application, external renderer or ModelThumbs binary is embedded.
STEP/IGES/BREP and all resin formats (including SL1/SL1S) are excluded.

## Current support

Local files support STL, OBJ, 3MF, and the first supported preview file in ZIP.
G-code/GCO scan the first 4 MiB and last 1 MiB for PNG, JPEG or QOI comment
thumbnails and choose by decoded pixel area. BGCode supports version 1 embedded
PNG/JPEG/QOI blocks, uncompressed or zlib/DEFLATE compressed; Heatshrink blocks
are skipped. Toolpath blocks are never decompressed.

UFP, FreeCAD (`.FCStd`), Fusion (`.f3d`/`.f3z`) and LightBurn (`.lbrn`/`.lbrn2`)
use stored previews. F3Z searches nested F3D designs, largest first, with one
level of nesting. ZIP image extraction has a cumulative 64 MiB budget. Files
with no stored image remain unavailable; these formats are not geometry loaders.
Printer-hosted previews support STL, OBJ and 3MF. Large local 3MFs can use an
embedded thumbnail without loading the entire archive into memory.
3MF thumbnails follow the package thumbnail relationship first, then prefer
thumbnail-named images and plate 1 before arbitrary images. Known helper masks
are excluded. Invalid candidates are skipped. PNG and JPEG previews are fully
decoded under resource limits; JPEG is normalized to PNG.

3MF geometry resolves the root model through package relationships, follows
production-extension part references inside the archive, selects build items,
and composes component/build transforms. Nonprintable build items are skipped.
Files omitting the build section fall back to resource objects. Cycles, missing
objects, malformed XML, invalid indices and nonfinite coordinates are rejected.
The aggregate XML budget is 128 MiB; assembly traversal is limited to 32 levels,
100,000 instance visits and two million output triangles.

## Mesh formats and limits

Geometry previews additionally support plain/zipped AMF, ASCII OFF, ASCII and
binary PLY (both byte orders), GLB/glTF 2.0, Collada, VRML 1/2, XML X3D and 3DS.
Indexed polygons are fan-triangulated; concave polygons and holes are not a
full tessellation feature. Rendering uses Melt's existing shading, without
textures, material colours, animation or skin deformation.

GLB/glTF preserves scene instances and transforms and converts Y-up to Z-up.
Local `.gltf` buffers may use embedded base64 or relative files confined to the
model directory (including symlink resolution); network/absolute/percent-encoded
URIs and required glTF extensions are rejected. Archived glTF needs embedded
buffers. Aggregate buffer reads are limited to 32 MiB.

Collada preserves node transforms and repeated geometry instances. VRML/X3D
supports indexed face/triangle sets, triangle sets, transforms and DEF/USE.
Primitive shapes, VRML PROTO/EXTERNPROTO, scripts and external scene resources
are unsupported. XML entity declarations/internal DTD subsets are rejected;
external DTD declarations are accepted without fetching resources. Scene walks
are bounded by 64 levels, 100,000 visits and two million output triangles.

## Point clouds

XYZ/PTS/ASC text, PCD (ASCII, binary and LZF binary_compressed), LAS and E57
render as points. PLY without faces also uses the point renderer. E57 combines
all scans with their poses and converts spherical coordinates when needed.
LAS/E57/text coordinates are recentered in f64 before f32 rendering.

Point storage/rendering is capped at one million samples, with deterministic
uniform thinning across the input. Local scans are limited to 512 MiB and
100 million records. Text lines are capped at 64 KiB. PCD input/decompressed
data is capped at 128 MiB; E57 XML at 8 MiB with standard 1024-byte pages.
Archived scans use the ordinary 32 MiB archive-entry limit. LAZ is excluded.
Point colours/intensities are not rendered; depth shading uses Melt's model colour.

## Native application previews

Blender (`.blend`/`.blend1`), Rhino (`.3dm`), SketchUp (`.skp`), SolidWorks
(`.sldprt`/`.sldasm`/`.slddrw`) and Inventor (`.ipt`/`.iam`/`.ipn`/`.idw`) use
stored images only. Supported containers include Blender TEST blocks (classic
32/64-bit, both byte orders, newer wide headers, plain or gzip), Rhino compressed
and plain DIB previews, SketchUp CDib PNGs, SolidWorks DEFLATE preview records,
and OLE/CFB preview and SummaryInformation bitmap streams.

Missing previews remain unavailable. Zstd-compressed Blender files are excluded.
Native formats vary by application/version; generic PNG fallbacks can select a
component image or texture when no identifiable document thumbnail exists.
No proprietary geometry is parsed, and application code/macros never execute.

Non-OLE native previews search the first 64 MiB; gzip output is likewise limited
to its first 64 MiB. OLE containers can be up to 512 MiB and use seekable stream
reads with a cumulative 64 MiB image budget. Individual stored images/bitmap
payloads are capped at 16 MiB. SolidWorks/Rhino decompression attempts share a
64 MiB output budget, including unsuccessful attempts.

## Validation and evidence

Format loaders need representative valid, malformed and oversized input tests.
Image decoding, decompression, recursion and output geometry must be bounded.
The existing cache and two-job concurrency limit remain in use.

Visible changes follow [the screenshot procedure](screenshots.md#required-ui-pr-evidence).
Browser screenshots use synthetic previews; loader tests establish actual
format decoding. Neither establishes native-OS or live-printer qualification.

## Progress

- Planning: complete. Baseline: `b2e8d5e09dd5c6824853d155e7f27a0e5f66cad6`.
- Chunk 1: complete. Preview tests (15) and thumbnail regressions (5) pass.
  Baseline screenshots captured from the disposable checkout into
  `/tmp/melt-previews-before-images` (14 views).
- Chunk 2: complete. All 18 preview tests and desktop Clippy pass, including
  cross-part transforms, build selection, cycles and malformed inputs.
- Chunk 3: complete. All 22 preview tests, 175 UI tests, desktop Clippy and
  the production UI build pass. Refreshed all 14 gallery captures; visually
  checked the mixed-format library.
- Chunk 4: complete. All 31 preview tests, 175 UI tests, desktop Clippy and
  the UI production build pass. All 14 gallery captures refreshed, including
  the GLB library card.
- Chunk 5: complete. All 35 preview tests, 175 UI tests, desktop Clippy and
  the production UI build pass. LAS/E57 writer-generated round trips verify
  scan decoding and E57 poses; all gallery captures refreshed.
- Chunk 6: complete. All 40 preview tests, 175 UI tests, desktop Clippy and
  the production UI build pass. All gallery captures refreshed and reviewed.
  Native decoders have generated Blender, Rhino, SolidWorks and OLE fixtures;
  upstream attribution is included in desktop bundle resources.
- Chunk 7: pending.
