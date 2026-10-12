# Firmware update evidence

Last-updated: 2026-10-12

Melt's firmware check is read-only. It compares installed versions with
printer-local and optional public evidence, but it does not download a package,
start an upgrade, or log in to Bambu Cloud.

## Evidence sources

Each component carries the exact bounded version observations used to derive
the compatibility fields shown by older clients:

- `Installed` — Bambu `info.get_version` inventory.
- `PrinterAdvertised` — Bambu `print.upgrade_state`; for Bambu this is the only
  source that can populate legacy `availableVersion` or `required` fields.
- `DeviceCatalogue` — the read-only Bambu `upgrade.get_history` response.
- `PublicStable` — the latest stable-looking version from an allowlisted
  official Bambu model page, when explicitly enabled.
- `UpstreamCurrent` — the existing Moonraker Update Manager result.

The report retains `source` and `availability` for compatibility and adds an
overall `assessment`, `evidenceSources`, and per-component `evidence`. A public
or history version is never copied into `availableVersion` as if the printer
had offered it.

Moonraker retains its existing `availableVersion` for the Klipper updater and
records that observation as `UpstreamCurrent`. Component kinds distinguish
`printerFirmware`, `accessoryFirmware` and `printerSoftware`. JSON enum values
are camelCase (for example `printerAdvertised`, `bambuLanHistory`,
`confirmedAvailable`); the capitalized names here describe Rust variants.

Reports are bounded to 64 components and 16 issues, with 64-byte component IDs
and versions and 128-byte labels. Unknown modules stay visible when useful
version evidence exists. Issues describe malformed, conflicting or unavailable
sources without copying raw credentials or network payloads.

## CLI

The default is LAN-only and preserves the existing behavior:

```sh
melt firmware check workshop
```

Use `--include-history` to request the printer's read-only catalogue. Add
`--include-public-catalogue` to query the public stable page for a known Bambu
model. The public option is HTTPS-only, has a bounded response body, refuses
redirects, and never sends an access code, serial number, cookie, or token.
The response limit is 2 MiB, and requests are restricted to `bambulab.com`,
`www.bambulab.com` and `wiki.bambulab.com`. The model-to-page allowlist lives in
`crates/melt-core/src/bambu/public_catalogue.rs`; unknown models are not routed
to a similar printer's page.

Public checks try the official firmware download page first. If it cannot be
fetched or has no usable version, A1 Mini and H2C checks also try their official
Bambu Wiki release histories:

- [A1 Mini firmware release history](https://wiki.bambulab.com/en/a1-mini/manual/a1-mini-firmware-release-history)
- [H2C firmware release history](https://wiki.bambulab.com/en/h2c/manual/h2c-firmware-release-history)

Wiki checks verify the model in the page title and read release headings,
excluding beta releases and accessory or application versions in the notes.
The internal public-release result retains the URL that supplied the version;
the public report's per-component evidence contains source, role, version and
required status, rather than a URL field. The same request limits and cache
policy apply to both sources.

For Moonraker, the ordinary check reads Update Manager status and `--refresh`
requests metadata refresh only. It reports the configured `klipper` entry,
not MCU firmware or all host packages. Busy-printer/updater refresh rejection
is a classified operation error. A missing Update Manager or unsupported
Klipper updater reports `capability_unsupported` with exit 5. Optional Bambu
history/public flags do not add those sources to Moonraker.

## Desktop

The **Printers → Firmware & software** screen shows installed, printer-advertised,
device-catalogue, and public-stable rails separately. History checks are
read-only and cached with the normal firmware result. **Refresh** re-runs the
check against the sources enabled under **Configuration → Firmware sources**, so a
LAN-only setup never contacts Bambu. **Also check Bambu.com** is a one-shot
opt-in that adds public-catalogue evidence without changing that preference;
a failed optional source leaves the other evidence visible and records a
bounded issue.

Bambu publishes firmware per model, so the one-shot check needs a known model.
The desktop uses the configured model or derives one from the TLS-bound serial,
with discovery as an advisory fallback. Discovery can fill the model when
adding a printer; a hand-added/unknown printer may still need a model set on
the printer form. Runtime authenticated model observations remain authoritative
for capability detection and the report.

Desktop result caching is separate from printer health: automatic checks begin
after ten seconds and recur every 30 minutes, without changing the five-second
status cadence. Passive view loads can reuse a fresh 30-minute result;
explicit refresh requests a new provider check. A failed check retains the last
useful report with an error; reports older than 24 hours are marked stale.
The cache is process-local and invalidated by connection/profile generations.

A successful public-catalogue result is cached in process per printer model for 24 hours
and shared by every profile of that model, and concurrent checks for the same
model wait for a single request. The one-shot check is served from that cache
while it is valid, so repeatedly checking never repeatedly contacts Bambu.
Failures are not cached and are retried on the next check.

## Interpretation

Assessment priority is `Required`, `ConfirmedAvailable`, `Conflict`,
`DeviceCatalogueNewer`, `PublicReleaseNewer`, then the base availability
(`Current`, `Unknown` or `Unsupported`).
“Public release newer” means only that an official page has a newer stable
version than the installed inventory; it is not proof of regional rollout,
device eligibility, or a staged update on the printer.

No physical Bambu no-update capture is checked in. Missing/empty printer
advertisements therefore remain `unknown`; matching optional public/history
versions do not prove that the printer is current. Moonraker can report
`current` from its explicit upstream version/hash status. The assessment and
the legacy availability can differ: a public-only newer release can have
`assessment: publicReleaseNewer` while `availability` remains `unknown`.
An optional-source failure becomes a bounded issue without erasing installed
inventory or positive evidence from other sources.

See [Bambu evidence policy](bambu-protocol-evidence.md#firmware-update-evidence-boundary)
and [physical qualification](bambu-physical-qualification.md) before promoting
a model/firmware-specific no-update interpretation.
