# Bambu protocol fixtures

Sanitized evidence is grouped under `discovery`, `mqtt`, `tunnel-6000`, `ftps`,
and `camera`. Each evidence directory must contain a `manifest.json` validated
against `manifest.schema.json`. Raw captures are optional; decoded assertions
and executable transcripts are preferred. See
[`docs/bambu-protocol-evidence.md`](../../docs/bambu-protocol-evidence.md).

Run `make bambu-evidence-check` before committing a fixture. The check enforces
the manifest contract, unique/cross-referenced IDs, safe relative file paths,
referenced-file existence, UTF-8 textual summaries, and common credential,
private-key, user-path, private-IP, and MAC-address redaction failures.
