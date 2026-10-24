# Agent instructions

## UI changes require before/after screenshots

For every PR that changes visible UI (layout, styling, text, assets, or
interactive states), use `scripts/capture_screenshots.py` to capture the
affected views **before and after** the change. Follow the
[capture and attachment procedure](docs/screenshots.md#required-ui-pr-evidence).

Embed the actual uploaded images, clearly labeled **Before** and **After**,
in the **PR description**, not just a comment. Local paths, Actions artifact
links, and screenshots from the checked-in gallery do not satisfy this rule.
Refresh the after captures whenever subsequent edits change the visible UI.

If capture or upload is blocked, report the blocker and leave the PR in draft
until the images are attached; do not claim the visual evidence is complete.
Changes with no visible UI effect do not require screenshots.

## Keep screenshots current

When a new screen is added or existing UI changes, update
`scripts/capture_screenshots.py` (scenarios, bridge IPC) and
`fixtures/screenshots/state.json`, run `make screenshots`, and commit the
refreshed `screenshots/*.png` and `capture-manifest.json`. Keep
`docs/screenshots.md` in sync. The script fails if a view has no scenario.
