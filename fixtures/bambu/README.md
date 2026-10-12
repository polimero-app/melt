# Bambu protocol fixtures

Last-updated: 2026-10-12

Sanitized evidence is grouped under `discovery`, `mqtt`, `tunnel-6000`, `ftps`,
and `camera`. Each evidence directory must contain a `manifest.json` validated
against `manifest.schema.json`. Raw captures are optional; decoded assertions
and executable transcripts are preferred. See
[`docs/bambu-protocol-evidence.md`](../../docs/bambu-protocol-evidence.md).

Run `make bambu-evidence-check` before committing a fixture. The check enforces
the manifest contract, unique/cross-referenced IDs, safe relative file paths,
referenced-file existence, UTF-8 textual summaries, and common credential,
private-key, user-path, private-IP, and MAC-address redaction failures.

Physical captures must also follow the staged safety, capture, sanitization,
review, and quirk-promotion procedure in
[`docs/bambu-physical-qualification.md`](../../docs/bambu-physical-qualification.md).

## Current fixture set

| Evidence ID | Provenance | Scoped assertion |
| --- | --- | --- |
| `tunnel-6000-upload-frag-id` | Executable test, upstream revision `7f8ba03057f394e844a6a336414425786d0a68c1`, captured 2026-08-04 | Upload `frag_id` is top-level; replies match command type and sequence |

The transcript uses a synthetic device and zeroed file bytes. Its qualified
confidence applies to that executable assertion, not a physical model/firmware
pair. Discovery, MQTT, FTPS and camera are supported evidence categories but
currently have no checked-in manifests. No physical Bambu no-update capture
or completed qualification row is present.

`make bambu-evidence-check` runs the manifest/redaction integration test. The
core unit test also exercises the transcript:

```sh
cargo test -p melt-core executable_upload_transcript_keeps_frag_id_top_level --locked
```

The integration test mirrors the schema with typed validation, checks every
artifact is declared, rejects symlinks and verifies conflict references. It is
not a general-purpose JSON Schema validator. Keep changes scoped to one
assertion per directory. Tapo fixtures are maintained separately under
[`fixtures/tapo`](../tapo/README.md).
