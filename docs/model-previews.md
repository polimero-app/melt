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

Local files support STL, OBJ, 3MF, the first
supported model in ZIP, and PNG thumbnails in a G-code comment header.
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
- Chunks 3–7: pending.
