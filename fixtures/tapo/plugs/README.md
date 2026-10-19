# Tapo smart plug evidence

Last-updated: 2026-10-18

Read-only login checks were run on three available P110s on 2026-10-18.
Relay control and attachment scenarios remain untested, so the compatibility
claim remains "P110 (qualification pending)". See
[smart plugs](../../../docs/smart-plugs.md).

Three units (A, B and C) are recorded here, one row each. The same saved
account credentials were used for independent login attempts; no relay command
was sent.

| Unit | Model | Hardware | Firmware | Region | Third-Party Compatibility | Protocol negotiated | Date |
|---|---|---|---|---|---|---|---|
| A | user-reported P110 | pending | pending | pending | pending | pending | 2026-10-18 |
| B | P110 | 1.0 | 1.1.3 Build 240523 Rel.175054 | pending | pending | pending | 2026-10-18 |
| C | user-reported P110 | pending | pending | pending | pending | pending | 2026-10-18 |

Unit B authenticated and returned device information. Units A and C returned
KLAP `HASH_MISMATCH` with the same credentials. Their model, hardware, and
firmware could not be read. This reproduces an authentication difference
between devices without involving Melt's profile save flow.

Coverage per unit: A covers authentication, device information, relay status
and on/off readback; B covers independent control and printer/camera
attachment scenarios; C covers concurrent sessions, shared attachments and
isolation from another plug's failure.

Only sanitized facts are stored. Real LAN addresses are replaced with the
documentation addresses `192.0.2.30` to `192.0.2.32`; credentials, tokens,
serials, MAC addresses and account identifiers never appear.
