# Bambu protocol evidence policy

Polimero treats undocumented printer behavior as qualified evidence, not as a
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
forms before commit.

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
entries are diagnostic-only. Release evidence identifies the exact Polimero
commit and requires informed consent for captures from printers Polimero's
maintainers do not own.
