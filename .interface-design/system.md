# Polimero Interface System

## Direction and feel

Polimero is an operational workspace for people supervising physical printers. It should feel calm, precise, and trustworthy: telemetry remains stable while transports recover, and strong alarm language is reserved for states that require attention.

Domain vocabulary: printer heartbeat, live telemetry, retained sample, reconnecting transport, print progress, material path, and true outage.

The interface signature is status freshness. A printer's last observed machine state and the health of the current connection are related but distinct signals; never collapse both into a binary online/offline label.

## Visual foundations

- Depth: use quiet borders and subtle same-hue surface shifts. Avoid dramatic shadows or large elevation jumps.
- Spacing: use a 4 px base grid. Prefer existing Tailwind spacing increments and symmetrical component padding.
- Typography: Inter carries interface hierarchy and prose. Hack carries printer identifiers, paths, measurements, and other telemetry.
- Structure: graphite and steel neutrals organize surfaces. Cyan identifies actions and selection, not connectivity.
- Semantic color: green means a confirmed live sample; amber means retained data while reconnecting; red means no usable status or a terminal fault; muted gray means not yet observed or unknown.
- Dark mode: keep semantic colors slightly desaturated with translucent backgrounds and low-emphasis borders.

## Monitoring state pattern

Use this state model anywhere printer connectivity appears: navigation dots, status badges, printer cards, control headers, and fleet summaries.

| State | Data contract | Presentation | Operational treatment |
| --- | --- | --- | --- |
| Live idle | Current usable status; no polling error | Green, “Online · idle” | Count as online |
| Live busy | Current printing or paused status; no polling error | Yellow, “Online · busy” | Count as online |
| Reconnecting | Retained usable status plus `stale`, or a current polling error | Amber, “Reconnecting” | Keep last telemetry visible; do not count as confirmed online |
| Offline | No usable status with a polling error, or terminal error/unknown printer state | Red, “Offline” | Show recovery action and unavailable treatment |
| Unknown | No monitor entry or no observation and no error | Muted gray, “Unknown” | Remain neutral while awaiting the first sample |

Rules:

- Check whether usable status exists before allowing a transport error to determine the badge.
- A stale sample must retain its original machine state and observation timestamp.
- Never replace retained telemetry with an empty unavailable screen during a transient reconnect.
- Use a compact amber status notice with a manual refresh action when the active printer is reconnecting.
- Fleet “online” totals and destructive or dispatch eligibility require a confirmed live sample unless the backend independently revalidates connectivity.
- Use red only when Polimero has no usable printer state or receives a terminal printer state.

## Reusable implementation

- Centralize state derivation in `ui/src/monitoring.ts`; presentation surfaces consume its `PrinterBadge` result instead of reimplementing precedence rules.
- Keep badge colors and labels synchronized between `ui/src/App.vue`, `ui/src/components/StatusBadge.vue`, and localized messages.
- Add a table-driven or focused unit test whenever the monitoring state contract changes, especially around stale status plus error precedence.
