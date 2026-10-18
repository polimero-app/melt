# Documentation

Last-updated: 2026-10-18

These guides describe the checked-in Rust/Tauri implementation. The workspace,
frontend, and Tauri package declare version `0.1.0`; nightly installer versions
are supplied by the packaging workflow. Historical plans record design and
implementation decisions rather than the current command reference.

## Using Melt

| Guide | Contents |
| --- | --- |
| [Project overview](../README.md) | Features, quick start, capability boundaries, license |
| [CLI reference](cli.md) | Commands, arguments, confirmation, JSON and exit codes |
| [Desktop guide](desktop.md) | Printers, Control, Cameras, Configuration, File library |
| [Configuration](configuration.md) | File locations, schemas, credentials, backup and downgrade |
| [IP cameras](ip-cameras.md) | Tapo setup, streams, desktop playback, troubleshooting |
| [Smart plugs](smart-plugs.md) | Tapo P110 setup, attachments, power-off semantics, limitations |
| [Firmware evidence](firmware-updates.md) | Optional sources, provenance, caching and assessment |
| [Screenshots](screenshots.md) | Fresh sample-data captures of nine current views and reproducible capture instructions |

## Developing and qualifying Melt

| Guide | Contents |
| --- | --- |
| [Development](development.md) | Prerequisites, workspace architecture, build and test targets, CI |
| [CLI fixtures](cli-contract-fixtures.md) | Public command inventory, JSON goldens and response compatibility |
| [Bambu compatibility](bambu-compatibility.md) | Runtime transport, authorization, controls, print workflow |
| [Protocol evidence policy](bambu-protocol-evidence.md) | Evidence precedence, manifests and redaction |
| [Physical qualification](bambu-physical-qualification.md) | Hardware capture and quirk promotion procedure |
| [Qualification row](templates/bambu-qualification-row.md) | Template for an exact model/firmware and tested behavior |
| [Bambu fixtures](../fixtures/bambu/README.md) | Checked-in manifest-governed protocol evidence |
| [Tapo fixtures](../fixtures/tapo/README.md) | C510W RTSP captures and automated coverage |
| [Screenshot samples](../fixtures/screenshots/README.md) | Isolated sample IPC/media for the current UI captures |
| [Release qualification](release-qualification.md) | Tagged packages, nightlies and frozen manual evidence |
| [Packaging checklist](releases/TEMPLATE/release-packaging-checklist.md) | Template for release installation and hardware checks |
| [Updater report](releases/TEMPLATE/updater-verification-report.md) | Template; no application updater is configured |
| [Interface system](../.interface-design/system.md) | Visual language and monitoring/playback state rules |

## Implemented and planned scope

The shipped surfaces are a headless CLI and a Tauri desktop app in the same
`melt` executable. Two printer drivers are registered: `bambu-lan` and
`moonraker`. Automatic printer discovery currently uses Bambu SSDP/UDP;
Moonraker profiles are entered manually. The `tapo` camera driver supports
H.264 RTSP video and JPEG snapshots, independent camera profiles, printer
links, and desktop viewers.

Camera phases 1 and 2 are implemented. ONVIF pan/tilt (phase 3) and optional
WS-Discovery (phase 4) remain planned. A web-command proposal exists in the
working tree, but there is no implemented `melt web` command or browser-hosted
application service. The frontend development server and loopback camera
proxies do not provide that service. Firmware checks and installer delivery
also do not install printer firmware or update the application in place.

See [the design and plan index](superpowers/README.md) for implementation
status and links to the current guides. Automated coverage and physical
qualification are separate: Bambu has an executable storage transcript but no
completed physical model/firmware matrix in this repository; Tapo has scoped
C510W hardware 1.0 / firmware 1.3.5 RTSP evidence.
