# Bambu qualification row

Last-updated: 2026-10-12

Template only. Follow [physical qualification](../bambu-physical-qualification.md),
record actual observations, and resolve not-applicable checks with a reason.
Automated parser success alone is not a physical-printer pass.

Melt commit:
Operator and UTC date:
Owner/consent record (private location or self-owned statement):
Marketed model:
Observed raw/canonical model:
Hardware configuration (no serials):
Firmware modules and versions:
Network/security mode:
Evidence manifest IDs:
Automated qualification workflow:
Tested OS/webview and playback transport:

## Results

- [ ] Discovery/identity and configured-host suggestion verified.
- [ ] TLS serial identity, pin, authentication, and reconnect verified.
- [ ] Full/delta status and explicit authorization evidence captured.
- [ ] One shared camera owner and bounded multi-viewer fan-out verified.
- [ ] Camera negotiation, timestamps, reconnect, and codec change verified.
- [ ] WebRTC/WebCodecs/MJPEG applicability, viewer isolation, hide/restore, last-viewer teardown and redacted playback diagnostics recorded.
- [ ] Storage roots, port 6000 correlation/bounds, and transfer integrity verified.
- [ ] Printer preview and sliced 3MF/package limits verified.
- [ ] AMS/nozzle mapping cases verified or marked not applicable with reason.
- [ ] Print start observed exactly once; pause/resume/cancel verified.
- [ ] Safe controls and emergency handling verified.
- [ ] AMS drying telemetry, capability gating and observed start/stop verified or marked not applicable with reason.
- [ ] Firmware inventory/advertisements, optional history/public sources and no-update evidence boundary verified.
- [ ] Authentication, TLS, timeout, storage, collision, rejection, and unsupported failures verified.
- [ ] Sanitized fixtures pass `make bambu-evidence-check`.
- [ ] Independent reviewer compared private raw and committed decoded evidence.

## Exceptions and not-applicable items

Record the item, reason, safety constraint, and impact on qualification scope.

## Conclusion

Qualified model/module firmware range:
Qualified behaviors:
Still provisional or unsupported:
Reviewer and UTC date:
