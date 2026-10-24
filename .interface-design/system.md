# Melt Interface System

Last-updated: 2026-10-18

## Direction and feel

Melt is an operational workspace for people supervising physical printers. It should feel calm, precise, and trustworthy: telemetry remains stable while transports recover, and strong alarm language is reserved for states that require attention.

Domain vocabulary: printer heartbeat, live telemetry, retained sample, reconnecting transport, print progress, material path, and true outage.

The interface signature is status freshness. A printer's last observed machine state and the health of the current connection are related but distinct signals; never collapse both into a binary online/offline label.

## Visual foundations

- Depth: use quiet borders and subtle same-hue surface shifts. Avoid dramatic shadows or large elevation jumps.
- Spacing: use a 4 px base grid. Prefer existing Tailwind spacing increments and symmetrical component padding.
- Typography: Inter carries interface hierarchy and prose. Hack carries printer identifiers, paths, measurements, and other telemetry.
- Control-card labels use medium weight without inherited light-weight body text; fault severity and firmware metadata chips use at least `text-xs` (12 px).
- Filament swatches expose a localized approximate color name and exact RGB hex in both a tooltip and screen-reader text. Missing or invalid colors are labeled unknown, not as the visual placeholder gray.
- Structure: graphite and steel neutrals organize surfaces. Cyan identifies actions and selection, not connectivity.
- Semantic color: green means a confirmed live sample; amber means retained data while reconnecting; red means no usable status or a terminal fault; muted gray means not yet observed or unknown.
- Dark mode: keep semantic colors slightly desaturated with translucent backgrounds and low-emphasis borders.

## Monitoring state pattern

Use this state model anywhere printer connectivity appears: navigation dots, status badges, printer cards, control headers, and fleet summaries.

| State | Data contract | Presentation | Operational treatment |
| --- | --- | --- | --- |
| Connecting | `connectionState: connecting`; initial transport setup | Muted gray, “Connecting” | Await a usable sample |
| Synchronizing | `connectionState: synchronizing`, or recovering without retained status | Muted gray, “Synchronizing” | Await authenticated status |
| Live idle | Current usable status; no polling error | Green, “Online · idle” | Count as online |
| Live busy | Current printing or paused status; no polling error | Yellow, “Online · busy” | Count as online |
| Printer error | Usable status whose machine state is `error` | Red, “Error” | Preserve telemetry and show actionable printer alerts |
| Reconnecting | Recovering with retained status, or nonterminal retained status plus `stale`/polling error | Amber, “Reconnecting” | Keep last telemetry visible; do not count as confirmed online |
| Offline | Explicit offline connection, no usable status with a polling error, or machine state `unknown` | Red, “Offline” | Show recovery action and unavailable treatment |
| Unknown | No monitor entry or no observation and no error | Muted gray, “Unknown” | Remain neutral while awaiting the first sample |

Rules:

- Check whether usable status exists before allowing a transport error to determine the badge.
- Honor explicit connection-state precedence in `monitorBadge`: connecting, synchronizing, recovering and offline are resolved before machine-state fallback. A live printer error is distinct from an offline transport.
- A stale sample must retain its original machine state and observation timestamp.
- Never replace retained telemetry with an empty unavailable screen during a transient reconnect.
- Use a compact amber status notice with a manual refresh action when the active printer is reconnecting.
- Fleet “online” totals and destructive or dispatch eligibility require a confirmed live sample unless the backend independently revalidates connectivity.
- Use red only when Melt has no usable printer state or receives a terminal printer state.

## Camera state pattern

- Section navigation labels are Printers, Cameras, Configuration and Files (the File library view); selecting a printer opens Control. The earlier “Manage printers” label remains only in historical plans.
- Smart plug state mirrors this split: relay (on, off, unknown) is separate from connection and observation time. An unreachable plug shows its last relay state as stale, never as current. Plug control never changes a printer's or camera's health, and the relay text changes only after the device confirms it.
- Plug Off always confirms by naming the plug and every attached printer and camera, because it cuts power immediately; On needs no confirmation. Disable both buttons only for the plug whose command is running.
- Camera errors stay in the viewer and never change the printer's health. An external camera can remain live while its linked printer is offline.
- The live badge reports the actual playback: `LIVE · WebRTC`, `LIVE · H.264` (WebCodecs), or amber `LIVE · MJPEG`. Amber describes the more expensive fallback, not a disconnected feed.
- WebCodecs becomes successful only after a decoded frame. Keep bounded fallback reasons available to diagnostics after the viewer closes; redact source identifiers and endpoints.
- Changing a feed or hiding/leaving a view releases only its viewer subscription. Quality is a viewer choice, not a profile setting.

## Reusable implementation

- Use a 68 px minimum rail (`min-h-17`) for shared card headers, whether they contain actions or text only. Keep 44 px `IconButton` hit targets intact and normalize height in `CardHeader.vue` instead of overriding individual cards.
- Centralize state derivation in `ui/src/monitoring.ts`; presentation surfaces consume its `PrinterBadge` result instead of reimplementing precedence rules.
- Keep badge colors and labels synchronized between `ui/src/App.vue`, `ui/src/components/StatusBadge.vue`, and localized messages.
- Add a table-driven or focused unit test whenever the monitoring state contract changes, especially around stale status plus error precedence.
- When `content-visibility` defers cards in large collections, restore normal painting on the active card with `:focus-within` and the component's explicit open state. This lets menus and popovers escape the card boundary without giving up off-screen rendering savings for the rest of the collection.
- Primary buttons use cyan-700 with white text in light mode and cyan-500 with gray-950 text in dark mode; operational telemetry remains legible in both themes.
- Keep file refresh errors visible above retained entries, suppress contradictory plate totals, and show a loading state when a large preview has no cached image yet.
- Minimum native window size is 1280×800; desktop CSS breakpoints are 480/640/720/840/1400 px. Check the smaller effective CSS viewport at OS scaling instead of assuming the native pixel width.
