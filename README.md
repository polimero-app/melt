# Polimero Tauri migration

This directory is the parallel Rust/Tauri implementation. The Go application
remains the released implementation until this workspace passes its CLI and GUI
compatibility gates.

- `crates/polimero-core` contains UI-agnostic behavior.
- `crates/polimero-cli` adapts that behavior to the `polimero` CLI contract.
- `crates/polimero-desktop` selects CLI mode for arguments and starts Tauri for
  a no-argument launch.
- `ui` contains the Vue, TypeScript, Tailwind CSS, and Headless UI frontend.

