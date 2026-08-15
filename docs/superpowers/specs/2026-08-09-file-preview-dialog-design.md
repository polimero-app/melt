# File preview dialog

Clicking a model thumbnail in the library file browser opens a dialog holding a
large version of that thumbnail, its metadata, and the same actions the card's
overflow menu offers.

## Scope

Only the library grid (`ui/src/App.vue`, the `visibleFiles` card list) gains the
affordance, and only for cards that render a real `ModelThumbnail` — files
matching `isModelFile` (`.3mf`, `.stl`, `.obj`, `.zip`). Non-model cards draw a
CSS placeholder shape with nothing to enlarge and stay inert.

Out of scope: the printer's on-device file table and the Control view's
current-job thumbnail, both of which also mount `ModelThumbnail`.

## Behaviour

Hovering (or keyboard-focusing) a model thumbnail dims it and centres a
magnifying-glass icon. Activating it opens a modal dialog containing:

- the filename as the dialog title, plus size and modified date
- the preview image, scaled to fit and letterboxed, static — no zoom or pan
- the file's actions (print, open with each enabled slicer, delete)
- a close button

Escape, a click on the backdrop, and the close button all dismiss it.

Choosing an action dismisses the dialog *before* running it. Delete opens the
existing `ConfirmDialog`; nesting one modal inside another makes Escape
ambiguous, so the preview closes out of the way first.

## Resolution

The preview shown in the grid is a 640×360 PNG. Blown up it is unusably soft, so
the dialog asks for a larger render — but only where a larger render exists to
be made:

| Source | Large view |
| --- | --- |
| `.3mf` with a slicer-embedded thumbnail | the embedded PNG, unchanged |
| `.stl`, `.obj`, `.3mf` without one, `.zip` | re-rasterized at 1920×1080 |

The embedded thumbnail is the slicer's own render, with plate, filament colours
and supports. Replacing it with our flat-shaded rasterization to gain sharpness
would be a downgrade, so we keep it at whatever size the slicer baked in and
decline to upscale past 100%.

The dialog opens immediately on the already-cached 640×360 preview and swaps to
the large render when it arrives, so there is no perceived wait.

## Rust

`crates/melt-desktop/src/preview.rs` hardcodes `WIDTH`/`HEIGHT` module
constants. They become parameters threaded through `rasterize`, `rasterize_3mf`,
`render` and `encode_png`, with two exported sizes replacing them:

```rust
pub const GRID_SIZE: (usize, usize) = (640, 360);
pub const LARGE_SIZE: (usize, usize) = (1920, 1080);
```

Both are 16:9, so the perspective projection frames the model identically at
either size — no camera retuning. 1920×1080 is 2.1 MP, well inside the existing
`MAX_PREVIEW_DIMENSION` (4096) and `MAX_PREVIEW_PIXELS` (16 M) guards that
`validate_png` enforces.

In `main.rs`, `LibraryPreviewRequest` gains `large: bool` (defaulting to false)
and `render_preview` gains a size argument. `printer_file_preview` passes
`GRID_SIZE` unconditionally.

Caching: the preview cache key gains a size tag. Large renders are held in the
in-memory `PreviewState` only and are not written to the on-disk thumbnail
cache — a 1920×1080 PNG is 1–3 MB against roughly 50 KB today, and the on-disk
cache has no eviction policy. Reopening a file within a session is instant;
across restarts it re-renders.

Large renders take a `RENDER_SEMAPHORE` permit like every other render, queueing
behind in-flight grid thumbnails rather than pre-empting them.

## Frontend

**`ui/src/preview.ts`** (new). `ModelThumbnail.vue` privately owns the preview
cache, the two-slot concurrency gate and the base64 decoder. The dialog needs
all three, so they move here behind `loadPreview(path, sizeBytes, modifiedAt,
large)`. `ModelThumbnail.vue` imports it and loses roughly thirty lines;
its behaviour is unchanged.

**`ui/src/components/FilePreviewDialog.vue`** (new). Built on HeadlessUI
`Dialog`, matching `ConfirmDialog.vue` — Escape, backdrop dismissal, focus trap
and focus restore all come from the library rather than a hand-rolled keydown
listener.

Props: `open`, the file's path/name/size/modified fields, a pre-formatted `meta`
string, `items: ActionMenuItem[]`, and labels. `formatSize` and `formatDate` are
locale-aware locals in `App.vue`, so `App.vue` formats and passes the result
down. `items` is the array `fileActionItems(file)` already builds for the card's
`ActionMenu`, rendered here as a button row.

If the large render fails, the dialog silently keeps the low-resolution image.
If both fail, it shows `filesView.previewUnavailable`.

**`ui/src/App.vue`**. The card's preview `<div>` becomes a `<button
type="button" class="group ...">` for model files — a real button, so keyboard
and touch both reach it, since a hover overlay alone reaches neither. It carries
the magnifier overlay and an `aria-label`. New `previewFile` ref drives the
dialog, mounted beside the existing `ConfirmDialog`.

**i18n.** One new key, `filesView.openPreview` ("Preview {name}" /
"Visualizar {name}"), added to the `MessageKey` union and both locales.
`common.close` and `filesView.previewUnavailable` already exist.

## Checks

- `ui/src/preview.test.ts` — a repeated `loadPreview` for the same path and size
  serves from cache without a second `invoke`; grid and large keys do not
  collide.
- `preview.rs` — rasterizing at `LARGE_SIZE` emits a PNG whose IHDR header reads
  1920×1080.
