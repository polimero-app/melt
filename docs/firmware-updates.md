# Firmware update evidence

Melt's firmware check is read-only. It compares installed versions with
printer-local and optional public evidence, but it does not download a package,
start an upgrade, or log in to Bambu Cloud.

## Evidence sources

Each component carries the exact bounded version observations used to derive
the compatibility fields shown by older clients:

- `Installed` — Bambu `info.get_version` inventory.
- `PrinterAdvertised` — Bambu `print.upgrade_state`; this is the only source
  that can populate the legacy `availableVersion` or `required` fields.
- `DeviceCatalogue` — the read-only Bambu `upgrade.get_history` response.
- `PublicStable` — the latest stable-looking version from an allowlisted
  official Bambu model page, when explicitly enabled.
- `UpstreamCurrent` — the existing Moonraker Update Manager result.

The report retains `source` and `availability` for compatibility and adds an
overall `assessment`, `evidenceSources`, and per-component `evidence`. A public
or history version is never copied into `availableVersion` as if the printer
had offered it.

## CLI

The default is LAN-only and preserves the existing behavior:

```sh
melt firmware check workshop
```

Use `--include-history` to request the printer's read-only catalogue. Add
`--include-public-catalogue` to query the public stable page for a known Bambu
model. The public option is HTTPS-only, has a bounded response body, refuses
redirects, and never sends an access code, serial number, cookie, or token.

## Desktop

The Manage printers firmware screen shows installed, printer-advertised,
device-catalogue, and public-stable rails separately. History checks are
read-only and cached with the normal firmware result. **Refresh** re-runs the
check against the sources enabled under Settings → Firmware sources, so a
LAN-only setup never contacts Bambu. **Also check Bambu.com** is a one-shot
opt-in that adds public-catalogue evidence without changing that preference;
a failed optional source leaves the other evidence visible and records a
bounded issue.

A successful public-catalogue result is cached per printer model for 24 hours
and shared by every profile of that model, and concurrent checks for the same
model wait for a single request. The one-shot check is served from that cache
while it is valid, so repeatedly checking never repeatedly contacts Bambu.
Failures are not cached and are retried on the next check.

## Interpretation

Assessment priority is `Required`, `ConfirmedAvailable`, `Conflict`,
`DeviceCatalogueNewer`, `PublicReleaseNewer`, `Current`, then `Unknown`.
“Public release newer” means only that an official page has a newer stable
version than the installed inventory; it is not proof of regional rollout,
device eligibility, or a staged update on the printer.
