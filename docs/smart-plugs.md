# Smart plugs

Last-updated: 2026-10-18

Melt can switch a smart plug on and off and show its relay state. A plug is an
independent profile, like an IP camera: it can power several printers and
cameras, and a printer or camera can have several plugs. Control is explicit:
one plug, one state. There is no toggle, group, schedule, auto-off after a
print or energy data.

## Supported plugs

| Driver | Model | Qualified hardware / firmware | Control |
|---|---|---|---|
| `tapo` | TP-Link Tapo P110 | Qualification pending | On/off with readback |

P110 support is implemented, but relay control has not been qualified on
hardware. A read-only login succeeded on one of three available P110s; the
other two rejected the same account credentials. Until relay tests pass,
P110 (qualification pending) is the only claim. Melt accepts only a device
that reports the exact model `P110`; no power command
is sent to another model: it fails as `capability_unsupported`,
or as `connection_failed` if its reply cannot be read. Evidence is tracked in
[the plug fixtures](../fixtures/tapo/plugs/README.md).

## Tapo setup

1. Add the plug to your TP-Link account in the Tapo app and give it a fixed
   LAN address (a DHCP reservation). If the address later answers as a
   different device, including another P110, Melt rejects its identity before
   sending a power command. It reads device information to check that identity.
2. The plug authenticates with your **TP-Link account** email and password,
   the full cloud account password. This is not the Tapo Camera Account used
   by [IP cameras](ip-cameras.md#tapo-setup). If your firmware shows a
   **Third-Party Compatibility** option, the driver can negotiate KLAP or
   TPAP with either setting. If a KLAP login is refused, check that setting
   in the Tapo app.
3. Melt must reach the plug directly on the LAN. It does not use the TP-Link
   cloud for control.
4. Save the password to a file only you can read (one line, a regular file,
   not a symlink, at most 4 KiB), as in the camera setup:

   ```sh
   (umask 077; touch ~/.config/melt/bench-power.pw)
   chmod 600 ~/.config/melt/bench-power.pw
   ```

5. Add the plug. Melt logs in and reads the device before saving; it
   never changes the relay during setup. A wrong password saves nothing and
   makes no second login attempt.

   ```sh
   melt plug add bench-power --driver tapo --host 192.0.2.30 \
     --username operator@example.invalid --password-file ~/.config/melt/bench-power.pw
   ```

The password is stored in the OS keychain under service `melt`, account
`plug:<id>:password`. It is never copied from, or shared with, a camera
password. The profile in `melt.yaml` holds the ID, driver, host, username
(case kept as entered), timeout, attachments and the verified physical
`device_id`, but no password and no observed model or firmware. The physical
identity is checked on every read, including the pre-read before a power
change and reads after reconnecting. It is omitted from public status and
list responses. See [configuration](configuration.md).

Profiles created by an earlier smart plug build may have no saved device
identity. They remain readable, and background polling does not enroll a
device. Check the address and connected load, then explicitly verify and save
the identity with `melt plug edit <plug> --password-file <path>`, or enter the
password again in the desktop edit form. Verification reads the device without
switching power. An existing identity cannot be replaced by editing the host
or password; to intentionally replace a physical plug, remove its profile,
add the replacement, and attach the required printers and cameras again.

## Attachments

A plug lists the printers and cameras it powers. Both kinds can share one
plug.

```sh
melt plug link bench-power --printer workshop
melt plug link bench-power --camera porch
melt plug list --printer workshop
```

Attachments describe power only. A plug linked to a printer is not linked to
that printer's cameras, and the reverse also holds. Camera-to-printer links
still describe video only. A printer's built-in camera needs no plug
attachment. Linking, unlinking, editing and removing never switch a relay.

Renaming a plug keeps its ID, links and keychain account. Renaming a printer
updates its name in every plug; renaming a camera changes nothing, because
links use the camera ID. Removing a printer or camera drops it from every
plug and keeps the plug profiles. Removing a plug drops its links and keeps
the printer and camera profiles.

## Switching power

```sh
melt plug status bench-power
melt plug on bench-power
melt plug off bench-power --yes
```

Turning a plug off cuts power immediately. It is not a print cancel, an
emergency stop or a graceful shutdown, and an active print on an attached
printer is lost. Both surfaces ask for confirmation first and name the plug
and every attached printer and camera. Turning a plug on needs no
confirmation.

- CLI: an interactive terminal must type `yes`. Without a terminal and
  without `--yes`, `melt plug off` exits 2 and makes no network request. The
  prompt lists attachments from the configuration only; the CLI makes no
  printer requests for it.
- Desktop: the dialog also shows printing, paused, heating or drying activity
  that monitoring already knows about for attached printers.

A power change reads the status first, sends the command, then reads the
status again, all within one deadline (the plug timeout, default `10s`,
including any wait for that plug's lock). The result is one of:

| Result | Meaning | CLI code (exit) |
|---|---|---|
| Confirmed | The readback equals the requested state | success (0) |
| Mismatch | The plug answered with the other state | `power_change_mismatch` (4) |
| Unconfirmed | Transport error, timeout or malformed answer during or after the send; the relay may or may not have changed | `power_change_unconfirmed` (4) |
| Rejected | The plug definitely refused the command (authentication, lockout, device error), so it was not applied | `authentication_failed`, `credential_mismatch`, `authentication_locked` or `device_rejected` |

Writes are never queued, replayed or retried. After an unconfirmed result,
check the actual relay with `melt plug status` (or the Smart plugs view)
before trying again. An unconfirmed result never claims the relay stayed as
it was. In JSON mode, mismatch and unconfirmed errors carry `error.details`
with `requestedState`, `lastObservedState`, `observedAt` and
`deliveryUncertain` (true only for unconfirmed). See the
[CLI reference](cli.md#smart-plugs).

Immediately before dispatch, Melt rechecks the profile after the pre-read.
The command keeps the plug ID, settings and attachments shown at confirmation:
an edit, attachment change, removal or replacement while the prompt is open,
during connection setup or during that read aborts it. The desktop does not
offer power-off confirmation if it cannot refresh the attachments.
A command already sent may still take effect. Concurrent profile saves
serialize their config and keychain changes, including rollback. Desktop
forms also reject changes made since the form opened; close and reopen a
conflicting form to review the newer settings before retrying.

If someone uses the Tapo app or the plug's button, the next read reports the
device's value; Melt never answers from a cache.

## Desktop

The **Smart plugs** view lists plug profiles with their attachments, relay
state, connection and observation time, and adds, edits, links and removes
them. The same On/Off control appears in two other places: on the printer's
**Control** view (including the offline screen) and under an external
camera's details. Each control also has a **Check status** button. See [Desktop](desktop.md#smart-plugs).

Relay state (on, off, unknown) is separate from connection health. When a plug
is unreachable the state is unknown and the last observation is shown as
stale. Printer reachability and camera playback never decide relay state.

The desktop reads every plug about every 5 seconds, with a per-plug backoff of
up to 60 seconds after failures. An authentication failure, credential
mismatch, or lockout stops automatic attempts for that plug until its profile
changes or you explicitly read status or switch power. This also applies when
these errors arise during a power command's readback: the power result stays
unconfirmed and automatic logins stop until an explicit retry or profile
change.

## Troubleshooting

| Symptom | Check |
|---|---|
| `authentication_failed` | The TP-Link account email and password (not the Camera Account); Third-Party Compatibility enabled |
| `credential_mismatch` | The plug rejected the account credential hash. Match the email's exact case. If the same credentials work on another plug, factory reset the affected plug, remove it from the app, and add it again via Bluetooth setup; see [Tapo troubleshooting](https://github.com/mihai-dinculescu/tapo/blob/v0.11.1/TROUBLESHOOTING.md#handshake-fails-with-a-hash-mismatch-invalid-credentials). |
| `authentication_locked` | Too many failed logins; wait, fix the password, then retry deliberately |
| `device_identity_unverified` | Check the plug's address, then edit it and enter the password to verify and save its identity |
| `device_identity_mismatch` | The address answers as another device; restore the original plug's address rather than replacing its saved identity |
| `secret_not_found` / `secret_store_failed` | The keychain entry is missing or the store is unavailable: run `melt plug edit <plug> --password-file ...` to store the password again |
| `connection_failed` / `timeout` | The plug is on the LAN, the address is correct, and the proxy settings below |
| `capability_unsupported` | The address answers as a model other than P110, or the protocol is unsupported (also after DHCP reassignment) |
| `device_rejected` | The plug refused the command; the relay was not changed |
| `power_change_unconfirmed` | Read status before retrying |

When removing a plug, Melt reports a warning if its keychain password could not
be deleted; delete the `plug:<id>:password` entry manually in that case.

## Security and `tapo` limitations

Melt talks directly to the configured LAN host. The plug driver uses the
`tapo` crate, pinned to `=0.11.1` with its `debug`, `python` and `json-schema`
features off; upgrades are deliberate. Melt never logs or shows raw device
errors, response bodies or authentication payloads.

### Accepted `tapo` 0.11.1 limitations

`tapo` builds its own `reqwest::Client` (`api_client.rs:1078`), and Melt
cannot replace it. Melt accepts this and documents it here:

- `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` apply to plug traffic. Users with a
  proxy should add the plug hosts to `NO_PROXY`.
- reqwest's default redirect policy applies. Melt still rejects hosts that
  contain a scheme, port, path or user info.
- TPAP-over-TLS accepts any certificate, because the device certificate is
  self-signed. There is no fingerprint pinning like the Bambu driver has.
- KLAP runs over plain HTTP with its own handshake cryptography.
- The plug password is the user's full TP-Link cloud account password.

## Downgrading Melt

Plug data needs configuration schema version 3. Before running an older Melt,
remove the plugs with the current version, or close Melt, back up `melt.yaml`,
delete the `plugs:` section and set `version` to 2 (cameras present) or 1.
Then delete the orphaned `plug:<id>:password` keychain entries. See
[configuration](configuration.md#backup-recovery-and-downgrade).
