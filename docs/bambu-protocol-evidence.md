# Bambu protocol evidence policy

Last-updated: 2026-10-12

Melt treats undocumented printer behavior as qualified evidence, not as a
permanent product truth. This policy covers identity aliases, capabilities,
firmware quirks, transport probes, and physical-printer qualification.

## Evidence and scope

When sources conflict, prefer: sanitized physical capture; executable
conformance test; implementation source tied to a commit; detailed protocol
documentation tied to a commit; then README, issue, or anecdotal report. A
higher-ranked source still applies only to the models, hardware revisions, and
module firmware it demonstrates.

Every checked-in fixture set has a `manifest.json` conforming to
`fixtures/bambu/manifest.schema.json`. It records provenance, source revision,
confidence, printer identity and module versions, sanitization, the supported
assertion, qualification steps, conflicts, and a removal condition.

Fixtures must not contain access codes, private keys, cloud tokens, Wi-Fi
credentials, user paths, IP/MAC addresses, or unsalted identifiers. Replace
serials consistently with documented test identifiers. Certificate fingerprints
are allowed only for generated test certificates. Inspect decoded and raw forms;
binary fixtures need a reproducible textual summary.

`make bambu-evidence-check` enforces the manifest contract and scans every
referenced textual artifact for common secret, private-key, user-path, private
network, and MAC-address forms. This is a conservative floor, not proof of
anonymity: contributors must still inspect semantic identifiers and decoded/raw
forms before commit. The integration test mirrors the manifest contract with
typed validation; it is not a general JSON Schema engine. It also rejects
symlinked artifacts, undeclared files and self/unknown conflict references.

## Checked-in evidence

There is currently one Bambu manifest:
`tunnel-6000-upload-frag-id`, under
`fixtures/bambu/tunnel-6000/upload-frag-id`. It is `executable-test` evidence
with `qualified` confidence, tied to upstream revision
`7f8ba03057f394e844a6a336414425786d0a68c1`. It demonstrates top-level upload
`frag_id` and typed/sequence-correlated replies using a synthetic device and
zeroed bytes. It does not qualify a physical printer or a firmware range.

`make bambu-evidence-check` validates manifests and sanitization. The transcript
is also exercised by the core unit test
`executable_upload_transcript_keeps_frag_id_top_level`, included in
`make cargo-test`. The three runtime camera quirks remain provisional. Tapo
SDP captures follow their [separate camera fixture notes](../fixtures/tapo/README.md),
not the Bambu manifest schema.

## Conflict handling and runtime precedence

Conflicting evidence remains visible and cross-referenced. Narrow the affected
rule instead of replacing an older fixture. P1S camera transport, for example,
must be selected from live `print.ipcam` fields or a safe probe until exact
firmware-scoped physical evidence establishes a default.

Runtime behavior resolves in this order:

1. fresh explicit printer observation;
2. successful non-mutating probe;
3. exact model, module, hardware, and firmware quirk;
4. exact-model default;
5. family hint;
6. unknown, retaining safe attempt-and-observe behavior.

Unknown identifiers remain operable. Every registry entry names its evidence,
qualification status, owner, and removal condition. Expired or contradicted
entries are diagnostic-only. Release evidence identifies the exact Melt
commit and requires informed consent for captures from printers Melt's
maintainers do not own.

## Firmware-update evidence boundary

Melt reads installed module versions from the LAN `info.get_version` response
and treats only qualified fields under `print.upgrade_state` as update
advertisements. Structured `new_ver_list` targets and the legacy named
`*_new_version_number` fields can establish that a target is advertised;
opaque numeric state values cannot.

No sanitized physical no-update capture is checked in yet. Consequently, an
empty or missing advertisement is reported as `unknown` (“No update
advertised”), never `current`. A future claim that Bambu firmware is current
requires a manifest-governed physical fixture that identifies the exact model,
module versions, full-versus-delta report shape, and clearing behavior. Tests
with synthetic documents exercise parser safety but do not raise this physical
qualification level.

Optional device history and public stable releases remain separate evidence
sources; they never turn an absent printer advertisement into a confirmed
offer or no-update state. Public catalogue requests are opt-in and bounded,
and do not use cloud account credentials. See
[firmware update evidence](firmware-updates.md) for the current contract.
