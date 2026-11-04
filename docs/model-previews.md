# Model previews

Last-updated: 2026-11-03

Melt generates static previews locally. Embedded images are preferred; models
without one use the desktop mesh/point renderer, with CPU fallback. Preview support does not imply that
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

The local library recognizes **40 extensions**, from the inventory shared by
the frontend and Rust loaders in `ui/src/preview-formats.json`:

| Family | Extensions | Method |
| --- | --- | --- |
| Printing meshes | `.stl`, `.obj`, `.amf`, `.off` | Geometry |
| 3MF | `.3mf` (including `.gcode.3mf`) | Stored image, otherwise geometry |
| Meshes/scenes | `.ply`, `.glb`, `.gltf`, `.dae`, `.wrl`, `.vrml`, `.x3d`, `.3ds` | Geometry; faceless PLY uses points |
| Point clouds | `.xyz`, `.pts`, `.asc`, `.pcd`, `.las`, `.e57` | Sampled points |
| G-code | `.gcode`, `.gco`, `.bgcode` | Stored image |
| ZIP projects | `.ufp`, `.fcstd`, `.f3d`, `.f3z` | Stored image |
| LightBurn | `.lbrn`, `.lbrn2` | Stored image |
| Native applications | `.blend`, `.blend1`, `.3dm`, `.skp`, `.sldprt`, `.sldasm`, `.slddrw`, `.ipt`, `.iam`, `.ipn`, `.idw` | Stored image |
| General ZIP | `.zip` | First supported non-ZIP entry, in archive order |

Extensions are case-insensitive. Geometry inputs and entries selected from
general ZIPs are capped at 32 MiB; 3MF mesh XML has a separate aggregate budget
of 128 MiB. Embedded images are limited to 16 MiB, 4096 pixels per side and
16 million decoded pixels. Grid renders are 640×360 and dialog renders 1920×1080.

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

Regression tests cover representative valid, malformed and oversized inputs.
Image decoding, decompression, recursion and output geometry are bounded.
The existing cache and two-job concurrency limit remain in use.
ZIP image searches share a 64 MiB decompression budget and consider at most
256 candidates per archive. Failed CRC/decompression attempts consume that
budget too. Entries cannot expand beyond their declared uncompressed size.

Visible changes follow [the screenshot procedure](screenshots.md#required-ui-pr-evidence).
Browser screenshots use synthetic previews; loader tests establish actual
format decoding. Neither establishes native-OS or live-printer qualification.
Generated regression fixtures cover binary layouts and transformations; they
are not a compatibility matrix for every version of the proprietary applications.

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
- Chunk 7: complete. Workspace tests (553), UI tests (175), workspace
  formatting/Clippy/build, license checks, dependency policy and production UI
  build pass. Archive-budget failures, malformed format inputs, LightBurn/UFP
  previews and LZF overlapping back-references have explicit regressions.
  Dependency policy reports existing duplicate/yanked-version warnings but no
  failing advisory, license or source checks.

Final comparison captures use identical sanitized fixture data. Baseline:
`b2e8d5e09dd5c6824853d155e7f27a0e5f66cad6` in the disposable checkout.
Before: `/tmp/melt-previews-before-final`; after: `/tmp/melt-previews-after-final`.
Both directories retain capture manifests. The only changed images are the file
library and the preview-dialog backdrop. All 14 gallery views were reviewed.
Comparison captures are local evidence; no PR or uploaded PR attachments have
been created by this implementation sequence.

## GPU renderer implementation sequence

1. Separate geometry preparation from drawing, add opt-in stage timings and a
   reproducible CPU benchmark for small/dense meshes and large point clouds.
2. Add a lazy shared `wgpu` offscreen renderer, reuse devices/pipelines/targets,
   and retain CPU rendering after adapter, device, validation or readback failure.
3. Compare cold and warm end-to-end timings and image fidelity; select an
   automatic policy from evidence and keep explicit CPU/GPU diagnostic overrides.
4. Qualify fallback, concurrency, resource bounds, package/dependency policy,
   documentation and visual evidence. GPU screenshots must identify the actual
   adapter; a software Vulkan adapter does not establish hardware performance.

GPU work applies to generated mesh/point images. Embedded images, decoding,
normal preparation, PNG encoding and existing preview caches remain independent.
Step 1 is complete: geometry preparation and CPU drawing are separate, stage
timings are available without recording filenames, and the benchmark reuses the
production preview library. Step 2 is complete: the optional, default-enabled
`gpu-previews` feature adds one lazy offscreen worker using `wgpu` 30, WGSL,
Vulkan/Metal/DX12 and no window or embedded renderer. Devices, pipelines, upload
buffers and up to two render/readback targets are reused. The queue holds two
jobs; callers fall back to CPU when busy, unavailable or unsuccessful. Device
errors disable the worker for the session. GPU waits have a two-second caller
deadline; targets are bounded to 1920×1080 and uploads to 192 MiB.

The initial automatic policy tries large previews and warms in the background;
steps 3–4 will finalize that policy and qualification. `MELT_PREVIEW_RENDERER`
accepts `auto` (default), `cpu` and `gpu`; explicit GPU mode still falls back on
failure. `MELT_PREVIEW_ADAPTER` optionally selects an adapter by name substring.
Automatic selection prefers integrated hardware and excludes CPU adapters;
forced GPU mode allows software adapters for diagnostics. CPU-only builds use
`--no-default-features`. Cache keys include the renderer mode and a new version.

Step 2 validation: CPU-only unit tests (44) pass. An explicitly invoked hardware
test passes on AMD integrated and NVIDIA discrete Vulkan adapters, comparing
asymmetric meshes and depth-shaded point clouds against the CPU at both normal
sizes and a padded-row size. It also checks target/buffer reuse, concurrent
requests and CPU recovery after simulated device loss. Ordinary tests never
require a GPU. Metal and DX12 have not been run on native hosts.

Run an uncached synthetic benchmark with:

```sh
cargo run -p melt-desktop --release --example preview_bench -- \
  --output /tmp/melt-preview-bench --iterations 3
```

It writes JSON stage timings and PNGs for a 12-triangle cube, a 65,536-triangle
sphere (100 mm radius) and a million-point cloud, at both production sizes.
Use `MELT_PREVIEW_PROFILE=1` to print local stage timings to stderr during normal
preview work. Cache hits and embedded images do not run the geometry stages.

Initial CPU baseline on the development Ryzen 9 7950X, optimized build, median
of three uncached renders (milliseconds; no GPU claims):

| Dataset | Grid total | Large total | Large CPU draw |
| --- | ---: | ---: | ---: |
| Small mesh | 8.00 | 59.35 | 55.72 |
| Dense mesh | 22.57 | 80.68 | 52.37 |
| Point cloud | 25.73 | 63.74 | 48.47 |

These measurements justify testing GPU rasterization, particularly for large
previews. Dense-mesh normal preparation and PNG encoding remain significant.
