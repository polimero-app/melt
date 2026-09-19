//! CLI rendering for shared Melt operations.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{IsTerminal, Read, Write},
    net::{IpAddr, Shutdown, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use melt_core::{
    AppError, app_info, bambu,
    config::{Config, ConfigError, NamedProfile, config_dir},
    diagnostics,
    drivers::{self, DriverError},
    firmware_updates::{
        FirmwareUpdateAvailability, FirmwareUpdateComponentKind, FirmwareUpdateReport,
    },
    keychain::{SERVICE, SecretError, SecretStore, SystemKeychain, account},
    moonraker,
    profiles::{self, ProfileError},
    trace::JsonlTracer,
};
use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Clone, Copy)]
enum OutputFormat {
    Human,
    Json,
}

struct CommandGroup {
    path: &'static [&'static str],
    short: &'static str,
    commands: &'static [(&'static str, &'static str)],
}

struct LeafCommand {
    path: &'static [&'static str],
    short: &'static str,
    args: &'static str,
    flags: &'static str,
}

const LEAF_COMMANDS: &[LeafCommand] = &[
    LeafCommand {
        path: &["camera", "snapshot"],
        short: "Capture one still image from a printer camera",
        args: "<name> [flags]",
        flags: "  -h, --help                    help for snapshot\n      --insecure                skip TLS fingerprint verification for this invocation\n      --overwrite               replace an existing destination file\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --to string               destination file path or directory",
    },
    LeafCommand {
        path: &["camera", "stream"],
        short: "Stream camera feed from a printer via a local HTTP server",
        args: "<name> [flags]",
        flags: "      --format string           output format for stream: mjpeg (transcode H.264 to MJPEG for browser viewing)\n  -h, --help                    help for stream\n      --insecure                skip TLS fingerprint verification for this invocation\n      --port int                local HTTP server port (default 8080)\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          auto-stop after this duration (e.g. 30m)",
    },
    LeafCommand {
        path: &["emergency-stop"],
        short: "Immediately halt all printer motion and heating",
        args: "<printer> [flags]",
        flags: "  -h, --help                    help for emergency-stop\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)",
    },
    LeafCommand {
        path: &["fans", "set"],
        short: "Set fan speed percentage on a printer",
        args: "<printer> <fan> <percent> [flags]",
        flags: "  -h, --help                    help for set\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --yes                     skip interactive confirmation",
    },
    LeafCommand {
        path: &["firmware", "check"],
        short: "Check for firmware and printer-software updates",
        args: "<printer> [flags]",
        flags: "  -h, --help                    help for check\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --refresh                 request fresh provider metadata without installing anything\n      --timeout string          override the profile connection timeout (e.g. 10s)",
    },
    LeafCommand {
        path: &["files", "delete"],
        short: "Permanently delete a file from printer storage",
        args: "<printer> <device-path>... [flags]",
        flags: "  -h, --help                    help for delete\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --yes                     skip interactive confirmation",
    },
    LeafCommand {
        path: &["files", "download"],
        short: "Download a file from printer storage",
        args: "<printer> <device-path>... [flags]",
        flags: "  -h, --help                    help for download\n      --insecure                skip TLS fingerprint verification for this invocation\n      --overwrite               allow overwriting existing destination files\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --to string               destination file or directory (directory required for multiple files)",
    },
    LeafCommand {
        path: &["files", "list"],
        short: "List files on printer storage",
        args: "<printer> [<device-path>...] [flags]",
        flags: "  -h, --help                    help for list\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --recursive               recursively list directory contents\n      --timeout string          override the profile connection timeout (e.g. 10s)",
    },
    LeafCommand {
        path: &["files", "roots"],
        short: "List storage roots available on a printer",
        args: "<printer> [flags]",
        flags: "  -h, --help                    help for roots\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)",
    },
    LeafCommand {
        path: &["files", "upload"],
        short: "Upload a file to printer storage",
        args: "<printer> <local-path> <device-path> [flags]",
        flags: "  -h, --help                    help for upload\n      --insecure                skip TLS fingerprint verification for this invocation\n      --overwrite               allow overwriting existing device file\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)",
    },
    LeafCommand {
        path: &["jobs", "cancel"],
        short: "Cancel the active or paused print job",
        args: "<printer> [flags]",
        flags: "  -h, --help                    help for cancel\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --yes                     skip interactive confirmation",
    },
    LeafCommand {
        path: &["jobs", "pause"],
        short: "Pause the active print job",
        args: "<printer> [flags]",
        flags: "  -h, --help                    help for pause\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --yes                     skip interactive confirmation",
    },
    LeafCommand {
        path: &["jobs", "preflight"],
        short: "Inspect and validate a local sliced 3MF package",
        args: "<local-path> [flags]",
        flags: "      --display-name string   job name shown by the printer\n  -h, --help                  help for preflight\n      --plate int             plate index within the sliced package",
    },
    LeafCommand {
        path: &["jobs", "resume"],
        short: "Resume a paused print job",
        args: "<printer> [flags]",
        flags: "  -h, --help                    help for resume\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --yes                     skip interactive confirmation",
    },
    LeafCommand {
        path: &["jobs", "start"],
        short: "Start a print job from a file on printer storage",
        args: "<printer> <device-path> [flags]",
        flags: "  -h, --help                    help for start\n      --insecure                skip TLS fingerprint verification for this invocation\n      --plate int               plate/sub-file index within a multi-plate file\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --skip-leveling           skip automatic bed leveling\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --yes                     skip interactive confirmation",
    },
    LeafCommand {
        path: &["lights", "set"],
        short: "Set light state (on/off) on a printer",
        args: "<printer> <light> <state> [flags]",
        flags: "  -h, --help                    help for set\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --yes                     skip interactive confirmation",
    },
    LeafCommand {
        path: &["motion", "home"],
        short: "Home printer axes",
        args: "<printer> [flags]",
        flags: "      --axis string             comma-separated axes to home: x,y,z (default: all)\n  -h, --help                    help for home\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --yes                     skip interactive confirmation",
    },
    LeafCommand {
        path: &["motion", "jog"],
        short: "Jog printer axes by a relative distance",
        args: "<printer> [flags]",
        flags: "      --feedrate int            move speed in mm/min (default 1500)\n  -h, --help                    help for jog\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --x float                 relative X-axis move in mm (range: -10 to 10)\n      --y float                 relative Y-axis move in mm (range: -10 to 10)\n      --yes                     skip interactive confirmation\n      --z float                 relative Z-axis move in mm (range: -10 to 10)",
    },
    LeafCommand {
        path: &["printer", "add"],
        short: "Add a printer profile",
        args: "<name> [flags]",
        flags: "      --access-code-file string   file containing the access code\n      --driver string             driver name (e.g. bambu-lan)\n  -h, --help                      help for add\n      --host string               printer IP or hostname\n      --insecure                  skip TLS verification and auth check\n      --model string              printer model reported by discovery\n      --protocol-trace string     write protocol diagnostics to this file (JSON Lines)\n      --serial string             printer serial number (required by some drivers)\n      --timeout string            connection timeout (default \"10s\")",
    },
    LeafCommand {
        path: &["printer", "capabilities"],
        short: "Inspect observed Bambu model, firmware, transport, and quirk evidence",
        args: "<name> [flags]",
        flags: "  -h, --help                    help for capabilities\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)",
    },
    LeafCommand {
        path: &["printer", "discover"],
        short: "Scan the local network for printers (mDNS, SSDP, UDP broadcast)",
        args: "[flags]",
        flags: "      --driver string           restrict discovery to a specific driver\n  -h, --help                    help for discover\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          scan duration (default 5s) (default \"5s\")",
    },
    LeafCommand {
        path: &["printer", "drivers"],
        short: "List available printer drivers",
        args: "[flags]",
        flags: "  -h, --help   help for drivers",
    },
    LeafCommand {
        path: &["printer", "list"],
        short: "List configured printer profiles",
        args: "[flags]",
        flags: "  -h, --help   help for list",
    },
    LeafCommand {
        path: &["printer", "remove"],
        short: "Remove a printer profile",
        args: "<name> [flags]",
        flags: "  -h, --help   help for remove\n      --yes    skip interactive confirmation",
    },
    LeafCommand {
        path: &["printer", "tls", "refresh"],
        short: "Re-pin or disable TLS certificate for a printer profile",
        args: "<name> [flags]",
        flags: "  -h, --help                    help for refresh\n      --insecure                disable TLS verification for this profile\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --yes                     skip interactive confirmation",
    },
    LeafCommand {
        path: &["speed", "set"],
        short: "Set active print speed profile on a printer",
        args: "<printer> <profile> [flags]",
        flags: "  -h, --help                    help for set\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --yes                     skip interactive confirmation",
    },
    LeafCommand {
        path: &["status"],
        short: "Show the current status of a printer",
        args: "<name> [flags]",
        flags: "      --detailed                include extended telemetry (fans, time, speed, AMS, etc.)\n  -h, --help                    help for status\n      --insecure                skip TLS fingerprint verification for this invocation\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)",
    },
    LeafCommand {
        path: &["temperature", "set"],
        short: "Set heater target temperatures on a printer",
        args: "<printer> [flags]",
        flags: "      --bed float               bed target temperature in Celsius (0 turns off)\n      --chamber float           chamber target temperature in Celsius (0 turns off)\n  -h, --help                    help for set\n      --insecure                skip TLS fingerprint verification for this invocation\n      --nozzle float            nozzle target temperature in Celsius (0 turns off)\n      --protocol-trace string   write protocol diagnostics to this file (JSON Lines)\n      --timeout string          override the profile connection timeout (e.g. 10s)\n      --yes                     skip interactive confirmation",
    },
];

const COMMAND_GROUPS: &[CommandGroup] = &[
    CommandGroup {
        path: &[],
        short: "CLI for interacting with 3D printers",
        commands: &[
            ("camera", "Camera operations on a named printer"),
            (
                "completion",
                "Generate the autocompletion script for the specified shell",
            ),
            (
                "emergency-stop",
                "Immediately halt all printer motion and heating",
            ),
            ("fans", "Fan control operations on a named printer"),
            ("firmware", "Firmware and printer-software update checks"),
            ("files", "File operations on a named printer"),
            ("help", "Help about any command"),
            ("jobs", "Job control operations on a named printer"),
            ("lights", "Light control operations on a named printer"),
            ("motion", "Motion control operations on a named printer"),
            ("printer", "Manage 3D printer profiles"),
            ("speed", "Print speed control operations on a named printer"),
            ("status", "Show the current status of a printer"),
            (
                "temperature",
                "Temperature control operations on a named printer",
            ),
        ],
    },
    CommandGroup {
        path: &["camera"],
        short: "Camera operations on a named printer",
        commands: &[
            ("snapshot", "Capture one still image from a printer camera"),
            (
                "stream",
                "Stream camera feed from a printer via a local HTTP server",
            ),
        ],
    },
    CommandGroup {
        path: &["fans"],
        short: "Fan control operations on a named printer",
        commands: &[("set", "Set fan speed percentage on a printer")],
    },
    CommandGroup {
        path: &["firmware"],
        short: "Firmware and printer-software update checks",
        commands: &[("check", "Check for available updates")],
    },
    CommandGroup {
        path: &["files"],
        short: "File operations on a named printer",
        commands: &[
            ("delete", "Permanently delete a file from printer storage"),
            ("download", "Download a file from printer storage"),
            ("list", "List files on printer storage"),
            ("roots", "List storage roots available on a printer"),
            ("upload", "Upload a file to printer storage"),
        ],
    },
    CommandGroup {
        path: &["jobs"],
        short: "Job control operations on a named printer",
        commands: &[
            ("cancel", "Cancel the active or paused print job"),
            ("pause", "Pause the active print job"),
            (
                "preflight",
                "Inspect and validate a local sliced 3MF package",
            ),
            ("resume", "Resume a paused print job"),
            ("start", "Start a print job from a file on printer storage"),
        ],
    },
    CommandGroup {
        path: &["lights"],
        short: "Light control operations on a named printer",
        commands: &[("set", "Set light state (on/off) on a printer")],
    },
    CommandGroup {
        path: &["motion"],
        short: "Motion control operations on a named printer",
        commands: &[
            ("home", "Home printer axes"),
            ("jog", "Jog printer axes by a relative distance"),
        ],
    },
    CommandGroup {
        path: &["printer"],
        short: "Manage 3D printer profiles",
        commands: &[
            ("add", "Add a printer profile"),
            (
                "capabilities",
                "Inspect observed Bambu model, firmware, transport, and quirk evidence",
            ),
            (
                "discover",
                "Scan the local network for printers (mDNS, SSDP, UDP broadcast)",
            ),
            ("drivers", "List available printer drivers"),
            ("list", "List configured printer profiles"),
            ("remove", "Remove a printer profile"),
            ("tls", "Manage TLS settings for a printer profile"),
        ],
    },
    CommandGroup {
        path: &["printer", "tls"],
        short: "Manage TLS settings for a printer profile",
        commands: &[(
            "refresh",
            "Re-pin or disable TLS certificate for a printer profile",
        )],
    },
    CommandGroup {
        path: &["speed"],
        short: "Print speed control operations on a named printer",
        commands: &[("set", "Set active print speed profile on a printer")],
    },
    CommandGroup {
        path: &["temperature"],
        short: "Temperature control operations on a named printer",
        commands: &[("set", "Set heater target temperatures on a printer")],
    },
];

fn validate_known_file_root(value: &str) -> Result<(), AppError> {
    let (root, _) = value
        .split_once(':')
        .ok_or_else(|| AppError::usage("device path must include a file root"))?;
    if !["gcodes", "sdcard"].contains(&root) {
        return Err(AppError::usage(format!("unsupported file root {root:?}")));
    }
    Ok(())
}

struct Invocation<'a> {
    command: Vec<&'a String>,
    format: OutputFormat,
}

#[derive(Default)]
struct ParsedOptions {
    flags: BTreeSet<String>,
    values: BTreeMap<String, String>,
}

impl ParsedOptions {
    fn enabled(&self, name: &str) -> bool {
        self.flags.contains(name)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }
}

#[derive(Default)]
struct ConnectionOptions {
    timeout: Option<String>,
    insecure: bool,
    protocol_trace: Option<String>,
}

struct ResolvedPrinter {
    name: String,
    driver: drivers::Profile,
    driver_kind: drivers::Driver,
    access_code: Option<String>,
    tls_fingerprint: Option<String>,
    timeout: Duration,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Meta<'a> {
    command: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
}

thread_local! {
    static NETWORK_CALL_START: std::cell::Cell<Option<Instant>> =
        const { std::cell::Cell::new(None) };
}

/// `durationMs` is only reported for commands that reach the network, matching
/// the reference implementation.
fn mark_network_call() {
    NETWORK_CALL_START.with(|start| {
        if start.get().is_none() {
            start.set(Some(Instant::now()));
        }
    });
}

fn network_duration_ms() -> Option<u64> {
    NETWORK_CALL_START.with(|start| {
        start
            .get()
            .map(|started| started.elapsed().as_millis().min(u64::MAX.into()) as u64)
    })
}

fn meta(command: &str) -> Meta<'_> {
    Meta {
        command,
        duration_ms: network_duration_ms(),
    }
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
    code: &'a str,
    message: &'a str,
    details: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct Envelope<'a, T: Serialize> {
    ok: bool,
    data: Option<T>,
    error: Option<ErrorDetail<'a>>,
    meta: Meta<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionData {
    version: &'static str,
    license: &'static str,
    source_url: &'static str,
    commit: &'static str,
    go_version: &'static str,
    platform: String,
    modes: [&'static str; 2],
}

pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if let Some(exit_code) = run_bare_group(args, out, err) {
        return exit_code;
    }
    let invocation = match Invocation::parse(args) {
        Ok(invocation) => invocation,
        Err(error) => return write_error("melt", OutputFormat::Human, error, out, err),
    };
    let command = command_name(&invocation.command);

    match invocation.command.as_slice() {
        [version] if version.as_str() == "version" => write_success(
            "version",
            invocation.format,
            VersionData {
                version: app_info().version,
                license: app_info().license,
                source_url: app_info().source_url,
                commit: "unknown",
                go_version: "rust",
                platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
                modes: ["gui", "headless"],
            },
            |out| {
                writeln!(out, "melt version {}", app_info().version)?;
                writeln!(out, "License: {}", app_info().license)?;
                writeln!(out, "Source: {}", app_info().source_url)
            },
            out,
        ),
        [printer, list] if printer.as_str() == "printer" && list.as_str() == "list" => {
            match Config::load() {
                Ok(config) => write_printer_list(invocation.format, config.sorted_profiles(), out),
                Err(error) => write_error(
                    "printer list",
                    invocation.format,
                    config_error(error),
                    out,
                    err,
                ),
            }
        }
        [printer, drivers] if printer.as_str() == "printer" && drivers.as_str() == "drivers" => {
            write_drivers(invocation.format, out)
        }
        [printer, add, name, flags @ ..]
            if printer.as_str() == "printer" && add.as_str() == "add" =>
        {
            add_profile(invocation.format, name, flags, out, err)
        }
        [printer, capabilities, rest @ ..]
            if printer.as_str() == "printer" && capabilities.as_str() == "capabilities" =>
        {
            printer_capabilities(invocation.format, rest, out, err)
        }
        [status, rest @ ..] if status.as_str() == "status" => {
            printer_status(invocation.format, rest, out, err)
        }
        [firmware, check, rest @ ..]
            if firmware.as_str() == "firmware" && check.as_str() == "check" =>
        {
            firmware_check(invocation.format, rest, out, err)
        }
        [printer, remove, name] if printer.as_str() == "printer" && remove.as_str() == "remove" => {
            remove_profile(invocation.format, name, false, out, err)
        }
        [printer, remove, name, yes]
            if printer.as_str() == "printer"
                && remove.as_str() == "remove"
                && yes.as_str() == "--yes" =>
        {
            remove_profile(invocation.format, name, true, out, err)
        }
        [files, roots, rest @ ..] if files.as_str() == "files" && roots.as_str() == "roots" => {
            files_roots(invocation.format, rest, out, err)
        }
        [files, list, rest @ ..] if files.as_str() == "files" && list.as_str() == "list" => {
            files_list(invocation.format, rest, out, err)
        }
        [files, delete, rest @ ..] if files.as_str() == "files" && delete.as_str() == "delete" => {
            files_delete(invocation.format, rest, out, err)
        }
        [jobs, action, rest @ ..]
            if jobs.as_str() == "jobs"
                && matches!(action.as_str(), "start" | "pause" | "resume" | "cancel") =>
        {
            job_action(invocation.format, action, rest, out, err)
        }
        [jobs, preflight, rest @ ..]
            if jobs.as_str() == "jobs" && preflight.as_str() == "preflight" =>
        {
            jobs_preflight(invocation.format, rest, out, err)
        }
        [emergency_stop, rest @ ..] if emergency_stop.as_str() == "emergency-stop" => {
            emergency_stop_command(invocation.format, rest, out, err)
        }
        [temperature, set, rest @ ..]
            if temperature.as_str() == "temperature" && set.as_str() == "set" =>
        {
            temperature_set(invocation.format, rest, out, err)
        }
        [motion, home, rest @ ..] if motion.as_str() == "motion" && home.as_str() == "home" => {
            motion_home(invocation.format, rest, out, err)
        }
        [motion, jog, rest @ ..] if motion.as_str() == "motion" && jog.as_str() == "jog" => {
            motion_jog(invocation.format, rest, out, err)
        }
        [fans, set, rest @ ..] if fans.as_str() == "fans" && set.as_str() == "set" => {
            fan_set(invocation.format, rest, out, err)
        }
        [speed, set, rest @ ..] if speed.as_str() == "speed" && set.as_str() == "set" => {
            speed_set(invocation.format, rest, out, err)
        }
        [files, upload, rest @ ..] if files.as_str() == "files" && upload.as_str() == "upload" => {
            files_upload(invocation.format, rest, out, err)
        }
        [files, download, rest @ ..]
            if files.as_str() == "files" && download.as_str() == "download" =>
        {
            files_download(invocation.format, rest, out, err)
        }
        [camera, action, rest @ ..]
            if camera.as_str() == "camera" && matches!(action.as_str(), "snapshot" | "stream") =>
        {
            match action.as_str() {
                "snapshot" => camera_snapshot(invocation.format, rest, out, err),
                "stream" => camera_stream(invocation.format, rest, out, err),
                _ => unreachable!("camera action is matched above"),
            }
        }
        [lights, set, rest @ ..] if lights.as_str() == "lights" && set.as_str() == "set" => {
            lights_set(invocation.format, rest, out, err)
        }
        [printer, discover, rest @ ..]
            if printer.as_str() == "printer" && discover.as_str() == "discover" =>
        {
            discover_printers(invocation.format, rest, out, err)
        }
        [printer, tls, refresh, rest @ ..]
            if printer.as_str() == "printer"
                && tls.as_str() == "tls"
                && refresh.as_str() == "refresh" =>
        {
            refresh_tls(invocation.format, rest, out, err)
        }
        _ => write_error(
            "melt",
            invocation.format,
            AppError::usage(format!("unknown command {command:?} for \"melt\"")),
            out,
            err,
        ),
    }
}

fn run_bare_group(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Option<i32> {
    let mut path = Vec::new();
    let mut unknown_flag = None;
    let mut json_requested = false;
    let mut help_requested = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--output" => {
                let value = args.get(index + 1)?;
                json_requested |= value == "json";
                index += 2;
            }
            "--help" | "-h" => {
                help_requested = true;
                index += 1;
            }
            "--verbose" | "-v" => index += 1,
            argument if let Some(value) = argument.strip_prefix("--output=") => {
                json_requested |= value == "json";
                index += 1;
            }
            argument if argument.starts_with('-') => {
                unknown_flag.get_or_insert(argument);
                index += 1;
            }
            _ => {
                path.push(args[index].as_str());
                index += 1;
            }
        }
    }

    // `<leaf> --help` describes the command instead of running it, even when
    // positional arguments are already present.
    if help_requested
        && let Some(leaf) = LEAF_COMMANDS
            .iter()
            .filter(|leaf| path.starts_with(leaf.path))
            .max_by_key(|leaf| leaf.path.len())
    {
        return Some(write_leaf_help(leaf, out));
    }

    let group = COMMAND_GROUPS
        .iter()
        .find(|group| group.path == path.as_slice())?;
    if let Some(flag) = unknown_flag {
        let command = group_command_name(group);
        return Some(write_error(
            &command,
            if json_requested {
                OutputFormat::Json
            } else {
                OutputFormat::Human
            },
            AppError::usage(format!("unknown flag: {flag}")),
            out,
            err,
        ));
    }
    Some(write_group_help(group, out))
}

fn write_leaf_help(leaf: &LeafCommand, out: &mut dyn Write) -> i32 {
    let command = leaf.path.join(" ");
    let help = format!(
        "{}\n\nUsage:\n  melt {command} {}\n\nFlags:\n{}\n\nGlobal Flags:\n      --output string   output format: human or json (default \"human\")\n  -v, --verbose         show detailed progress output\n",
        leaf.short, leaf.args, leaf.flags
    );
    out.write_all(help.as_bytes()).map_or(1, |_| 0)
}

fn group_command_name(group: &CommandGroup) -> String {
    match group.path {
        [] => "melt".into(),
        path => path.join(" "),
    }
}

fn write_group_help(group: &CommandGroup, out: &mut dyn Write) -> i32 {
    let command = group_command_name(group);
    let mut help = format!("{}\n\nUsage:\n  melt", group.short);
    if !group.path.is_empty() {
        help.push(' ');
        help.push_str(&command);
    }
    help.push_str(" [command]\n\nAvailable Commands:\n");
    // Cobra pads names to the longest in the group, with 11 as the floor.
    let width = group
        .commands
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0)
        .max(11);
    for (name, description) in group.commands {
        help.push_str(&format!("  {name:<width$} {description}\n"));
    }
    if group.path.is_empty() {
        help.push_str(
            "\nFlags:\n  -h, --help            help for melt\n      --output string   output format: human or json (default \"human\")\n  -v, --verbose         show detailed progress output\n      --version         version for melt\n",
        );
        help.push_str(
            "\nMelt is free software under AGPL-3.0-only, without warranty.\nSource: https://github.com/polimero-app/melt\nLicense: https://www.gnu.org/licenses/agpl-3.0.html\n",
        );
    } else {
        help.push_str(&format!(
            "\nFlags:\n  -h, --help   help for {}\n\nGlobal Flags:\n      --output string   output format: human or json (default \"human\")\n  -v, --verbose         show detailed progress output\n",
            group.path.last().expect("non-root groups have a name")
        ));
    }
    help.push_str(&format!(
        "\nUse \"melt{} [command] --help\" for more information about a command.\n",
        if group.path.is_empty() {
            String::new()
        } else {
            format!(" {command}")
        }
    ));
    out.write_all(help.as_bytes()).map_or(1, |_| 0)
}

fn add_profile(
    format: OutputFormat,
    name: &str,
    flags: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let request = match parse_add_request(name, flags) {
        Ok(request) => request,
        Err(error) => return write_error("printer add", format, AppError::usage(error), out, err),
    };
    let dir = match config_dir() {
        Ok(dir) => dir,
        Err(error) => {
            return write_error(
                "printer add",
                format,
                AppError {
                    exit_code: 1,
                    code: "internal_error",
                    message: error.to_string(),
                },
                out,
                err,
            );
        }
    };
    match profiles::create(dir, &SystemKeychain, request) {
        Ok(profile) => {
            let name = profile.name.clone();
            write_success(
                "printer add",
                format,
                AddData { profile },
                |out| writeln!(out, "Printer profile added: {name}"),
                out,
            )
        }
        Err(error) => write_error("printer add", format, profile_error(error), out, err),
    }
}

fn parse_add_request(name: &str, flags: &[&String]) -> Result<profiles::CreateRequest, String> {
    let mut request = profiles::CreateRequest {
        name: name.into(),
        driver: String::new(),
        host: String::new(),
        serial: String::new(),
        model: String::new(),
        timeout: "10s".into(),
        insecure: false,
        access_code: String::new(),
    };
    let mut index = 0;
    while index < flags.len() {
        let flag = flags[index].as_str();
        match flag {
            "--insecure" => request.insecure = true,
            "--driver" | "--host" | "--serial" | "--model" | "--timeout" | "--access-code-file" => {
                let value = flags
                    .get(index + 1)
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                match flag {
                    "--driver" => request.driver = (*value).clone(),
                    "--host" => request.host = (*value).clone(),
                    "--serial" => request.serial = (*value).clone(),
                    "--model" => request.model = (*value).clone(),
                    "--timeout" => request.timeout = (*value).clone(),
                    "--access-code-file" => {
                        request.access_code = read_access_code_file(value)?;
                    }
                    _ => unreachable!("recognized flags are exhaustive"),
                }
                index += 1;
            }
            _ => return Err(format!("unknown printer add option {flag:?}")),
        }
        index += 1;
    }
    if request.driver.is_empty() {
        return Err("--driver is required".into());
    }
    if request.host.is_empty() {
        return Err("--host is required".into());
    }
    Ok(request)
}

fn read_access_code_file(path: &str) -> Result<String, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot read --access-code-file: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 4096 {
        return Err("--access-code-file must be a regular file no larger than 4 KiB".into());
    }
    #[cfg(unix)]
    if std::os::unix::fs::PermissionsExt::mode(&metadata.permissions()) & 0o077 != 0 {
        return Err("--access-code-file must not grant group or other access".into());
    }
    let mut code = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read --access-code-file: {error}"))?;
    if let Some(stripped) = code
        .strip_suffix("\r\n")
        .or_else(|| code.strip_suffix('\n'))
    {
        code = stripped.into();
    }
    Ok(code)
}

fn printer_status(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) = match parse_options(
        args,
        &["insecure", "detailed"],
        &["timeout", "protocol-trace"],
    ) {
        Ok(value) => value,
        Err(error) => return write_error("status", format, AppError::usage(error), out, err),
    };
    let name = match one_positional("status", &positionals) {
        Ok(name) => name,
        Err(error) => return write_error("status", format, error, out, err),
    };
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("status", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::Status) {
        Ok(printer) => printer,
        Err(error) => return write_error("status", format, error, out, err),
    };

    match drivers::status(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
    ) {
        Ok(status) => {
            let detailed = options.enabled("detailed");
            let report = human_status(&printer.name, &status, detailed);
            write_success(
                "status",
                format,
                StatusData {
                    profile: printer.name.clone(),
                    driver: printer.driver_kind.name(),
                    status: if detailed {
                        status
                    } else {
                        status.without_extended()
                    },
                    capabilities: printer.driver_kind.capabilities(),
                },
                |out| writeln!(out, "{report}"),
                out,
            )
        }
        Err(error) => write_error("status", format, driver_error(error), out, err),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FirmwareCheckData {
    profile: String,
    driver: &'static str,
    checked_at: String,
    #[serde(flatten)]
    report: FirmwareUpdateReport,
    capabilities: drivers::Capabilities,
}

fn firmware_check(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let command = "firmware check";
    let (positionals, options) = match parse_options(
        args,
        &["insecure", "refresh"],
        &["timeout", "protocol-trace"],
    ) {
        Ok(value) => value,
        Err(error) => return write_error(command, format, AppError::usage(error), out, err),
    };
    let name = match one_positional(command, &positionals) {
        Ok(name) => name,
        Err(error) => return write_error(command, format, error, out, err),
    };
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error(command, format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::FirmwareUpdateCheck) {
        Ok(printer) => printer,
        Err(error) => return write_error(command, format, error, out, err),
    };
    let report = match drivers::firmware_update_status(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        options.enabled("refresh"),
    ) {
        Ok(report) => report,
        Err(error) => return write_error(command, format, driver_error(error), out, err),
    };
    if report.availability == FirmwareUpdateAvailability::Unsupported {
        return write_error(
            command,
            format,
            AppError {
                exit_code: 5,
                code: "capability_unsupported",
                message: report
                    .issues
                    .first()
                    .map_or("firmware update checks are unavailable", |issue| {
                        issue.message
                    })
                    .into(),
            },
            out,
            err,
        );
    }

    let human = human_firmware_updates(&printer.name, &report);
    let checked_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("RFC 3339 formatting is infallible");
    write_success(
        command,
        format,
        FirmwareCheckData {
            profile: printer.name,
            driver: printer.driver_kind.name(),
            checked_at,
            report,
            capabilities: printer.driver_kind.capabilities(),
        },
        |out| writeln!(out, "{human}"),
        out,
    )
}

fn human_firmware_updates(profile: &str, report: &FirmwareUpdateReport) -> String {
    let availability = match report.availability {
        FirmwareUpdateAvailability::Available => "update available",
        FirmwareUpdateAvailability::Current => "current",
        FirmwareUpdateAvailability::Unknown => "unknown",
        FirmwareUpdateAvailability::Unsupported => "unsupported",
    };
    let mut lines = vec![
        format!("Printer: {}", sanitize(profile)),
        format!("Availability: {availability}"),
    ];
    for component in &report.components {
        let kind = match component.kind {
            FirmwareUpdateComponentKind::PrinterFirmware => "Printer firmware",
            FirmwareUpdateComponentKind::AccessoryFirmware => "Accessory firmware",
            FirmwareUpdateComponentKind::PrinterSoftware => "Printer software",
        };
        let current = component.current_version.as_deref().unwrap_or("unknown");
        let version = match component.availability {
            FirmwareUpdateAvailability::Available => {
                component.available_version.as_deref().map_or_else(
                    || sanitize(current),
                    |target| format!("{} -> {}", sanitize(current), sanitize(target)),
                )
            }
            FirmwareUpdateAvailability::Current => format!("{} (current)", sanitize(current)),
            FirmwareUpdateAvailability::Unknown => {
                format!("{} (no update advertised)", sanitize(current))
            }
            FirmwareUpdateAvailability::Unsupported => sanitize(current),
        };
        lines.push(format!(
            "{kind} ({}): {version}",
            sanitize(&component.label)
        ));
    }
    lines.push(format!(
        "Required: {}",
        if report.components.iter().any(|component| component.required) {
            "yes"
        } else {
            "no"
        }
    ));
    for issue in &report.issues {
        lines.push(format!("Warning: {}", sanitize(issue.message)));
    }
    lines.join("\n")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrinterCapabilitiesData {
    profile: String,
    compatibility: diagnostics::BambuCompatibilityReport,
}

fn printer_capabilities(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) =
        match parse_options(args, &["insecure"], &["timeout", "protocol-trace"]) {
            Ok(value) => value,
            Err(error) => {
                return write_error(
                    "printer capabilities",
                    format,
                    AppError::usage(error),
                    out,
                    err,
                );
            }
        };
    let name = match one_positional("printer capabilities", &positionals) {
        Ok(name) => name,
        Err(error) => return write_error("printer capabilities", format, error, out, err),
    };
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("printer capabilities", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::Status) {
        Ok(printer) => printer,
        Err(error) => return write_error("printer capabilities", format, error, out, err),
    };
    let drivers::Profile::Bambu(profile) = &printer.driver else {
        return write_error(
            "printer capabilities",
            format,
            AppError {
                exit_code: 5,
                code: "capability_unsupported",
                message:
                    "live compatibility evidence is currently available only for Bambu LAN profiles"
                        .into(),
            },
            out,
            err,
        );
    };
    let client = bambu::Client::new(profile.clone());
    let capabilities = match client.runtime_capabilities(
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
    ) {
        Ok(capabilities) => capabilities,
        Err(error) => {
            return write_error(
                "printer capabilities",
                format,
                driver_error(DriverError::Bambu(error)),
                out,
                err,
            );
        }
    };
    let selection = bambu::select_camera_transport(profile.host(), &capabilities);
    let firmware_updates = client
        .firmware_update_status(
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            false,
        )
        .ok();
    let compatibility = diagnostics::bambu_compatibility_report_with_updates(
        1,
        &capabilities,
        &selection,
        printer.tls_fingerprint.is_some(),
        None,
        firmware_updates.as_ref(),
    );
    let report = human_capabilities(&printer.name, &compatibility);
    write_success(
        "printer capabilities",
        format,
        PrinterCapabilitiesData {
            profile: printer.name,
            compatibility,
        },
        |out| writeln!(out, "{report}"),
        out,
    )
}

fn human_capabilities(profile: &str, report: &diagnostics::BambuCompatibilityReport) -> String {
    let firmware = report
        .firmware_modules
        .iter()
        .map(|module| format!("{}={}", sanitize(&module.name), sanitize(&module.software)))
        .collect::<Vec<_>>()
        .join(", ");
    let active_quirks = report.quirks.iter().filter(|quirk| quirk.active).count();
    let inactive_quirks = report.quirks.len().saturating_sub(active_quirks);
    let update_summary = report.firmware_updates.as_ref().map_or_else(
        || "unobserved".into(),
        |updates| {
            format!(
                "{:?} ({} components)",
                updates.availability,
                updates.components.len()
            )
        },
    );
    format!(
        "Profile: {}\nModel: {} ({:?}, {:?})\nFirmware: {}\nUpdates: {}\nAuthorization: {:?}{}\nCamera: {:?} ({:?})\nStorage: {:?}\nQuirks: {} active, {} matching but inactive\nObservations: {} (values redacted)",
        sanitize(profile),
        sanitize(&report.model_raw),
        report.canonical_model,
        report.model_family,
        if firmware.is_empty() {
            "unobserved"
        } else {
            &firmware
        },
        update_summary,
        report.authorization.effective,
        if report.authorization.conflict {
            " (conflicting evidence)"
        } else {
            ""
        },
        report.camera.preferred,
        report.camera.source,
        report.storage_transport,
        active_quirks,
        inactive_quirks,
        report.capability_observations.len(),
    )
}

/// Printer-supplied text reaches a terminal here, so control characters are
/// replaced before display to keep escape sequences from being interpreted.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect()
}

fn temperature_line(label: &str, reading: &moonraker::Temperature) -> String {
    match reading.target_celsius {
        Some(target) => format!("{label}: {:.1} C / {target:.1} C", reading.current_celsius),
        None => format!("{label}: {:.1} C", reading.current_celsius),
    }
}

fn fan_display_name(key: &str) -> &str {
    match key {
        // Legacy slot names.
        "partCooling" => "Part cooling",
        "heatbreak" => "Heatbreak",
        "chamber" => "Chamber",
        // Airduct names; `auxiliary` is shared by both generations.
        "parts" => "Part cooling",
        "auxiliary" => "Auxiliary",
        "auxiliarySecondary" => "Auxiliary (secondary)",
        "hotend" => "Hotend",
        "hotendSecondary" => "Hotend (secondary)",
        "exhaust" => "Exhaust",
        "mainboard" => "Mainboard",
        "heat" => "Heat",
        other => other,
    }
}

fn human_status(name: &str, status: &moonraker::Status, detailed: bool) -> String {
    let mut lines = vec![
        format!("Printer: {}", sanitize(name)),
        format!("State: {}", status.state.as_str()),
    ];
    if detailed
        && let Some(stage) = status.stage
        && status.state != moonraker::PrinterState::Idle
    {
        lines.push(format!("Stage: {stage}"));
    }
    if let Some(progress) = &status.progress {
        match (detailed, progress.current_layer, progress.total_layers) {
            (true, Some(current), Some(total)) => lines.push(format!(
                "Progress: {}% (layer {current} / {total})",
                progress.percent
            )),
            _ => lines.push(format!("Progress: {}%", progress.percent)),
        }
    }
    if detailed && let Some(speed) = &status.speed_level {
        lines.push(format!("Speed: {}", sanitize(speed)));
    }
    if detailed && let Some(estimates) = &status.time_estimates {
        lines.push(time_estimate_line(estimates));
    }
    if let Some(temperatures) = &status.temperatures {
        if let Some(nozzle) = &temperatures.nozzle {
            lines.push(temperature_line("Nozzle", nozzle));
        }
        if let Some(bed) = &temperatures.bed {
            lines.push(temperature_line("Bed", bed));
        }
        if let Some(chamber) = &temperatures.chamber {
            lines.push(format!("Chamber: {:.1} C", chamber.current_celsius));
        }
    }
    if detailed && !status.controls.fans.is_empty() {
        lines.push("Fans:".into());
        // Known fans lead in a fixed order; anything else follows sorted. Both
        // firmware generations appear here: legacy slot names and the airduct
        // names H2-class reports. The control inventory is the only map that
        // carries both, and it has already dropped the phantom slots legacy
        // firmware publishes for hardware it does not have.
        let order = [
            "partCooling",
            "heatbreak",
            "auxiliary",
            "chamber",
            "parts",
            "hotend",
            "exhaust",
            "heat",
        ];
        let known = order
            .iter()
            .filter_map(|key| status.controls.fans.get(*key).map(|fan| (*key, fan)));
        let rest = status
            .controls
            .fans
            .iter()
            .filter(|(key, _)| !order.contains(&key.as_str()))
            .map(|(key, fan)| (key.as_str(), fan));
        for (key, fan) in known.chain(rest) {
            if let Some(speed) = fan.speed_percent {
                lines.push(format!("  {}: {speed}%", fan_display_name(key)));
            }
        }
    }
    if detailed && let Some(wifi) = &status.wifi {
        lines.push(format!("Wi-Fi: {} dBm", wifi.signal_dbm));
    }
    if detailed && !status.lights.is_empty() {
        lines.push("Lights:".into());
        for (light, state) in &status.lights {
            lines.push(format!("  {}: {}", sanitize(light), sanitize(state)));
        }
    }
    if let Some(job) = &status.job {
        let mut line = format!("Job: {}", sanitize(&job.name));
        if detailed && let Some(meta) = &status.print_meta {
            let mut parts = Vec::new();
            if let Some(size) = meta.file_size {
                parts.push(format_file_size(size));
            }
            if let Some(diameter) = meta.nozzle_diameter {
                parts.push(format!("{diameter:.1}mm nozzle"));
            }
            if let Some(bed_type) = &meta.bed_type {
                parts.push(sanitize(bed_type));
            }
            if !parts.is_empty() {
                line.push_str(&format!(" ({})", parts.join(", ")));
            }
        }
        lines.push(line);
    }
    if detailed && let Some(position) = &status.gcode_position {
        if position.z_mm > 0.0 {
            lines.push(format!(
                "G-code: Z {:.2} mm, line {} / {}",
                position.z_mm, position.current_line, position.total_lines
            ));
        } else {
            lines.push(format!(
                "G-code: line {} / {}",
                position.current_line, position.total_lines
            ));
        }
    }
    if detailed && let Some(timelapse) = &status.timelapse {
        lines.push(match (timelapse.recording, timelapse.progress) {
            (true, Some(percent)) => format!("Timelapse: recording ({percent}%)"),
            (true, None) => "Timelapse: recording".into(),
            (false, _) => "Timelapse: off".into(),
        });
    }
    if detailed
        && let Some(extension) = &status.extensions.bambu_lan
        && let Some(ams) = &extension.ams
    {
        lines.push("AMS:".into());
        for unit in &ams.units {
            let mut parts = Vec::new();
            if let (Some(range), Some(level)) = (unit.humidity_range, unit.humidity_level) {
                parts.push(format!("humidity: {range} [{level}]"));
            }
            if let Some(temperature) = unit.temperature {
                parts.push(format!("temp: {temperature:.1} C"));
            }
            let suffix = if parts.is_empty() {
                String::new()
            } else {
                format!(" ({})", parts.join(", "))
            };
            lines.push(format!("  Unit {}{suffix}:", unit.id));
            for tray in &unit.trays {
                let mut line = format!("    Slot {}: ", tray.slot);
                match &tray.filament_type {
                    Some(filament) => {
                        line.push_str(&sanitize(filament));
                        if let Some(color) = &tray.color {
                            line.push_str(&format!(" {}", sanitize(color)));
                        }
                        if let Some(remaining) = tray.remaining_percent {
                            line.push_str(&format!(" ({remaining}%)"));
                        }
                    }
                    None => line.push_str("(empty)"),
                }
                lines.push(line);
            }
        }
    }
    if !status.errors.is_empty() {
        lines.push("Errors:".into());
        for error in &status.errors {
            lines.push(format!("- {} {}", error.code, sanitize(&error.message)));
        }
    }
    if !status.warnings.is_empty() {
        lines.push("Warnings:".into());
        for warning in &status.warnings {
            lines.push(format!("- {}", warning.message));
        }
    }
    lines.join("\n")
}

fn time_estimate_line(estimates: &moonraker::TimeEstimates) -> String {
    let mut parts = Vec::new();
    if estimates.elapsed_seconds > 0 {
        parts.push(format!(
            "{} elapsed",
            format_duration(estimates.elapsed_seconds)
        ));
    }
    if let Some(remaining) = estimates.remaining_seconds.filter(|value| *value > 0) {
        parts.push(format!("{} remaining", format_duration(remaining)));
    }
    if parts.is_empty() {
        return "Time: unknown".into();
    }
    format!("Time: {}", parts.join(", "))
}

fn format_duration(seconds: u32) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let (hours, minutes) = (seconds / 3600, (seconds % 3600) / 60);
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn format_file_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    match bytes {
        _ if value >= KB * KB * KB => format!("{:.1} GB", value / (KB * KB * KB)),
        _ if value >= KB * KB => format!("{:.1} MB", value / (KB * KB)),
        _ if value >= KB => format!("{:.1} KB", value / KB),
        _ => format!("{bytes} B"),
    }
}

#[derive(Serialize)]
struct StatusData {
    profile: String,
    driver: &'static str,
    #[serde(flatten)]
    status: melt_core::moonraker::Status,
    capabilities: drivers::Capabilities,
}

fn parse_options(
    args: &[&String],
    boolean_options: &[&str],
    value_options: &[&str],
) -> Result<(Vec<String>, ParsedOptions), String> {
    let mut positionals = Vec::new();
    let mut options = ParsedOptions::default();
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].as_str();
        if !argument.starts_with("--") {
            positionals.push(argument.to_owned());
            index += 1;
            continue;
        }
        let (name, inline_value) = argument
            .strip_prefix("--")
            .expect("options start with two dashes")
            .split_once('=')
            .map_or(
                (argument.trim_start_matches("--"), None),
                |(name, value)| (name, Some(value)),
            );
        if boolean_options.contains(&name) {
            if inline_value.is_some() {
                return Err(format!("--{name} does not take a value"));
            }
            options.flags.insert(name.to_owned());
            index += 1;
            continue;
        }
        if value_options.contains(&name) {
            let value = match inline_value {
                Some(value) if !value.is_empty() => value.to_owned(),
                Some(_) => return Err(format!("--{name} requires a value")),
                None => {
                    index += 1;
                    args.get(index)
                        .map(|value| (*value).clone())
                        .ok_or_else(|| format!("--{name} requires a value"))?
                }
            };
            if options.values.insert(name.to_owned(), value).is_some() {
                return Err(format!("--{name} may only be specified once"));
            }
            index += 1;
            continue;
        }
        return Err(format!("unknown option {argument:?}"));
    }
    Ok((positionals, options))
}

fn one_positional<'a>(command: &str, positionals: &'a [String]) -> Result<&'a str, AppError> {
    match positionals {
        [name] => Ok(name),
        [] => Err(AppError::usage(format!(
            "{command} requires a printer profile name"
        ))),
        _ => Err(AppError::usage(format!(
            "{command} expects exactly one printer profile name"
        ))),
    }
}

/// `printer add` and `printer discover` do not resolve an existing profile,
/// so there is no request/response session to attach a tracer to.
fn protocol_trace_unsupported() -> AppError {
    AppError {
        exit_code: 5,
        code: "capability_unsupported",
        message: "protocol tracing is not available for this command yet".into(),
    }
}

fn connection_options(options: &ParsedOptions) -> Result<ConnectionOptions, AppError> {
    Ok(ConnectionOptions {
        timeout: options.value("timeout").map(str::to_owned),
        insecure: options.enabled("insecure"),
        protocol_trace: options.value("protocol-trace").map(str::to_owned),
    })
}

fn resolve_printer(
    requested_name: &str,
    connection: ConnectionOptions,
    operation: drivers::Operation,
) -> Result<ResolvedPrinter, AppError> {
    mark_network_call();
    let name = requested_name.to_ascii_lowercase();
    let config = Config::load().map_err(config_error)?;
    let mut profile = config
        .get_profile(&name)
        .cloned()
        .ok_or_else(|| AppError::usage(format!("printer profile {requested_name:?} not found")))?;
    if let Some(timeout) = connection.timeout {
        profile.timeout = timeout;
    }
    if connection.insecure {
        profile.insecure = true;
    }
    let timeout = drivers::parse_timeout(&profile.timeout).map_err(driver_error)?;
    let mut driver = drivers::profile(&profile).map_err(driver_error)?;
    if let Some(path) = connection.protocol_trace {
        let tracer = JsonlTracer::create(Path::new(&path)).map_err(|_| AppError {
            exit_code: 1,
            code: "internal_error",
            message: format!("cannot write protocol trace to {path:?}"),
        })?;
        driver = drivers::attach_tracer(driver, std::sync::Arc::new(tracer));
    }
    let driver_kind = driver.driver();
    if !driver.capabilities().supports(operation) {
        return Err(driver_error(DriverError::UnsupportedOperation(
            driver_kind,
            operation,
        )));
    }

    let access_code =
        match SystemKeychain.get(SERVICE, &account(&profile.driver, &name, "access-code")) {
            Ok(access_code) => Some(access_code),
            Err(SecretError::NotFound) if driver_kind.requires_access_code() => {
                return Err(AppError {
                    exit_code: 3,
                    code: "secret_not_found",
                    message: "required printer access code is unavailable".into(),
                });
            }
            Err(SecretError::NotFound) => None,
            // Moonraker supports a keyless trusted-LAN configuration, so an
            // unavailable keychain cannot block it when no credential is needed.
            Err(SecretError::Unavailable(_)) if driver_kind == drivers::Driver::Moonraker => None,
            Err(SecretError::Unavailable(_)) => {
                return Err(AppError {
                    exit_code: 3,
                    code: "secret_store_failed",
                    message: "keychain operation failed".into(),
                });
            }
        };
    let tls_fingerprint = if driver_kind == drivers::Driver::BambuLan && !profile.insecure {
        match SystemKeychain.get(SERVICE, &account(&profile.driver, &name, "tls-fingerprint")) {
            Ok(fingerprint) => Some(fingerprint),
            Err(SecretError::NotFound) => {
                return Err(AppError {
                    exit_code: 3,
                    code: "secret_not_found",
                    message: "required printer TLS fingerprint is unavailable".into(),
                });
            }
            Err(SecretError::Unavailable(_)) => {
                return Err(AppError {
                    exit_code: 3,
                    code: "secret_store_failed",
                    message: "keychain operation failed".into(),
                });
            }
        }
    } else {
        None
    };

    Ok(ResolvedPrinter {
        name,
        driver,
        driver_kind,
        access_code,
        tls_fingerprint,
        timeout,
    })
}

fn require_confirmation(
    command: &str,
    yes: bool,
    prompt: &str,
    format: OutputFormat,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(), i32> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(write_error(
            command,
            format,
            AppError::usage("non-interactive mode requires --yes"),
            out,
            err,
        ));
    }
    if write!(err, "{prompt}").is_err() {
        return Err(1);
    }
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return Err(write_error(
            command,
            format,
            AppError {
                exit_code: 1,
                code: "internal_error",
                message: "cannot read confirmation".into(),
            },
            out,
            err,
        ));
    }
    if answer.trim_end() != "yes" {
        return Err(write_error(
            command,
            format,
            AppError::usage("confirmation declined; no printer action was sent"),
            out,
            err,
        ));
    }
    Ok(())
}

fn require_state(
    command: &str,
    printer: &ResolvedPrinter,
    allowed: &[melt_core::moonraker::PrinterState],
) -> Result<(), AppError> {
    let status = drivers::status(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
    )
    .map_err(driver_error)?;
    if allowed.contains(&status.state) {
        return Ok(());
    }
    Err(AppError {
        exit_code: 2,
        code: "invalid_printer_state",
        message: format!("printer state {:?} does not allow {command}", status.state),
    })
}

fn camera_snapshot(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) = match parse_options(
        args,
        &["insecure", "overwrite"],
        &["to", "timeout", "protocol-trace"],
    ) {
        Ok(value) => value,
        Err(error) => {
            return write_error("camera snapshot", format, AppError::usage(error), out, err);
        }
    };
    let name = match one_positional("camera snapshot", &positionals) {
        Ok(name) => name,
        Err(error) => return write_error("camera snapshot", format, error, out, err),
    };
    let destination = match snapshot_destination(options.value("to"), &name.to_ascii_lowercase()) {
        Ok(path) => path,
        Err(error) => return write_error("camera snapshot", format, error, out, err),
    };
    if let Err(error) = validate_snapshot_destination(&destination, options.enabled("overwrite")) {
        return write_error("camera snapshot", format, error, out, err);
    }
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("camera snapshot", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::CameraSnapshot) {
        Ok(printer) => printer,
        Err(error) => return write_error("camera snapshot", format, error, out, err),
    };
    let image = match drivers::camera_snapshot(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        printer.timeout,
    ) {
        Ok(image) => image,
        Err(error) => return write_error("camera snapshot", format, driver_error(error), out, err),
    };
    let bytes = match write_snapshot_file(&destination, &image, options.enabled("overwrite")) {
        Ok(bytes) => bytes,
        Err(error) => return write_error("camera snapshot", format, error, out, err),
    };
    let path = destination.display().to_string();
    write_success(
        "camera snapshot",
        format,
        CameraSnapshotData {
            profile: printer.name,
            driver: printer.driver_kind.name(),
            path: path.clone(),
            size_bytes: bytes,
            protocol: "mjpeg",
        },
        |out| writeln!(out, "Snapshot saved to {path} ({bytes} bytes)."),
        out,
    )
}

fn snapshot_destination(requested: Option<&str>, profile: &str) -> Result<PathBuf, AppError> {
    let generated = format!(
        "{}-{}.jpg",
        profile,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AppError::usage("system clock is before the Unix epoch"))?
            .as_secs()
    );
    let destination = requested
        .map(PathBuf::from)
        .unwrap_or_else(|| generated.clone().into());
    match fs::metadata(&destination) {
        Ok(metadata) if metadata.is_dir() => Ok(destination.join(generated)),
        Ok(_) => Ok(destination),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(destination),
        Err(_) => Err(AppError::usage(format!(
            "cannot inspect destination path: {}",
            destination.display()
        ))),
    }
}

fn validate_snapshot_destination(destination: &Path, overwrite: bool) -> Result<(), AppError> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    match fs::metadata(parent) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(AppError::usage(format!(
                "destination directory is not a directory: {}",
                parent.display()
            )));
        }
        Err(_) => {
            return Err(AppError::usage(format!(
                "destination directory does not exist: {}",
                parent.display()
            )));
        }
    }
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(AppError::usage(format!(
            "destination path is unsafe: {}",
            destination.display()
        ))),
        Ok(metadata) if metadata.is_dir() => Err(AppError::usage(format!(
            "destination path is a directory: {}",
            destination.display()
        ))),
        Ok(_) if !overwrite => Err(AppError::usage(format!(
            "destination file already exists: {} (use --overwrite to replace)",
            destination.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(AppError::usage(format!(
            "cannot inspect destination path: {}",
            destination.display()
        ))),
    }
}

fn write_snapshot_file(destination: &Path, image: &[u8], overwrite: bool) -> Result<u64, AppError> {
    if image.is_empty() {
        return Err(AppError {
            exit_code: 1,
            code: "internal_error",
            message: "camera snapshot returned empty image data".into(),
        });
    }
    validate_snapshot_destination(destination, overwrite)?;
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|_| AppError {
        exit_code: 1,
        code: "internal_error",
        message: "cannot create snapshot file".into(),
    })?;
    temporary.write_all(image).map_err(|_| AppError {
        exit_code: 1,
        code: "internal_error",
        message: "cannot write snapshot file".into(),
    })?;
    temporary.as_file_mut().sync_all().map_err(|_| AppError {
        exit_code: 1,
        code: "internal_error",
        message: "cannot finalize snapshot file".into(),
    })?;
    let committed = if overwrite {
        temporary.persist(destination)
    } else {
        temporary.persist_noclobber(destination)
    };
    committed.map_err(|_| AppError {
        exit_code: 1,
        code: "internal_error",
        message: "cannot move snapshot file into place".into(),
    })?;
    Ok(image.len() as u64)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CameraSnapshotData {
    profile: String,
    driver: &'static str,
    path: String,
    size_bytes: u64,
    protocol: &'static str,
}

fn camera_stream(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) = match parse_options(
        args,
        &["insecure"],
        &["port", "duration", "format", "timeout", "protocol-trace"],
    ) {
        Ok(value) => value,
        Err(error) => {
            return write_error("camera stream", format, AppError::usage(error), out, err);
        }
    };
    let name = match one_positional("camera stream", &positionals) {
        Ok(name) => name,
        Err(error) => return write_error("camera stream", format, error, out, err),
    };
    let port = match options.value("port").unwrap_or("8080").parse::<u16>() {
        Ok(port) if port != 0 => port,
        _ => {
            return write_error(
                "camera stream",
                format,
                AppError::usage("--port must be between 1 and 65535"),
                out,
                err,
            );
        }
    };
    let duration = match options.value("duration") {
        Some(value) => match parse_stream_duration(value) {
            Some(duration) => Some(duration),
            None => {
                return write_error(
                    "camera stream",
                    format,
                    AppError::usage("--duration must be a positive duration such as 1m30s or 1.5s"),
                    out,
                    err,
                );
            }
        },
        None => None,
    };
    if let Some(value) = options.value("format").filter(|value| *value != "mjpeg") {
        return write_error(
            "camera stream",
            format,
            AppError::usage(format!(
                "invalid --format {value:?}: must be \"mjpeg\" or omitted"
            )),
            out,
            err,
        );
    }
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("camera stream", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::CameraStream) {
        Ok(printer) => printer,
        Err(error) => return write_error("camera stream", format, error, out, err),
    };
    let stream = match drivers::camera_stream(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        printer.timeout,
    ) {
        Ok(stream) => stream,
        Err(error) => return write_error("camera stream", format, driver_error(error), out, err),
    };
    let upstream_shutdown = if duration.is_some() {
        match stream.shutdown_handle() {
            Ok(socket) => Some(socket),
            Err(_) => {
                return write_error(
                    "camera stream",
                    format,
                    AppError {
                        exit_code: 1,
                        code: "internal_error",
                        message: "cannot prepare camera stream shutdown".into(),
                    },
                    out,
                    err,
                );
            }
        }
    } else {
        None
    };
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => listener,
        Err(_) => {
            return write_error(
                "camera stream",
                format,
                AppError::usage(format!("port {port} is already in use or unavailable")),
                out,
                err,
            );
        }
    };
    if listener.set_nonblocking(true).is_err() {
        return write_error(
            "camera stream",
            format,
            AppError {
                exit_code: 1,
                code: "internal_error",
                message: "cannot configure local camera server".into(),
            },
            out,
            err,
        );
    }
    let url = format!("http://127.0.0.1:{port}/stream");
    let profile = printer.name.clone();
    let result = write_success(
        "camera stream",
        format,
        CameraStreamData {
            profile: profile.clone(),
            url: url.clone(),
            format: "mjpeg",
            port,
        },
        |out| {
            writeln!(out, "Streaming camera from {profile}")?;
            writeln!(out, "Format: MJPEG (open in browser)")?;
            writeln!(out, "URL: {url}\n")?;
            writeln!(out, "Press Ctrl+C to stop.")
        },
        out,
    );
    if result != 0 {
        return result;
    }
    serve_camera_stream(listener, stream, duration, upstream_shutdown);
    if matches!(format, OutputFormat::Human) {
        writeln!(out, "Stream stopped.").map_or(1, |_| 0)
    } else {
        0
    }
}

#[derive(Serialize)]
struct CameraStreamData {
    profile: String,
    url: String,
    format: &'static str,
    port: u16,
}

fn serve_camera_stream(
    listener: TcpListener,
    stream: melt_core::bambu::MjpegStream,
    duration: Option<Duration>,
    upstream_shutdown: Option<TcpStream>,
) {
    let deadline = duration.map(|duration| Instant::now() + duration);
    let active = AtomicBool::new(false);
    let (done_sender, done_receiver) = mpsc::channel();
    let mut stream = Some(stream);
    let mut worker: Option<thread::JoinHandle<()>> = None;
    let mut active_socket = None;

    loop {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        if let Ok(returned_stream) = done_receiver.try_recv() {
            if let Some(worker) = worker.take() {
                let _ = worker.join();
            }
            stream = Some(returned_stream);
            active.store(false, Ordering::Release);
            active_socket = None;
        }
        match listener.accept() {
            Ok((mut socket, _)) => {
                if !camera_stream_request(&mut socket) {
                    let _ = write_http_response(&mut socket, "403 Forbidden", &[]);
                } else if !claim_stream_client(&active) {
                    let _ = write_http_response(
                        &mut socket,
                        "503 Service Unavailable",
                        &[("Retry-After", "1")],
                    );
                } else if let Some(reader) = stream.take() {
                    active_socket = socket.try_clone().ok();
                    let sender = done_sender.clone();
                    worker = Some(thread::spawn(move || {
                        let _ = write_http_response(
                            &mut socket,
                            "200 OK",
                            &[
                                ("Content-Type", "multipart/x-mixed-replace; boundary=frame"),
                                ("Cache-Control", "no-cache"),
                                ("Connection", "close"),
                            ],
                        );
                        let mut reader = reader;
                        let _ = std::io::copy(&mut reader, &mut socket);
                        let _ = sender.send(reader);
                    }));
                } else {
                    active.store(false, Ordering::Release);
                    let _ = write_http_response(&mut socket, "503 Service Unavailable", &[]);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => break,
        }
    }
    drop(listener);
    if let Some(socket) = active_socket {
        let _ = socket.shutdown(Shutdown::Both);
    }
    if let Some(socket) = upstream_shutdown {
        let _ = socket.shutdown(Shutdown::Both);
    }
    if let Some(worker) = worker {
        let _ = worker.join();
    }
}

fn camera_stream_request(socket: &mut TcpStream) -> bool {
    let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
    let request = read_camera_stream_header(socket);
    let _ = socket.set_read_timeout(None);
    let Some(request) = request else {
        return false;
    };
    let Ok(request) = std::str::from_utf8(&request) else {
        return false;
    };
    is_camera_stream_request(request)
}

const MAX_CAMERA_STREAM_HEADER: usize = 8192;

fn read_camera_stream_header(reader: &mut dyn Read) -> Option<Vec<u8>> {
    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0; 1024];
    loop {
        let remaining = MAX_CAMERA_STREAM_HEADER.checked_sub(request.len())?;
        if remaining == 0 {
            return None;
        }
        let read_size = remaining.min(buffer.len());
        let read = reader.read(&mut buffer[..read_size]).ok()?;
        if read == 0 {
            return None;
        }
        request.extend_from_slice(&buffer[..read]);
        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            request.truncate(end + 4);
            return Some(request);
        }
    }
}

fn is_camera_stream_request(request: &str) -> bool {
    let mut lines = request.split("\r\n");
    let Some(request_line) = lines.next() else {
        return false;
    };
    let mut request_line = request_line.split_ascii_whitespace();
    if !matches!(
        (
            request_line.next(),
            request_line.next(),
            request_line.next(),
            request_line.next()
        ),
        (Some("GET"), Some("/stream"), Some(_), None)
    ) {
        return false;
    }
    lines
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("host").then_some(value.trim())
        })
        .is_some_and(is_loopback_host)
}

fn is_loopback_host(host: &str) -> bool {
    let (host, port) = if let Some(host) = host.strip_prefix('[') {
        let Some((host, rest)) = host.split_once(']') else {
            return false;
        };
        if rest.is_empty() {
            (host, None)
        } else if let Some(port) = rest.strip_prefix(':') {
            (host, Some(port))
        } else {
            return false;
        }
    } else {
        match host.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host, None),
        }
    };
    if port.is_some_and(|port| port.parse::<u16>().is_err()) {
        return false;
    }
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn parse_stream_duration(value: &str) -> Option<Duration> {
    let mut value = value.strip_prefix('+').unwrap_or(value);
    if value.starts_with('-') {
        return None;
    }
    let mut duration = Duration::ZERO;

    while !value.is_empty() {
        let number_end = value
            .char_indices()
            .take_while(|(_, character)| character.is_ascii_digit() || *character == '.')
            .map(|(index, character)| index + character.len_utf8())
            .last()?;
        let number = value[..number_end].parse::<f64>().ok()?;
        if !number.is_finite() || number < 0.0 {
            return None;
        }
        value = &value[number_end..];

        let (unit, seconds) = [
            ("ns", 1e-9),
            ("us", 1e-6),
            ("µs", 1e-6),
            ("ms", 1e-3),
            ("s", 1.0),
            ("m", 60.0),
            ("h", 60.0 * 60.0),
        ]
        .into_iter()
        .find_map(|(unit, seconds)| value.strip_prefix(unit).map(|rest| (rest, seconds)))?;
        value = unit;
        duration = duration.checked_add(Duration::try_from_secs_f64(number * seconds).ok()?)?;
    }

    (!duration.is_zero()).then_some(duration)
}

fn claim_stream_client(active: &AtomicBool) -> bool {
    active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

fn write_http_response(
    socket: &mut TcpStream,
    status: &str,
    headers: &[(&str, &str)],
) -> std::io::Result<()> {
    write!(socket, "HTTP/1.1 {status}\r\n")?;
    for (name, value) in headers {
        write!(socket, "{name}: {value}\r\n")?;
    }
    if status == "200 OK" {
        socket.write_all(b"\r\n")
    } else {
        socket.write_all(b"Content-Length: 0\r\n\r\n")
    }
}

fn lights_set(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) =
        match parse_options(args, &["yes", "insecure"], &["timeout", "protocol-trace"]) {
            Ok(value) => value,
            Err(error) => {
                return write_error("lights set", format, AppError::usage(error), out, err);
            }
        };
    let (name, light, state_name) = match positionals.as_slice() {
        [name, light, state] => (name.as_str(), light.as_str(), state.as_str()),
        _ => {
            return write_error(
                "lights set",
                format,
                AppError::usage("lights set requires a printer profile, light name, and state"),
                out,
                err,
            );
        }
    };
    let light = match normalize_light(light) {
        Ok(light) => light,
        Err(error) => return write_error("lights set", format, error, out, err),
    };
    let state = match melt_core::moonraker::LightState::parse(state_name) {
        Some(state) => state,
        None => {
            return write_error(
                "lights set",
                format,
                AppError::usage("light state must be exactly \"on\" or \"off\""),
                out,
                err,
            );
        }
    };
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("lights set", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::LightSet) {
        Ok(printer) => printer,
        Err(error) => return write_error("lights set", format, error, out, err),
    };
    if let Err(error) = require_state(
        "lights set",
        &printer,
        &[
            melt_core::moonraker::PrinterState::Idle,
            melt_core::moonraker::PrinterState::Printing,
            melt_core::moonraker::PrinterState::Paused,
            melt_core::moonraker::PrinterState::Error,
        ],
    ) {
        return write_error("lights set", format, error, out, err);
    }
    if let Err(code) = require_confirmation(
        "lights set",
        options.enabled("yes"),
        &format!(
            "Set {light} light to {state_name} on {}? Type 'yes' to continue: ",
            printer.name
        ),
        format,
        out,
        err,
    ) {
        return code;
    }
    match drivers::light_set(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        &light,
        state,
    ) {
        Ok(result) => {
            let display = match result.light.as_str() {
                "chamber_light" => "Chamber".into(),
                "aux_light" => "Aux".into(),
                _ => result.light.clone(),
            };
            let state = match result.state {
                melt_core::moonraker::LightState::On => "on",
                melt_core::moonraker::LightState::Off => "off",
            };
            write_success(
                "lights set",
                format,
                LightSetData {
                    profile: printer.name,
                    driver: printer.driver_kind.name(),
                    light: result.light,
                    state: result.state,
                    warnings: Vec::new(),
                    capabilities: printer.driver_kind.capabilities(),
                },
                |out| writeln!(out, "{display} light set to {state}."),
                out,
            )
        }
        Err(error) => write_error("lights set", format, driver_error(error), out, err),
    }
}

fn normalize_light(light: &str) -> Result<String, AppError> {
    if light.is_empty()
        || !light
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(AppError::usage("invalid light name syntax"));
    }
    match light.to_ascii_lowercase().as_str() {
        "chamber" | "chamber-light" | "chamber_light" => Ok("chamber_light".into()),
        "aux" | "aux-light" | "aux_light" => Ok("aux_light".into()),
        _ => Ok(light.into()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LightSetData {
    profile: String,
    driver: &'static str,
    light: String,
    state: melt_core::moonraker::LightState,
    warnings: Vec<String>,
    capabilities: drivers::Capabilities,
}

fn discover_printers(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) =
        match parse_options(args, &[], &["driver", "timeout", "protocol-trace"]) {
            Ok(value) => value,
            Err(error) => {
                return write_error("printer discover", format, AppError::usage(error), out, err);
            }
        };
    if !positionals.is_empty() {
        return write_error(
            "printer discover",
            format,
            AppError::usage("printer discover does not accept positional arguments"),
            out,
            err,
        );
    }
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("printer discover", format, error, out, err),
    };
    if connection.protocol_trace.is_some() {
        return write_error(
            "printer discover",
            format,
            protocol_trace_unsupported(),
            out,
            err,
        );
    }
    let driver = match options.value("driver") {
        Some(name) => match drivers::Driver::parse(name) {
            Ok(driver) => driver,
            Err(error) => {
                return write_error("printer discover", format, driver_error(error), out, err);
            }
        },
        None => drivers::Driver::BambuLan,
    };
    if !driver.supports(drivers::Operation::Discovery) {
        return write_error(
            "printer discover",
            format,
            driver_error(DriverError::UnsupportedOperation(
                driver,
                drivers::Operation::Discovery,
            )),
            out,
            err,
        );
    }
    let timeout = match drivers::parse_timeout(connection.timeout.as_deref().unwrap_or("5s")) {
        Ok(timeout) => timeout,
        Err(_) => {
            return write_error(
                "printer discover",
                format,
                AppError::usage("invalid --timeout: expected a positive duration such as 5s"),
                out,
                err,
            );
        }
    };
    mark_network_call();
    let configured = match Config::load() {
        Ok(config) => config
            .sorted_profiles()
            .into_iter()
            .filter(|profile| !profile.profile.serial.is_empty())
            .map(|profile| (profile.profile.serial, profile.name))
            .collect::<BTreeMap<_, _>>(),
        Err(error) => {
            return write_error("printer discover", format, config_error(error), out, err);
        }
    };
    match melt_core::bambu::discover(timeout) {
        Ok(printers) => {
            let printers = printers
                .into_iter()
                .map(|printer| DiscoveryPrinter {
                    configured_as: configured.get(&printer.serial).cloned(),
                    driver: printer.driver,
                    host: printer.host,
                    serial: printer.serial,
                    model: printer.model,
                    name: printer.name,
                })
                .collect::<Vec<_>>();
            let count = printers.len();
            write_success(
                "printer discover",
                format,
                DiscoveryData {
                    printers: printers.clone(),
                },
                |out| {
                    if count == 0 {
                        writeln!(out, "No printers found on the local network.")
                    } else {
                        writeln!(out, "NAME\tSERIAL\tMODEL\tHOST\tCONFIGURED")?;
                        for printer in &printers {
                            writeln!(
                                out,
                                "{}\t{}\t{}\t{}\t{}",
                                printer.name,
                                printer.serial,
                                printer.model,
                                printer.host,
                                printer.configured_as.as_deref().unwrap_or("-")
                            )?;
                        }
                        Ok(())
                    }
                },
                out,
            )
        }
        Err(_) => write_error(
            "printer discover",
            format,
            AppError {
                exit_code: 4,
                code: "connection_failed",
                message: "printer discovery failed".into(),
            },
            out,
            err,
        ),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryData {
    printers: Vec<DiscoveryPrinter>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryPrinter {
    driver: &'static str,
    host: String,
    serial: String,
    model: String,
    name: String,
    configured_as: Option<String>,
}

fn refresh_tls(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) =
        match parse_options(args, &["yes", "insecure"], &["timeout", "protocol-trace"]) {
            Ok(value) => value,
            Err(error) => {
                return write_error(
                    "printer tls refresh",
                    format,
                    AppError::usage(error),
                    out,
                    err,
                );
            }
        };
    let name = match one_positional("printer tls refresh", &positionals) {
        Ok(name) => name,
        Err(error) => return write_error("printer tls refresh", format, error, out, err),
    };
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("printer tls refresh", format, error, out, err),
    };
    let insecure = options.enabled("insecure");
    if !options.enabled("yes") && !std::io::stdin().is_terminal() {
        return write_error(
            "printer tls refresh",
            format,
            AppError::usage("non-interactive mode requires --yes"),
            out,
            err,
        );
    }
    let dir = match config_dir() {
        Ok(dir) => dir,
        Err(error) => {
            return write_error(
                "printer tls refresh",
                format,
                AppError {
                    exit_code: 1,
                    code: "internal_error",
                    message: error.to_string(),
                },
                out,
                err,
            );
        }
    };
    let result = if insecure {
        let prompt =
            format!("Permanently disable TLS verification for {name}? Type 'yes' to continue: ");
        if let Err(exit) = require_confirmation(
            "printer tls refresh",
            options.enabled("yes"),
            &prompt,
            format,
            out,
            err,
        ) {
            return exit;
        }
        profiles::refresh_tls(dir, &SystemKeychain, name, true, connection.timeout)
    } else {
        let fingerprint = match profiles::preview_tls(&dir, name, connection.timeout) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                return write_error(
                    "printer tls refresh",
                    format,
                    profile_error(error),
                    out,
                    err,
                );
            }
        };
        let prompt = format!(
            "Replace the TLS fingerprint for {name} with {fingerprint}? Type 'yes' to continue: "
        );
        if let Err(exit) = require_confirmation(
            "printer tls refresh",
            options.enabled("yes"),
            &prompt,
            format,
            out,
            err,
        ) {
            return exit;
        }
        profiles::store_tls_fingerprint(dir, &SystemKeychain, name, &fingerprint)
    };
    match result {
        Ok(result) => {
            let fingerprint = result.fingerprint.clone();
            write_success(
                "printer tls refresh",
                format,
                result,
                |out| match fingerprint {
                    Some(fingerprint) => writeln!(
                        out,
                        "TLS certificate re-pinned: {name}\nFingerprint: {fingerprint}"
                    ),
                    None => writeln!(
                        out,
                        "TLS certificate verification disabled: {name}\nWarning: TLS verification is disabled for this profile."
                    ),
                },
                out,
            )
        }
        Err(error) => write_error(
            "printer tls refresh",
            format,
            profile_error(error),
            out,
            err,
        ),
    }
}

fn files_roots(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) =
        match parse_options(args, &["insecure"], &["timeout", "protocol-trace"]) {
            Ok(value) => value,
            Err(error) => {
                return write_error("files roots", format, AppError::usage(error), out, err);
            }
        };
    let name = match one_positional("files roots", &positionals) {
        Ok(name) => name,
        Err(error) => return write_error("files roots", format, error, out, err),
    };
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("files roots", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::FileList) {
        Ok(printer) => printer,
        Err(error) => return write_error("files roots", format, error, out, err),
    };
    match drivers::file_roots(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
    ) {
        Ok(roots) => {
            let human_roots = roots.clone();
            let profile = printer.name.clone();
            write_success(
                "files roots",
                format,
                FileRootsData {
                    profile: printer.name,
                    driver: printer.driver_kind.name(),
                    roots,
                    warnings: Vec::new(),
                    capabilities: file_capabilities(printer.driver_kind.capabilities()),
                },
                |out| {
                    writeln!(out, "Printer: {profile}\n")?;
                    writeln!(
                        out,
                        "{:<8} {:<9} {:<10} {:<10} DESCRIPTION",
                        "ROOT", "WRITABLE", "FREE", "CAPACITY"
                    )?;
                    for root in &human_roots {
                        writeln!(
                            out,
                            "{:<8} {:<9} {:<10} {:<10} {}",
                            root.name,
                            root.writable,
                            optional_size(root.free_bytes.map(|v| v as i64)),
                            optional_size(root.capacity_bytes.map(|v| v as i64)),
                            root.description
                        )?;
                    }
                    Ok(())
                },
                out,
            )
        }
        Err(error) => write_error("files roots", format, driver_error(error), out, err),
    }
}

fn files_list(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) = match parse_options(
        args,
        &["insecure", "recursive"],
        &["timeout", "protocol-trace"],
    ) {
        Ok(value) => value,
        Err(error) => return write_error("files list", format, AppError::usage(error), out, err),
    };
    let (name, requested_paths) = match positionals.split_first() {
        Some(value) => value,
        None => {
            return write_error(
                "files list",
                format,
                AppError::usage("files list requires a printer profile name"),
                out,
                err,
            );
        }
    };
    for path in requested_paths {
        if let Err(error) = validate_known_file_root(path) {
            return write_error("files list", format, error, out, err);
        }
    }
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("files list", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::FileList) {
        Ok(printer) => printer,
        Err(error) => return write_error("files list", format, error, out, err),
    };
    let root = file_root(printer.driver_kind);
    let paths = if requested_paths.is_empty() {
        vec![("/".to_owned(), format!("{root}:/"))]
    } else {
        let mut paths = Vec::new();
        for path in requested_paths {
            match parse_device_path(path, root) {
                Ok(path) => paths.push(path),
                Err(error) => return write_error("files list", format, error, out, err),
            }
        }
        paths
    };
    let mut results = Vec::new();
    for (path, device_path) in paths {
        match drivers::file_list(
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            &path,
            options.enabled("recursive"),
        ) {
            Ok(listing) => results.push(FilePathData {
                device_path,
                entries: listing.entries,
            }),
            Err(error) => return write_error("files list", format, driver_error(error), out, err),
        }
    }
    let profile = printer.name.clone();
    let human_paths = results.clone();
    write_success(
        "files list",
        format,
        FileListData {
            profile: printer.name,
            driver: printer.driver_kind.name(),
            paths: results,
            warnings: Vec::new(),
            capabilities: file_capabilities(printer.driver_kind.capabilities()),
        },
        |out| {
            writeln!(out, "Printer: {profile}")?;
            for (index, path) in human_paths.iter().enumerate() {
                if index > 0 {
                    writeln!(out)?;
                }
                writeln!(out, "Path: {}\n", sanitize(&path.device_path))?;
                if path.entries.is_empty() {
                    writeln!(out, "(empty)")?;
                    continue;
                }
                writeln!(out, "{:<10} {:<9} {:<21} NAME", "TYPE", "SIZE", "MODIFIED")?;
                for entry in &path.entries {
                    writeln!(
                        out,
                        "{:<10} {:<9} {:<21} {}",
                        entry.entry_type.as_str(),
                        optional_size(entry.size_bytes),
                        entry
                            .modified_at
                            .as_deref()
                            .map_or_else(|| "-".to_owned(), format_modified_time),
                        sanitize(&entry.name)
                    )?;
                }
            }
            Ok(())
        },
        out,
    )
}

/// Binary-unit sizes, matching the reference `files` tables. Distinct from
/// [`format_file_size`], which renders decimal units for print metadata.
fn optional_size(bytes: Option<i64>) -> String {
    let Some(bytes) = bytes else {
        return "-".to_owned();
    };
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let value = bytes as f64;
    match value {
        v if v >= GIB => format!("{:.1} GiB", v / GIB),
        v if v >= MIB => format!("{:.1} MiB", v / MIB),
        v if v >= KIB => format!("{:.1} KiB", v / KIB),
        _ => format!("{bytes} B"),
    }
}

/// ponytail: both drivers emit UTC RFC 3339 (`2026-06-19T00:44:00Z`), so the
/// display form is a slice rather than a parse. Anything else passes through
/// unchanged, matching the reference fallback.
fn format_modified_time(rfc3339: &str) -> String {
    let bytes = rfc3339.as_bytes();
    if bytes.len() == 20 && bytes[10] == b'T' && bytes[19] == b'Z' {
        format!("{} {} UTC", &rfc3339[..10], &rfc3339[11..16])
    } else {
        rfc3339.to_owned()
    }
}

fn file_root(driver: drivers::Driver) -> &'static str {
    match driver {
        drivers::Driver::BambuLan => "sdcard",
        drivers::Driver::Moonraker => "gcodes",
    }
}

fn parse_device_path(value: &str, expected_root: &str) -> Result<(String, String), AppError> {
    let (root, path) = value.split_once(':').ok_or_else(|| {
        AppError::usage(format!(
            "device path must use the {expected_root}:/path form"
        ))
    })?;
    if root != expected_root {
        return Err(AppError::usage(format!("unsupported file root {root:?}")));
    }
    let path = if path.is_empty() { "/" } else { path };
    Ok((path.to_owned(), format!("{expected_root}:{path}")))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileCapabilities {
    file_list: bool,
    file_download: bool,
    file_upload: bool,
    file_delete: bool,
}

fn file_capabilities(capabilities: drivers::Capabilities) -> FileCapabilities {
    FileCapabilities {
        file_list: capabilities.file_list,
        file_download: capabilities.file_download,
        file_upload: capabilities.file_upload,
        file_delete: capabilities.file_delete,
    }
}

#[derive(Serialize)]
struct FileRootsData {
    profile: String,
    driver: &'static str,
    roots: Vec<melt_core::moonraker::FileRoot>,
    warnings: Vec<String>,
    capabilities: FileCapabilities,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FilePathData {
    device_path: String,
    entries: Vec<melt_core::moonraker::FileEntry>,
}

#[derive(Serialize)]
struct FileListData {
    profile: String,
    driver: &'static str,
    paths: Vec<FilePathData>,
    warnings: Vec<String>,
    capabilities: FileCapabilities,
}

fn files_upload(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) = match parse_options(
        args,
        &["insecure", "overwrite"],
        &["timeout", "protocol-trace"],
    ) {
        Ok(value) => value,
        Err(error) => {
            return write_error("files upload", format, AppError::usage(error), out, err);
        }
    };
    let (name, source, requested_destination) = match positionals.as_slice() {
        [name, source] => (name.as_str(), PathBuf::from(source), None),
        [name, source, destination] => (
            name.as_str(),
            PathBuf::from(source),
            Some(destination.as_str()),
        ),
        _ => {
            return write_error(
                "files upload",
                format,
                AppError::usage(
                    "files upload requires a printer profile, local path, and optional device path",
                ),
                out,
                err,
            );
        }
    };
    if let Some(destination) = requested_destination {
        if let Err(error) = validate_known_file_root(destination) {
            return write_error("files upload", format, error, out, err);
        }
    }
    let source_metadata = match fs::metadata(&source) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            return write_error(
                "files upload",
                format,
                AppError::usage("local source must be a regular file"),
                out,
                err,
            );
        }
        Err(_) => {
            return write_error(
                "files upload",
                format,
                AppError::usage("local source does not exist"),
                out,
                err,
            );
        }
    };
    let source_name = match source.file_name().and_then(|name| name.to_str()) {
        Some(name) if !name.is_empty() => name,
        _ => {
            return write_error(
                "files upload",
                format,
                AppError::usage("local source must have a valid file name"),
                out,
                err,
            );
        }
    };
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("files upload", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::FileUpload) {
        Ok(printer) => printer,
        Err(error) => return write_error("files upload", format, error, out, err),
    };
    let (destination, destination_display) = match upload_destination(
        requested_destination,
        source_name,
        file_root(printer.driver_kind),
    ) {
        Ok(destination) => destination,
        Err(error) => return write_error("files upload", format, error, out, err),
    };
    match drivers::upload_file(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        &source,
        &destination,
        options.enabled("overwrite"),
    ) {
        Ok(transferred) => write_success(
            "files upload",
            format,
            FileTransferData {
                profile: printer.name,
                driver: printer.driver_kind.name(),
                source: source.display().to_string(),
                destination: destination_display,
                bytes_transferred: transferred,
                warnings: Vec::new(),
                capabilities: file_capabilities(printer.driver_kind.capabilities()),
            },
            |out| writeln!(out, "Uploaded {} bytes.", source_metadata.len()),
            out,
        ),
        Err(error) => write_error("files upload", format, driver_error(error), out, err),
    }
}

fn upload_destination(
    requested: Option<&str>,
    source_name: &str,
    root: &str,
) -> Result<(String, String), AppError> {
    let default = format!("{root}:/");
    let raw = requested.unwrap_or(&default);
    let (mut path, _) = parse_device_path(raw, root)?;
    if raw.ends_with('/') || path == "/" {
        path = format!("{}/{}", path.trim_end_matches('/'), source_name);
        if !path.starts_with('/') {
            path.insert(0, '/');
        }
    }
    if path == "/" {
        return Err(AppError::usage(
            "upload destination must include a file name",
        ));
    }
    Ok((path.clone(), format!("{root}:{path}")))
}

fn files_download(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) = match parse_options(
        args,
        &["insecure", "overwrite"],
        &["to", "timeout", "protocol-trace"],
    ) {
        Ok(value) => value,
        Err(error) => {
            return write_error("files download", format, AppError::usage(error), out, err);
        }
    };
    let (name, requested_paths) = match positionals.split_first() {
        Some((name, paths)) if !paths.is_empty() => (name.as_str(), paths),
        _ => {
            return write_error(
                "files download",
                format,
                AppError::usage("files download requires a printer profile and device path"),
                out,
                err,
            );
        }
    };
    for requested_path in requested_paths {
        if let Err(error) = validate_known_file_root(requested_path) {
            return write_error("files download", format, error, out, err);
        }
    }
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("files download", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::FileDownload) {
        Ok(printer) => printer,
        Err(error) => return write_error("files download", format, error, out, err),
    };
    let device_paths = match requested_paths
        .iter()
        .map(|path| parse_device_path(path, file_root(printer.driver_kind)))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(paths) if paths.iter().all(|(path, _)| path != "/") => paths,
        Ok(_) => {
            return write_error(
                "files download",
                format,
                AppError::usage("cannot download a directory; specify a file path"),
                out,
                err,
            );
        }
        Err(error) => return write_error("files download", format, error, out, err),
    };
    let destinations = match download_destinations(
        &device_paths,
        options.value("to"),
        options.enabled("overwrite"),
    ) {
        Ok(paths) => paths,
        Err(error) => return write_error("files download", format, error, out, err),
    };
    let mut transfers = Vec::with_capacity(device_paths.len());
    for ((device_path, display_path), destination) in device_paths.into_iter().zip(destinations) {
        let transferred = match download_file(
            &printer,
            &device_path,
            &destination,
            options.enabled("overwrite"),
        ) {
            Ok(transferred) => transferred,
            Err(error) => return write_error("files download", format, error, out, err),
        };
        transfers.push(DownloadedFile {
            source: display_path,
            destination: destination.display().to_string(),
            bytes_transferred: transferred,
        });
    }
    let profile = printer.name;
    let driver = printer.driver_kind.name();
    let capabilities = file_capabilities(printer.driver_kind.capabilities());
    if transfers.len() == 1 {
        let transfer = transfers.pop().expect("one transfer");
        return write_success(
            "files download",
            format,
            FileTransferResult {
                profile,
                driver,
                warnings: Vec::new(),
                capabilities,
                transfer: transfer.clone(),
            },
            |out| writeln!(out, "Downloaded file to {}.", transfer.destination),
            out,
        );
    }
    write_success(
        "files download",
        format,
        FileTransfersData {
            profile,
            driver,
            files: transfers.clone(),
            warnings: Vec::new(),
            capabilities,
        },
        |out| {
            for transfer in &transfers {
                writeln!(out, "Downloaded file to {}.", transfer.destination)?;
            }
            Ok(())
        },
        out,
    )
}

fn download_file(
    printer: &ResolvedPrinter,
    device_path: &str,
    destination: &Path,
    overwrite: bool,
) -> Result<u64, AppError> {
    let parent = destination
        .parent()
        .expect("download destination has a parent");
    let mut temporary = match tempfile::NamedTempFile::new_in(parent) {
        Ok(file) => file,
        Err(_) => {
            return Err(AppError {
                exit_code: 1,
                code: "internal_error",
                message: "cannot create destination file".into(),
            });
        }
    };
    let transferred = match drivers::download_to(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        device_path,
        temporary.as_file_mut(),
    ) {
        Ok(transferred) => transferred,
        Err(error) => return Err(driver_error(error)),
    };
    if temporary.as_file_mut().sync_all().is_err() {
        return Err(AppError {
            exit_code: 1,
            code: "internal_error",
            message: "cannot finalize downloaded file".into(),
        });
    }
    let commit = if overwrite {
        temporary.persist(destination)
    } else {
        temporary.persist_noclobber(destination)
    };
    if commit.is_err() {
        return Err(AppError {
            exit_code: 1,
            code: "internal_error",
            message: "cannot move downloaded file into place".into(),
        });
    }
    Ok(transferred)
}

fn download_destinations(
    device_paths: &[(String, String)],
    destination: Option<&str>,
    overwrite: bool,
) -> Result<Vec<PathBuf>, AppError> {
    let multiple = device_paths.len() > 1;
    let directory = match destination {
        Some(value) if multiple => match fs::metadata(value) {
            Ok(metadata) if metadata.is_dir() => Some(PathBuf::from(value)),
            _ => {
                return Err(AppError::usage(
                    "--to must be an existing directory when downloading multiple files",
                ));
            }
        },
        Some(value) => return Ok(vec![download_destination(value, &device_paths[0].0)?]),
        None => None,
    };
    let mut destinations = Vec::with_capacity(device_paths.len());
    let mut seen = BTreeSet::new();
    for (device_path, _) in device_paths {
        let name = download_file_name(device_path)?;
        let path = directory
            .as_ref()
            .map_or_else(|| PathBuf::from(name), |directory| directory.join(name));
        if !seen.insert(path.clone()) {
            return Err(AppError::usage(format!(
                "multiple printer files would overwrite {}",
                path.display()
            )));
        }
        if path.exists() && !overwrite {
            return Err(AppError::usage(format!(
                "destination file already exists: {} (use --overwrite to replace)",
                path.display()
            )));
        }
        destinations.push(path);
    }
    Ok(destinations)
}

fn download_destination(destination: &str, device_path: &str) -> Result<PathBuf, AppError> {
    let name = download_file_name(device_path)?;
    let destination = PathBuf::from(destination);
    if destination.is_dir() {
        return Ok(destination.join(name));
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    match fs::metadata(parent) {
        Ok(metadata) if metadata.is_dir() => Ok(destination),
        _ => Err(AppError::usage(format!(
            "destination directory does not exist: {}",
            parent.display()
        ))),
    }
}

fn download_file_name(device_path: &str) -> Result<&str, AppError> {
    let name = device_path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AppError::usage("cannot download a directory"))?;
    Ok(name)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileTransferData {
    profile: String,
    driver: &'static str,
    source: String,
    destination: String,
    bytes_transferred: u64,
    warnings: Vec<String>,
    capabilities: FileCapabilities,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadedFile {
    source: String,
    destination: String,
    bytes_transferred: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileTransferResult {
    profile: String,
    driver: &'static str,
    transfer: DownloadedFile,
    warnings: Vec<String>,
    capabilities: FileCapabilities,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileTransfersData {
    profile: String,
    driver: &'static str,
    files: Vec<DownloadedFile>,
    warnings: Vec<String>,
    capabilities: FileCapabilities,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileDeleteData {
    device_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileDeleteResult {
    profile: String,
    driver: &'static str,
    file: FileDeleteData,
    warnings: Vec<String>,
    capabilities: FileCapabilities,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileDeletesData {
    profile: String,
    driver: &'static str,
    files: Vec<FileDeleteData>,
    warnings: Vec<String>,
    capabilities: FileCapabilities,
}

fn files_delete(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let command = "files delete";
    let (positionals, options) =
        match parse_options(args, &["yes", "insecure"], &["timeout", "protocol-trace"]) {
            Ok(value) => value,
            Err(error) => return write_error(command, format, AppError::usage(error), out, err),
        };
    let (name, requested_paths) = match positionals.split_first() {
        Some((name, paths)) if !paths.is_empty() => (name.as_str(), paths),
        _ => {
            return write_error(
                command,
                format,
                AppError::usage("files delete requires a printer profile and device path"),
                out,
                err,
            );
        }
    };
    let mut seen = BTreeSet::new();
    for requested_path in requested_paths {
        if let Err(error) = validate_known_file_root(requested_path) {
            return write_error(command, format, error, out, err);
        }
        if requested_path
            .split_once(':')
            .is_some_and(|(_, path)| path.is_empty() || path == "/")
        {
            return write_error(
                command,
                format,
                AppError::usage("cannot delete a directory; specify a file path"),
                out,
                err,
            );
        }
        if !seen.insert(requested_path.as_str()) {
            return write_error(
                command,
                format,
                AppError::usage(format!("duplicate device path {requested_path:?}")),
                out,
                err,
            );
        }
    }
    if let Err(code) = require_confirmation(
        command,
        options.enabled("yes"),
        &format!(
            "Delete {} file(s) from {name}? Type 'yes' to continue: ",
            requested_paths.len()
        ),
        format,
        out,
        err,
    ) {
        return code;
    }
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error(command, format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::FileDelete) {
        Ok(printer) => printer,
        Err(error) => return write_error(command, format, error, out, err),
    };
    let device_paths = match requested_paths
        .iter()
        .map(|path| parse_device_path(path, file_root(printer.driver_kind)))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(paths) => paths,
        Err(error) => return write_error(command, format, error, out, err),
    };
    let files = device_paths
        .iter()
        .map(|(_, display_path)| FileDeleteData {
            device_path: display_path.clone(),
        })
        .collect::<Vec<_>>();
    for (device_path, _) in &device_paths {
        if let Err(error) = drivers::delete_file(
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            device_path,
        ) {
            return write_error(command, format, driver_error(error), out, err);
        }
    }
    let profile = printer.name;
    let driver = printer.driver_kind.name();
    let capabilities = file_capabilities(printer.driver_kind.capabilities());
    if files.len() == 1 {
        let file = files.into_iter().next().expect("one deleted file");
        write_success(
            command,
            format,
            FileDeleteResult {
                profile,
                driver,
                file: file.clone(),
                warnings: Vec::new(),
                capabilities,
            },
            |out| writeln!(out, "Deleted {}.", file.device_path),
            out,
        )
    } else {
        write_success(
            command,
            format,
            FileDeletesData {
                profile,
                driver,
                files: files.clone(),
                warnings: Vec::new(),
                capabilities,
            },
            |out| {
                for file in &files {
                    writeln!(out, "Deleted {}.", file.device_path)?;
                }
                Ok(())
            },
            out,
        )
    }
}

fn jobs_preflight(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) = match parse_options(args, &[], &["display-name", "plate"]) {
        Ok(value) => value,
        Err(error) => {
            return write_error("jobs preflight", format, AppError::usage(error), out, err);
        }
    };
    let source = match positionals.as_slice() {
        [source] => PathBuf::from(source),
        _ => {
            return write_error(
                "jobs preflight",
                format,
                AppError::usage("jobs preflight requires one local 3MF path"),
                out,
                err,
            );
        }
    };
    let plate = match options.value("plate") {
        Some(value) => match value.parse::<u32>() {
            Ok(0) | Err(_) => {
                return write_error(
                    "jobs preflight",
                    format,
                    AppError::usage("--plate must be a positive integer"),
                    out,
                    err,
                );
            }
            Ok(value) => Some(value),
        },
        None => None,
    };
    let preflight = match melt_core::bambu::preflight_print_package(
        &source,
        options.value("display-name"),
        plate,
        None,
    ) {
        Ok(preflight) => preflight,
        Err(error) => {
            return write_error(
                "jobs preflight",
                format,
                AppError {
                    exit_code: 4,
                    code: "print_package_invalid",
                    message: error.to_string(),
                },
                out,
                err,
            );
        }
    };
    let human = preflight.clone();
    write_success(
        "jobs preflight",
        format,
        preflight,
        |out| {
            writeln!(out, "Source: {}", sanitize(&human.package.source_file_name))?;
            writeln!(
                out,
                "Remote name: {}",
                sanitize(&human.names.remote_filename)
            )?;
            writeln!(out, "Plate: {}", human.selected_plate.index)?;
            writeln!(
                out,
                "G-code: {}",
                sanitize(&human.selected_plate.gcode_path)
            )?;
            writeln!(out, "Ready: {}", human.ready)?;
            for issue in &human.issues {
                writeln!(
                    out,
                    "{:?}: {}: {}",
                    issue.severity,
                    issue.code,
                    sanitize(&issue.message)
                )?;
            }
            Ok(())
        },
        out,
    )
}

fn job_action(
    format: OutputFormat,
    action: &str,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) = match parse_options(
        args,
        &["yes", "insecure", "skip-leveling"],
        &["timeout", "protocol-trace", "plate"],
    ) {
        Ok(value) => value,
        Err(error) => {
            return write_error(
                &format!("jobs {action}"),
                format,
                AppError::usage(error),
                out,
                err,
            );
        }
    };
    let command = format!("jobs {action}");
    let (name, requested_path) = match (action, positionals.as_slice()) {
        ("start", [name, path]) => (name.as_str(), Some(path.as_str())),
        ("start", _) => {
            return write_error(
                &command,
                format,
                AppError::usage("jobs start requires a printer profile and device path"),
                out,
                err,
            );
        }
        (_, [name]) => (name.as_str(), None),
        _ => {
            return write_error(
                &command,
                format,
                AppError::usage(format!(
                    "{command} requires exactly one printer profile name"
                )),
                out,
                err,
            );
        }
    };
    if let Some(path) = requested_path {
        if let Err(error) = validate_known_file_root(path) {
            return write_error(&command, format, error, out, err);
        }
    }
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error(&command, format, error, out, err),
    };
    let operation = match action {
        "start" => drivers::Operation::JobStart,
        "pause" => drivers::Operation::JobPause,
        "resume" => drivers::Operation::JobResume,
        "cancel" => drivers::Operation::JobCancel,
        _ => unreachable!("job action is matched by dispatch"),
    };
    let printer = match resolve_printer(name, connection, operation) {
        Ok(printer) => printer,
        Err(error) => return write_error(&command, format, error, out, err),
    };
    if action == "start"
        && printer.driver_kind == drivers::Driver::Moonraker
        && (options.value("plate").is_some() || options.enabled("skip-leveling"))
    {
        return write_error(
            &command,
            format,
            AppError {
                exit_code: 5,
                code: "capability_unsupported",
                message: "Moonraker does not support Bambu job start options".into(),
            },
            out,
            err,
        );
    }
    let device_path = match requested_path {
        Some(path) => match parse_device_path(path, file_root(printer.driver_kind)) {
            Ok(path) => Some(path),
            Err(error) => return write_error(&command, format, error, out, err),
        },
        None => None,
    };
    let start_options = match options.value("plate") {
        Some(value) => match value.parse() {
            Ok(plate) => melt_core::bambu::JobStartOptions {
                plate: Some(plate),
                skip_leveling: options.enabled("skip-leveling"),
                ..Default::default()
            },
            Err(_) => {
                return write_error(
                    &command,
                    format,
                    AppError::usage("--plate must be a non-negative integer"),
                    out,
                    err,
                );
            }
        },
        None => melt_core::bambu::JobStartOptions {
            plate: None,
            skip_leveling: options.enabled("skip-leveling"),
            ..Default::default()
        },
    };
    let allowed = match action {
        "start" => &[melt_core::moonraker::PrinterState::Idle][..],
        "pause" => &[melt_core::moonraker::PrinterState::Printing][..],
        "resume" => &[melt_core::moonraker::PrinterState::Paused][..],
        "cancel" => &[
            melt_core::moonraker::PrinterState::Printing,
            melt_core::moonraker::PrinterState::Paused,
        ][..],
        _ => unreachable!("job action is matched by dispatch"),
    };
    if let Err(error) = require_state(&command, &printer, allowed) {
        return write_error(&command, format, error, out, err);
    }
    if let Err(code) = require_confirmation(
        &command,
        options.enabled("yes"),
        &format!(
            "{} the active print on {}? Type 'yes' to continue: ",
            action, printer.name
        ),
        format,
        out,
        err,
    ) {
        return code;
    }
    let result = match action {
        "start" => drivers::job_start(
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            &device_path
                .as_ref()
                .expect("jobs start always has a device path")
                .0,
            start_options,
        ),
        "pause" => drivers::job_pause(
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
        ),
        "resume" => drivers::job_resume(
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
        ),
        "cancel" => drivers::job_cancel(
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
        ),
        _ => unreachable!("job action is matched by dispatch"),
    };
    match result {
        Ok(result) => write_success(
            &command,
            format,
            JobActionData {
                profile: printer.name,
                driver: printer.driver_kind.name(),
                action: action.to_owned(),
                state: result.state,
                device_path: device_path.map(|(_, device_path)| device_path),
                warnings: Vec::new(),
                capabilities: printer.driver_kind.capabilities(),
            },
            |out| writeln!(out, "Job {action} command accepted."),
            out,
        ),
        Err(error) => write_error(&command, format, driver_error(error), out, err),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JobActionData {
    profile: String,
    driver: &'static str,
    action: String,
    state: melt_core::moonraker::PrinterState,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_path: Option<String>,
    warnings: Vec<String>,
    capabilities: drivers::Capabilities,
}

fn emergency_stop_command(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) =
        match parse_options(args, &["insecure"], &["timeout", "protocol-trace"]) {
            Ok(value) => value,
            Err(error) => {
                return write_error("emergency-stop", format, AppError::usage(error), out, err);
            }
        };
    let name = match one_positional("emergency-stop", &positionals) {
        Ok(name) => name,
        Err(error) => return write_error("emergency-stop", format, error, out, err),
    };
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("emergency-stop", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::EmergencyStop) {
        Ok(printer) => printer,
        Err(error) => return write_error("emergency-stop", format, error, out, err),
    };
    let recovery = match printer.driver_kind {
        drivers::Driver::BambuLan => "Power-cycle the printer to clear the emergency stop.",
        drivers::Driver::Moonraker => {
            "Run FIRMWARE_RESTART on the printer to clear the emergency stop."
        }
    };
    match drivers::emergency_stop(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
    ) {
        Ok(()) => write_success(
            "emergency-stop",
            format,
            EmergencyStopData {
                profile: printer.name,
                driver: printer.driver_kind.name(),
                action: "emergency-stop",
                recovery,
                capabilities: printer.driver_kind.capabilities(),
            },
            |out| writeln!(out, "Emergency stop sent."),
            out,
        ),
        Err(error) => write_error("emergency-stop", format, driver_error(error), out, err),
    }
}

#[derive(Serialize)]
struct EmergencyStopData {
    profile: String,
    driver: &'static str,
    action: &'static str,
    recovery: &'static str,
    capabilities: drivers::Capabilities,
}

fn temperature_set(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) = match parse_options(
        args,
        &["yes", "insecure"],
        &["nozzle", "bed", "chamber", "timeout", "protocol-trace"],
    ) {
        Ok(value) => value,
        Err(error) => {
            return write_error("temperature set", format, AppError::usage(error), out, err);
        }
    };
    let name = match one_positional("temperature set", &positionals) {
        Ok(name) => name,
        Err(error) => return write_error("temperature set", format, error, out, err),
    };
    let targets = match temperature_targets(&options) {
        Ok(targets) => targets,
        Err(error) => return write_error("temperature set", format, error, out, err),
    };
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("temperature set", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::TemperatureSet) {
        Ok(printer) => printer,
        Err(error) => return write_error("temperature set", format, error, out, err),
    };
    if let Err(error) = require_state(
        "temperature set",
        &printer,
        &[melt_core::moonraker::PrinterState::Idle],
    ) {
        return write_error("temperature set", format, error, out, err);
    }
    if let Err(code) = require_confirmation(
        "temperature set",
        options.enabled("yes"),
        &format!(
            "Set temperature targets on {}? Type 'yes' to continue: ",
            printer.name
        ),
        format,
        out,
        err,
    ) {
        return code;
    }
    match drivers::temperature_set(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        targets,
    ) {
        Ok(result) => write_success(
            "temperature set",
            format,
            TemperatureSetData {
                profile: printer.name,
                driver: printer.driver_kind.name(),
                targets: result.targets,
                warnings: Vec::new(),
                capabilities: printer.driver_kind.capabilities(),
            },
            |out| writeln!(out, "Temperature targets updated."),
            out,
        ),
        Err(error) => write_error("temperature set", format, driver_error(error), out, err),
    }
}

fn temperature_targets(
    options: &ParsedOptions,
) -> Result<melt_core::moonraker::TemperatureTargets, AppError> {
    let parse = |name: &str| {
        options
            .value(name)
            .map(|value| {
                value.parse::<f64>().map_err(|_| {
                    AppError::usage(format!("--{name} must be a temperature in Celsius"))
                })
            })
            .transpose()
    };
    let targets = melt_core::moonraker::TemperatureTargets {
        nozzle_celsius: parse("nozzle")?,
        bed_celsius: parse("bed")?,
        chamber_celsius: parse("chamber")?,
    };
    if targets.nozzle_celsius.is_none()
        && targets.bed_celsius.is_none()
        && targets.chamber_celsius.is_none()
    {
        return Err(AppError::usage(
            "at least one of --nozzle, --bed, --chamber is required",
        ));
    }
    Ok(targets)
}

#[derive(Serialize)]
struct TemperatureSetData {
    profile: String,
    driver: &'static str,
    targets: melt_core::moonraker::TemperatureTargets,
    warnings: Vec<String>,
    capabilities: drivers::Capabilities,
}

fn motion_home(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) = match parse_options(
        args,
        &["yes", "insecure"],
        &["axis", "timeout", "protocol-trace"],
    ) {
        Ok(value) => value,
        Err(error) => return write_error("motion home", format, AppError::usage(error), out, err),
    };
    let name = match one_positional("motion home", &positionals) {
        Ok(name) => name,
        Err(error) => return write_error("motion home", format, error, out, err),
    };
    let axes = match home_axes(options.value("axis")) {
        Ok(axes) => axes,
        Err(error) => return write_error("motion home", format, error, out, err),
    };
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("motion home", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::MotionHome) {
        Ok(printer) => printer,
        Err(error) => return write_error("motion home", format, error, out, err),
    };
    if let Err(error) = require_state(
        "motion home",
        &printer,
        &[melt_core::moonraker::PrinterState::Idle],
    ) {
        return write_error("motion home", format, error, out, err);
    }
    if let Err(code) = require_confirmation(
        "motion home",
        options.enabled("yes"),
        &format!("Home axes on {}? Type 'yes' to continue: ", printer.name),
        format,
        out,
        err,
    ) {
        return code;
    }
    match drivers::motion_home(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        &axes,
    ) {
        Ok(result) => write_success(
            "motion home",
            format,
            MotionData {
                profile: printer.name,
                driver: printer.driver_kind.name(),
                action: "home",
                state: result.state,
                warnings: Vec::new(),
                capabilities: printer.driver_kind.capabilities(),
            },
            |out| writeln!(out, "Homing command accepted."),
            out,
        ),
        Err(error) => write_error("motion home", format, driver_error(error), out, err),
    }
}

fn home_axes(value: Option<&str>) -> Result<Vec<melt_core::moonraker::Axis>, AppError> {
    let value = value.unwrap_or("x,y,z");
    let mut axes = Vec::new();
    for axis in value
        .split(',')
        .map(str::trim)
        .filter(|axis| !axis.is_empty())
    {
        let axis = match axis.to_ascii_lowercase().as_str() {
            "x" => melt_core::moonraker::Axis::X,
            "y" => melt_core::moonraker::Axis::Y,
            "z" => melt_core::moonraker::Axis::Z,
            _ => {
                return Err(AppError::usage(format!(
                    "invalid axis {axis:?}: use x, y, or z"
                )));
            }
        };
        if !axes.contains(&axis) {
            axes.push(axis);
        }
    }
    if axes.is_empty() {
        return Err(AppError::usage("at least one axis is required"));
    }
    Ok(axes)
}

fn motion_jog(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) = match parse_options(
        args,
        &["yes", "insecure"],
        &["x", "y", "z", "feedrate", "timeout", "protocol-trace"],
    ) {
        Ok(value) => value,
        Err(error) => return write_error("motion jog", format, AppError::usage(error), out, err),
    };
    let name = match one_positional("motion jog", &positionals) {
        Ok(name) => name,
        Err(error) => return write_error("motion jog", format, error, out, err),
    };
    let delta = match jog_delta(&options) {
        Ok(delta) => delta,
        Err(error) => return write_error("motion jog", format, error, out, err),
    };
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("motion jog", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::MotionJog) {
        Ok(printer) => printer,
        Err(error) => return write_error("motion jog", format, error, out, err),
    };
    if let Err(error) = require_state(
        "motion jog",
        &printer,
        &[melt_core::moonraker::PrinterState::Idle],
    ) {
        return write_error("motion jog", format, error, out, err);
    }
    if let Err(code) = require_confirmation(
        "motion jog",
        options.enabled("yes"),
        &format!("Move axes on {}? Type 'yes' to continue: ", printer.name),
        format,
        out,
        err,
    ) {
        return code;
    }
    match drivers::motion_jog(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        delta,
    ) {
        Ok(result) => write_success(
            "motion jog",
            format,
            MotionData {
                profile: printer.name,
                driver: printer.driver_kind.name(),
                action: "jog",
                state: result.state,
                warnings: Vec::new(),
                capabilities: printer.driver_kind.capabilities(),
            },
            |out| writeln!(out, "Jog command accepted."),
            out,
        ),
        Err(error) => write_error("motion jog", format, driver_error(error), out, err),
    }
}

fn jog_delta(options: &ParsedOptions) -> Result<melt_core::moonraker::JogDelta, AppError> {
    let parse = |name: &str| {
        options
            .value(name)
            .map(|value| {
                value
                    .parse::<f64>()
                    .map_err(|_| AppError::usage(format!("--{name} must be a distance in mm")))
            })
            .transpose()
    };
    let delta = melt_core::moonraker::JogDelta {
        x_millimeters: parse("x")?,
        y_millimeters: parse("y")?,
        z_millimeters: parse("z")?,
        feedrate_mm_per_min: options
            .value("feedrate")
            .map(|value| {
                value
                    .parse()
                    .map_err(|_| AppError::usage("--feedrate must be an integer"))
            })
            .transpose()?
            .unwrap_or(1500),
    };
    if delta.x_millimeters.is_none()
        && delta.y_millimeters.is_none()
        && delta.z_millimeters.is_none()
    {
        return Err(AppError::usage("at least one of --x, --y, --z is required"));
    }
    Ok(delta)
}

#[derive(Serialize)]
struct MotionData {
    profile: String,
    driver: &'static str,
    action: &'static str,
    state: melt_core::moonraker::MotionState,
    warnings: Vec<String>,
    capabilities: drivers::Capabilities,
}

fn fan_set(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) =
        match parse_options(args, &["yes", "insecure"], &["timeout", "protocol-trace"]) {
            Ok(value) => value,
            Err(error) => return write_error("fans set", format, AppError::usage(error), out, err),
        };
    let (name, fan, speed_percent) = match positionals.as_slice() {
        [name, fan, speed] => match speed.parse::<u8>() {
            Ok(speed) => (name.as_str(), fan.as_str(), speed),
            Err(_) => {
                return write_error(
                    "fans set",
                    format,
                    AppError::usage("fan percent must be an integer from 0 to 100"),
                    out,
                    err,
                );
            }
        },
        _ => {
            return write_error(
                "fans set",
                format,
                AppError::usage("fans set requires a printer profile, fan name, and percent"),
                out,
                err,
            );
        }
    };
    if speed_percent > 100 {
        return write_error(
            "fans set",
            format,
            AppError::usage("fan percent must be an integer from 0 to 100"),
            out,
            err,
        );
    }
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("fans set", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::FanSet) {
        Ok(printer) => printer,
        Err(error) => return write_error("fans set", format, error, out, err),
    };
    if let Err(error) = require_state(
        "fans set",
        &printer,
        &[
            melt_core::moonraker::PrinterState::Idle,
            melt_core::moonraker::PrinterState::Printing,
            melt_core::moonraker::PrinterState::Paused,
            melt_core::moonraker::PrinterState::Error,
        ],
    ) {
        return write_error("fans set", format, error, out, err);
    }
    if let Err(code) = require_confirmation(
        "fans set",
        options.enabled("yes"),
        &format!(
            "Set {fan} fan to {speed_percent}% on {}? Type 'yes' to continue: ",
            printer.name
        ),
        format,
        out,
        err,
    ) {
        return code;
    }
    match drivers::fan_set(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        fan,
        speed_percent,
    ) {
        Ok(result) => write_success(
            "fans set",
            format,
            FanSetData {
                profile: printer.name,
                driver: printer.driver_kind.name(),
                fan: result.fan,
                speed_percent: result.speed_percent,
                warnings: Vec::new(),
                capabilities: printer.driver_kind.capabilities(),
            },
            |out| writeln!(out, "Fan speed updated."),
            out,
        ),
        Err(error) => write_error("fans set", format, driver_error(error), out, err),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FanSetData {
    profile: String,
    driver: &'static str,
    fan: String,
    speed_percent: u8,
    warnings: Vec<String>,
    capabilities: drivers::Capabilities,
}

fn speed_set(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positionals, options) =
        match parse_options(args, &["yes", "insecure"], &["timeout", "protocol-trace"]) {
            Ok(value) => value,
            Err(error) => {
                return write_error("speed set", format, AppError::usage(error), out, err);
            }
        };
    let (name, speed_profile) = match positionals.as_slice() {
        [name, speed_profile] => (name.as_str(), speed_profile.as_str()),
        _ => {
            return write_error(
                "speed set",
                format,
                AppError::usage("speed set requires a printer profile and speed profile"),
                out,
                err,
            );
        }
    };
    if !["silent", "standard", "sport", "ludicrous"].contains(&speed_profile) {
        return write_error(
            "speed set",
            format,
            AppError::usage("speed profile must be silent, standard, sport, or ludicrous"),
            out,
            err,
        );
    }
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("speed set", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::SpeedSet) {
        Ok(printer) => printer,
        Err(error) => return write_error("speed set", format, error, out, err),
    };
    if let Err(error) = require_state(
        "speed set",
        &printer,
        &[
            melt_core::moonraker::PrinterState::Printing,
            melt_core::moonraker::PrinterState::Paused,
        ],
    ) {
        return write_error("speed set", format, error, out, err);
    }
    if let Err(code) = require_confirmation(
        "speed set",
        options.enabled("yes"),
        &format!(
            "Set speed profile {speed_profile} on {}? Type 'yes' to continue: ",
            printer.name
        ),
        format,
        out,
        err,
    ) {
        return code;
    }
    match drivers::speed_set(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        speed_profile,
    ) {
        Ok(result) => write_success(
            "speed set",
            format,
            SpeedSetData {
                profile: printer.name,
                driver: printer.driver_kind.name(),
                speed_profile: result.speed_profile,
                warnings: Vec::new(),
                capabilities: printer.driver_kind.capabilities(),
            },
            |out| writeln!(out, "Speed profile updated."),
            out,
        ),
        Err(error) => write_error("speed set", format, driver_error(error), out, err),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpeedSetData {
    profile: String,
    driver: &'static str,
    speed_profile: String,
    warnings: Vec<String>,
    capabilities: drivers::Capabilities,
}

fn driver_error(error: DriverError) -> AppError {
    match error {
        DriverError::Unknown(_) | DriverError::InvalidProfile(_) | DriverError::InvalidTimeout => {
            AppError::usage(error.to_string())
        }
        DriverError::UnsupportedOperation(_, _) => AppError {
            exit_code: 5,
            code: "capability_unsupported",
            message: error.to_string(),
        },
        DriverError::Moonraker(melt_core::moonraker::Error::Authentication) => AppError {
            exit_code: 3,
            code: "authentication_failed",
            message: "printer authentication failed".into(),
        },
        DriverError::Moonraker(melt_core::moonraker::Error::Timeout) => AppError {
            exit_code: 4,
            code: "timeout",
            message: "printer operation timed out".into(),
        },
        DriverError::Moonraker(melt_core::moonraker::Error::Transport(_)) => AppError {
            exit_code: 4,
            code: "connection_failed",
            message: "printer connection failed".into(),
        },
        DriverError::Moonraker(melt_core::moonraker::Error::HttpStatus(status))
            if matches!(status.as_u16(), 502..=504) =>
        {
            AppError {
                exit_code: 4,
                code: "connection_failed",
                message: "printer connection failed".into(),
            }
        }
        DriverError::Moonraker(melt_core::moonraker::Error::Unsupported(_)) => AppError {
            exit_code: 5,
            code: "capability_unsupported",
            message: error.to_string(),
        },
        DriverError::Moonraker(melt_core::moonraker::Error::UpdateRefreshRejected) => AppError {
            exit_code: 4,
            code: "operation_rejected",
            message: "Moonraker rejected the metadata refresh while the updater or printer was busy"
                .into(),
        },
        DriverError::Moonraker(melt_core::moonraker::Error::InvalidDevicePath)
        | DriverError::Moonraker(melt_core::moonraker::Error::InvalidTemperatureTarget)
        | DriverError::Moonraker(melt_core::moonraker::Error::InvalidJog)
        | DriverError::Moonraker(melt_core::moonraker::Error::InvalidSpeedProfile)
        | DriverError::Moonraker(melt_core::moonraker::Error::FileAlreadyExists)
        | DriverError::Moonraker(melt_core::moonraker::Error::DirectoryDestination) => {
            AppError::usage(error.to_string())
        }
        DriverError::Moonraker(melt_core::moonraker::Error::LocalIo(_)) => AppError {
            exit_code: 1,
            code: "internal_error",
            message: "local file operation failed".into(),
        },
        DriverError::Bambu(
            melt_core::bambu::TransportError::Authentication
            | melt_core::bambu::TransportError::Pin(_),
        ) => AppError {
            exit_code: 3,
            code: "authentication_failed",
            message: "printer authentication failed".into(),
        },
        DriverError::Bambu(melt_core::bambu::TransportError::UnsignedCommand) => AppError {
            exit_code: 5,
            code: "signing_required",
            message: "printer rejected an unsigned command; Melt does not support signed mutations, so enable Developer Mode/LAN-only mode".into(),
        },
        DriverError::Bambu(melt_core::bambu::TransportError::AuthorizationRequired(_)) => {
            AppError {
                exit_code: 5,
                code: "signing_required",
                message: "observed printer security requires signed commands; Melt does not support signed mutations, so enable Developer Mode/LAN-only mode".into(),
            }
        }
        DriverError::Bambu(melt_core::bambu::TransportError::AuthorizationConflict(_)) => {
            AppError {
                exit_code: 5,
                code: "authorization_conflict",
                message: "conflicting live authorization evidence blocks this mutation; refresh status and run `melt printer capabilities <name>` before retrying".into(),
            }
        }
        DriverError::Bambu(melt_core::bambu::TransportError::Timeout) => AppError {
            exit_code: 4,
            code: "timeout",
            message: "printer operation timed out".into(),
        },
        DriverError::Bambu(
            melt_core::bambu::TransportError::Connection
            | melt_core::bambu::TransportError::Tls
            | melt_core::bambu::TransportError::MissingCertificate
            | melt_core::bambu::TransportError::FileTransfer,
        ) => AppError {
            exit_code: 4,
            code: "connection_failed",
            message: "printer connection failed".into(),
        },
        DriverError::Bambu(
            melt_core::bambu::TransportError::Unsupported(_)
            | melt_core::bambu::TransportError::InvalidSpeedProfile,
        ) => AppError {
            exit_code: 5,
            code: "capability_unsupported",
            message: error.to_string(),
        },
        DriverError::Bambu(
            melt_core::bambu::TransportError::MissingAccessCode
            | melt_core::bambu::TransportError::InvalidAccessCode
            | melt_core::bambu::TransportError::InvalidDevicePath
            | melt_core::bambu::TransportError::FileAlreadyExists
            | melt_core::bambu::TransportError::DirectoryDestination
            | melt_core::bambu::TransportError::InvalidTemperatureTarget
            | melt_core::bambu::TransportError::InvalidMotion,
        ) => AppError::usage(error.to_string()),
        DriverError::Bambu(melt_core::bambu::TransportError::LocalIo) => AppError {
            exit_code: 1,
            code: "internal_error",
            message: "local file operation failed".into(),
        },
        DriverError::Bambu(_) => AppError {
            exit_code: 1,
            code: "printer_unavailable",
            message: "printer request failed".into(),
        },
        DriverError::Moonraker(_) => AppError {
            exit_code: 1,
            code: "printer_unavailable",
            message: "printer request failed".into(),
        },
        DriverError::Camera(_) => AppError {
            exit_code: 1,
            code: "printer_unavailable",
            message: "printer camera request failed".into(),
        },
    }
}

fn remove_profile(
    format: OutputFormat,
    name: &str,
    yes: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    if !yes && !std::io::stdin().is_terminal() {
        return write_error(
            "printer remove",
            format,
            AppError::usage("non-interactive mode requires --yes"),
            out,
            err,
        );
    }
    if !yes {
        let _ = write!(
            err,
            "Remove printer profile {name} and its stored secrets? Type 'yes' to continue: "
        );
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() {
            return write_error(
                "printer remove",
                format,
                AppError {
                    exit_code: 1,
                    code: "internal_error",
                    message: "cannot read confirmation".into(),
                },
                out,
                err,
            );
        }
        if answer.trim_end() != "yes" {
            return write_error(
                "printer remove",
                format,
                AppError::usage("confirmation declined; profile not removed"),
                out,
                err,
            );
        }
    }

    let dir = match melt_core::config::config_dir() {
        Ok(dir) => dir,
        Err(error) => {
            return write_error(
                "printer remove",
                format,
                AppError {
                    exit_code: 1,
                    code: "internal_error",
                    message: error.to_string(),
                },
                out,
                err,
            );
        }
    };
    match profiles::remove(dir, &SystemKeychain, name) {
        Ok(result) => write_success(
            "printer remove",
            format,
            RemoveData { removed: result },
            |out| writeln!(out, "Printer profile removed: {name}"),
            out,
        ),
        Err(error) => write_error("printer remove", format, profile_error(error), out, err),
    }
}

#[derive(Serialize)]
struct RemoveData {
    removed: profiles::RemoveResult,
}

#[derive(Serialize)]
struct AddData {
    profile: profiles::CreateResult,
}

fn profile_error(error: ProfileError) -> AppError {
    match error {
        ProfileError::MissingName
        | ProfileError::InvalidName
        | ProfileError::InvalidHost
        | ProfileError::InvalidAccessCode
        | ProfileError::InvalidTlsFingerprint
        | ProfileError::MissingAccessCode
        | ProfileError::NotFound(_) => AppError::usage(error.to_string()),
        ProfileError::Driver(error) => driver_error(error),
        ProfileError::Secret(_) => AppError {
            exit_code: 3,
            code: "secret_store_failed",
            message: "keychain operation failed".into(),
        },
        ProfileError::Config(ConfigError::ProfileAlreadyExists) => {
            AppError::usage("profile already exists")
        }
        ProfileError::Config(_) | ProfileError::RollbackFailed => AppError {
            exit_code: 1,
            code: "internal_error",
            message: error.to_string(),
        },
    }
}

fn write_drivers(format: OutputFormat, out: &mut dyn Write) -> i32 {
    let drivers = drivers::registered();
    match format {
        OutputFormat::Json => write_success(
            "printer drivers",
            format,
            DriverList {
                drivers: drivers.to_vec(),
            },
            |_| Ok(()),
            out,
        ),
        OutputFormat::Human => {
            let mut rows = vec![vec!["DRIVER".to_owned(), "DESCRIPTION".to_owned()]];
            rows.extend(
                drivers
                    .iter()
                    .map(|driver| vec![driver.name.to_owned(), driver.description.to_owned()]),
            );
            write_padded_table(out, &rows).map_or(1, |()| 0)
        }
    }
}

impl<'a> Invocation<'a> {
    fn parse(args: &'a [String]) -> Result<Self, AppError> {
        let mut command = Vec::new();
        let mut format = OutputFormat::Human;
        let mut index = 0;
        while index < args.len() {
            if args[index] == "--output" {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| AppError::usage("--output requires a value"))?;
                format = match value.as_str() {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => {
                        return Err(AppError::usage(format!(
                            "invalid output format {value:?}: must be human or json"
                        )));
                    }
                };
                index += 2;
                continue;
            }
            if let Some(value) = args[index].strip_prefix("--output=") {
                format = match value {
                    "human" => OutputFormat::Human,
                    "json" => OutputFormat::Json,
                    _ => {
                        return Err(AppError::usage(format!(
                            "invalid output format {value:?}: must be human or json"
                        )));
                    }
                };
                index += 1;
                continue;
            }
            if matches!(args[index].as_str(), "--verbose" | "-v") {
                index += 1;
                continue;
            }
            command.push(&args[index]);
            index += 1;
        }
        if command.is_empty() {
            return Err(AppError::usage("a command is required"));
        }
        Ok(Self { command, format })
    }
}

fn command_name(command: &[&String]) -> String {
    command
        .iter()
        .map(|part| part.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_success<T: Serialize>(
    command: &str,
    format: OutputFormat,
    data: T,
    human: impl FnOnce(&mut dyn Write) -> std::io::Result<()>,
    out: &mut dyn Write,
) -> i32 {
    match format {
        OutputFormat::Human => human(out).map_or(1, |_| 0),
        OutputFormat::Json => {
            let envelope = Envelope {
                ok: true,
                data: Some(data),
                error: None,
                meta: meta(command),
            };
            write_json(out, &envelope).map_or(1, |_| 0)
        }
    }
}

#[derive(Serialize)]
struct PrinterProfile<'a> {
    name: &'a str,
    driver: &'a str,
    host: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    serial: &'a str,
    timeout: &'a str,
    insecure: bool,
}

#[derive(Serialize)]
struct PrinterList<'a> {
    profiles: Vec<PrinterProfile<'a>>,
}

#[derive(Serialize)]
struct DriverList {
    drivers: Vec<drivers::DriverInfo>,
}

fn write_printer_list(
    format: OutputFormat,
    profiles: Vec<NamedProfile>,
    out: &mut dyn Write,
) -> i32 {
    let data = profiles
        .iter()
        .map(|named| PrinterProfile {
            name: &named.name,
            driver: &named.profile.driver,
            host: &named.profile.host,
            serial: &named.profile.serial,
            timeout: &named.profile.timeout,
            insecure: named.profile.insecure,
        })
        .collect::<Vec<_>>();
    match format {
        OutputFormat::Json => write_success(
            "printer list",
            format,
            PrinterList { profiles: data },
            |_| Ok(()),
            out,
        ),
        OutputFormat::Human if data.is_empty() => {
            writeln!(out, "No printer profiles configured.").map_or(1, |_| 0)
        }
        OutputFormat::Human => {
            let mut rows = vec![
                ["NAME", "DRIVER", "HOST", "SERIAL", "TIMEOUT", "INSECURE"]
                    .map(str::to_owned)
                    .to_vec(),
            ];
            rows.extend(data.iter().map(|profile| {
                vec![
                    profile.name.to_string(),
                    profile.driver.to_string(),
                    profile.host.to_string(),
                    profile.serial.to_string(),
                    profile.timeout.to_string(),
                    profile.insecure.to_string(),
                ]
            }));
            write_padded_table(out, &rows).map_or(1, |()| 0)
        }
    }
}

/// Column-aligned table with a two-space gap, matching the reference tables.
/// The final column is never padded, so rows carry no trailing whitespace.
fn write_padded_table(out: &mut dyn Write, rows: &[Vec<String>]) -> std::io::Result<()> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    for row in rows {
        for (column, cell) in row.iter().enumerate() {
            if column + 1 == row.len() {
                writeln!(out, "{cell}")?;
            } else {
                let pad = widths[column] - cell.chars().count() + 2;
                write!(out, "{cell}{:pad$}", "")?;
            }
        }
    }
    Ok(())
}

fn config_error(error: ConfigError) -> AppError {
    match error {
        ConfigError::UnsupportedVersion { .. } => {
            AppError::usage("unsupported config schema version")
        }
        ConfigError::Malformed(_) => AppError::usage("config file is malformed"),
        _ => AppError {
            exit_code: 1,
            code: "internal_error",
            message: "failed to read config".into(),
        },
    }
}

fn write_error(
    command: &str,
    format: OutputFormat,
    error: AppError,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    if matches!(format, OutputFormat::Json) {
        let envelope: Envelope<()> = Envelope {
            ok: false,
            data: None,
            error: Some(ErrorDetail {
                code: error.code,
                message: &error.message,
                details: BTreeMap::new(),
            }),
            // The reference implementation reports durationMs on success only.
            meta: Meta {
                command,
                duration_ms: None,
            },
        };
        let _ = write_json(out, &envelope);
    } else {
        let _ = writeln!(err, "Error: {}", error.message);
    }
    error.exit_code
}

fn write_json<T: Serialize>(out: &mut dyn Write, value: &T) -> serde_json::Result<()> {
    serde_json::to_writer_pretty(&mut *out, value)?;
    writeln!(out).map_err(serde_json::Error::io)
}

#[cfg(test)]
mod tests {
    fn sample_status() -> moonraker::Status {
        moonraker::Status {
            state: moonraker::PrinterState::Printing,
            temperatures: Some(moonraker::Temperatures {
                nozzle: Some(moonraker::Temperature {
                    current_celsius: 212.5,
                    target_celsius: Some(220.0),
                }),
                bed: Some(moonraker::Temperature {
                    current_celsius: 59.0,
                    target_celsius: Some(60.0),
                }),
                chamber: Some(moonraker::Temperature {
                    current_celsius: 26.0,
                    target_celsius: None,
                }),
            }),
            job: Some(moonraker::Job {
                name: "cube.3mf".into(),
                id: "job-1".into(),
            }),
            progress: Some(moonraker::Progress {
                percent: 61,
                preparation_percent: None,
                current_layer: Some(3),
                total_layers: Some(100),
            }),
            errors: Vec::new(),
            warnings: Vec::new(),
            fans: [("auxiliary".to_string(), 15u8), ("partCooling".into(), 9)]
                .into_iter()
                .collect(),
            time_estimates: Some(moonraker::TimeEstimates {
                elapsed_seconds: 0,
                remaining_seconds: Some(5460),
                total_seconds: None,
            }),
            speed_level: Some("standard".into()),
            wifi: Some(moonraker::Wifi { signal_dbm: -68 }),
            lights: [("chamber_light".to_string(), "on".to_string())]
                .into_iter()
                .collect(),
            controls: moonraker::ControlInventory {
                fans: [
                    (
                        "partCooling".to_string(),
                        moonraker::FanControl {
                            kind: moonraker::FanKind::Parts,
                            speed_percent: Some(9),
                            mode: moonraker::FanMode::Manual,
                            minimum_percent: 0,
                            maximum_percent: 100,
                            controllable: true,
                        },
                    ),
                    (
                        "auxiliary".to_string(),
                        moonraker::FanControl {
                            kind: moonraker::FanKind::Auxiliary,
                            speed_percent: Some(15),
                            mode: moonraker::FanMode::Manual,
                            minimum_percent: 0,
                            maximum_percent: 100,
                            controllable: true,
                        },
                    ),
                ]
                .into_iter()
                .collect(),
                ..moonraker::ControlInventory::default()
            },
            print_meta: Some(moonraker::PrintMeta {
                file_name: "cube.3mf".into(),
                file_size: Some(2 * 1024 * 1024),
                nozzle_diameter: Some(0.4),
                bed_type: Some("textured_plate".into()),
                plate_index: None,
                plate_count: None,
                print_type: None,
                queue_position: None,
                queue_total: None,
            }),
            stage: Some("printing"),
            stage_code: Some(0),
            timelapse: Some(moonraker::Timelapse {
                recording: false,
                progress: None,
                ready: None,
            }),
            gcode_position: Some(moonraker::GcodePosition {
                z_mm: 0.0,
                current_line: 120,
                total_lines: 4000,
            }),
            firmware_version: Some("01.08.00.00".into()),
            extensions: moonraker::Extensions {
                bambu_lan: Some(moonraker::BambuExtension {
                    ams: Some(moonraker::AmsData {
                        units: vec![moonraker::AmsUnit {
                            id: 0,
                            humidity_range: Some("20-30%"),
                            humidity_level: Some("moderate"),
                            temperature: Some(25.4),
                            trays: vec![
                                moonraker::AmsTray {
                                    slot: 0,
                                    filament_type: Some("PLA".into()),
                                    color: Some("161616FF".into()),
                                    remaining_percent: Some(42),
                                    nozzle_temp_min: None,
                                    nozzle_temp_max: None,
                                    ..Default::default()
                                },
                                moonraker::AmsTray {
                                    slot: 1,
                                    filament_type: None,
                                    color: None,
                                    remaining_percent: None,
                                    nozzle_temp_min: None,
                                    nozzle_temp_max: None,
                                    ..Default::default()
                                },
                            ],
                            ..Default::default()
                        }],
                    }),
                    sd_card_state: Some("normal"),
                    emmc_storage: None,
                    extruder_count: None,
                    mqtt_alive_supported: None,
                    status_transport: None,
                    reported_ip: None,
                    hms: Vec::new(),
                    airduct_fans: false,
                }),
            },
        }
    }

    #[test]
    fn detailed_status_lists_airduct_fans_from_the_control_inventory() {
        // H2-class firmware reports fans under `device.airduct`, which lands in
        // `controls.fans`. `Status::fans` only ever carries legacy slot names,
        // so reading it leaves these printers showing no fans at all.
        let fan = |kind, speed| moonraker::FanControl {
            kind,
            speed_percent: Some(speed),
            mode: moonraker::FanMode::Manual,
            minimum_percent: 0,
            maximum_percent: 100,
            controllable: true,
        };
        let mut status = sample_status();
        status.fans.clear();
        status.controls.fans = [
            ("parts".to_string(), fan(moonraker::FanKind::Parts, 75)),
            ("exhaust".to_string(), fan(moonraker::FanKind::Exhaust, 40)),
        ]
        .into_iter()
        .collect();

        let detailed = human_status("dakota", &status, true);
        assert!(
            detailed.contains("Fans:\n  Part cooling: 75%\n  Exhaust: 40%"),
            "{detailed}"
        );
    }

    #[test]
    fn human_status_matches_the_reference_layout() {
        let brief = human_status("dakota", &sample_status(), false);
        assert_eq!(
            brief,
            "Printer: dakota\nState: printing\nProgress: 61%\nNozzle: 212.5 C / 220.0 C\nBed: 59.0 C / 60.0 C\nChamber: 26.0 C\nJob: cube.3mf"
        );

        let detailed = human_status("dakota", &sample_status(), true);
        assert!(detailed.contains("Progress: 61% (layer 3 / 100)"));
        // Known fans keep the reference order rather than the map's ordering.
        assert!(detailed.contains("Fans:\n  Part cooling: 9%\n  Auxiliary: 15%"));
        // Extended sections follow the reference implementation's ordering.
        assert!(detailed.contains("Stage: printing"), "{detailed}");
        assert!(detailed.contains("Speed: standard"), "{detailed}");
        assert!(detailed.contains("Time: 1h 31m remaining"), "{detailed}");
        assert!(detailed.contains("Wi-Fi: -68 dBm"), "{detailed}");
        assert!(
            detailed.contains("Lights:\n  chamber_light: on"),
            "{detailed}"
        );
        assert!(
            detailed.contains("Job: cube.3mf (2.0 MB, 0.4mm nozzle, textured_plate)"),
            "{detailed}"
        );
        assert!(detailed.contains("G-code: line 120 / 4000"), "{detailed}");
        assert!(detailed.contains("Timelapse: off"), "{detailed}");
        assert!(
            detailed.contains(
                "AMS:\n  Unit 0 (humidity: 20-30% [moderate], temp: 25.4 C):\n    Slot 0: PLA 161616FF (42%)\n    Slot 1: (empty)"
            ),
            "{detailed}"
        );
    }

    #[test]
    fn brief_status_drops_the_detail_only_fields() {
        let brief = sample_status().without_extended();
        assert!(brief.fans.is_empty());
        assert!(brief.lights.is_empty());
        assert!(brief.wifi.is_none());
        assert!(brief.speed_level.is_none());
        assert!(brief.extensions.is_empty());
        // Base fields survive.
        assert!(brief.progress.is_some());
        assert!(brief.temperatures.is_some());
    }

    #[test]
    fn human_status_neutralizes_printer_supplied_escape_sequences() {
        let mut status = sample_status();
        status.job = Some(moonraker::Job {
            name: "evil\u{1b}[2Jname".into(),
            id: "job-1".into(),
        });
        let rendered = human_status("dakota", &status, false);
        assert!(!rendered.contains('\u{1b}'), "{rendered}");
        assert!(rendered.contains("Job: evil\u{fffd}[2Jname"), "{rendered}");
    }

    use super::*;
    use melt_core::config::Profile;

    #[test]
    fn tables_align_every_column_but_the_last() {
        let rows = vec![
            ["NAME", "DRIVER", "HOST"].map(str::to_owned).to_vec(),
            ["alaska", "bambu-lan", "10.20.20.5"]
                .map(str::to_owned)
                .to_vec(),
        ];
        let mut out = Vec::new();

        write_padded_table(&mut out, &rows).unwrap();

        // Two spaces past the widest cell, and no trailing padding to strip.
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "NAME    DRIVER     HOST\nalaska  bambu-lan  10.20.20.5\n"
        );
    }

    #[test]
    fn file_sizes_and_timestamps_use_the_reference_display_forms() {
        assert_eq!(optional_size(None), "-");
        assert_eq!(optional_size(Some(16)), "16 B");
        assert_eq!(optional_size(Some(229_376)), "224.0 KiB");
        assert_eq!(optional_size(Some(6_601_748)), "6.3 MiB");
        assert_eq!(optional_size(Some(2_147_483_648)), "2.0 GiB");
        assert_eq!(
            format_modified_time("2026-06-19T00:44:00Z"),
            "2026-06-19 00:44 UTC"
        );
        // Anything that is not plain UTC passes through untouched.
        assert_eq!(format_modified_time("whenever"), "whenever");
    }

    #[test]
    fn firmware_update_human_output_sanitizes_provider_text() {
        use melt_core::firmware_updates::{
            FirmwareUpdateComponent, FirmwareUpdateIssue, FirmwareUpdateSource,
        };

        let report = FirmwareUpdateReport::from_components(
            FirmwareUpdateSource::BambuMqtt,
            vec![FirmwareUpdateComponent {
                id: "ota".into(),
                label: "Printer\u{1b}[31m".into(),
                kind: FirmwareUpdateComponentKind::PrinterFirmware,
                current_version: Some("1.0\nold".into()),
                available_version: Some("2.0".into()),
                availability: FirmwareUpdateAvailability::Available,
                required: true,
            }],
            vec![FirmwareUpdateIssue {
                code: "test",
                message: "Review\rnow",
            }],
        );

        let output = human_firmware_updates("workshop", &report);
        assert!(!output.contains('\u{1b}'));
        assert!(!output.contains("old\n"));
        assert!(output.contains("Availability: update available"));
        assert!(output.contains("Required: yes"));
    }

    #[test]
    fn firmware_update_human_output_names_every_availability_state() {
        use melt_core::firmware_updates::{FirmwareUpdateComponent, FirmwareUpdateSource};

        for (availability, expected) in [
            (FirmwareUpdateAvailability::Available, "update available"),
            (FirmwareUpdateAvailability::Current, "current"),
            (FirmwareUpdateAvailability::Unknown, "unknown"),
        ] {
            let report = FirmwareUpdateReport::from_components(
                FirmwareUpdateSource::BambuMqtt,
                vec![FirmwareUpdateComponent {
                    id: "ota".into(),
                    label: "Printer".into(),
                    kind: FirmwareUpdateComponentKind::PrinterFirmware,
                    current_version: Some("1.0".into()),
                    available_version: None,
                    availability,
                    required: false,
                }],
                Vec::new(),
            );
            assert!(
                human_firmware_updates("workshop", &report)
                    .contains(&format!("Availability: {expected}"))
            );
        }

        let unsupported = FirmwareUpdateReport::unsupported(
            FirmwareUpdateSource::MoonrakerUpdateManager,
            "missing",
            "missing",
        );
        assert!(
            human_firmware_updates("workshop", &unsupported).contains("Availability: unsupported")
        );
    }

    #[test]
    fn every_leaf_command_documents_itself() {
        for leaf in LEAF_COMMANDS {
            let mut args = leaf
                .path
                .iter()
                .map(|part| (*part).into())
                .collect::<Vec<String>>();
            args.push("--help".into());
            let mut out = Vec::new();
            let mut err = Vec::new();

            assert_eq!(run(&args, &mut out, &mut err), 0, "{:?}", leaf.path);
            assert!(err.is_empty(), "{:?}", leaf.path);
            let output = String::from_utf8(out).unwrap();
            assert!(output.starts_with(leaf.short), "{:?}", leaf.path);
            assert!(
                output.contains(&format!("melt {} {}", leaf.path.join(" "), leaf.args)),
                "{:?}",
                leaf.path
            );
            assert!(output.contains("\nGlobal Flags:\n"), "{:?}", leaf.path);
        }
    }

    #[test]
    fn bare_commands_render_cobra_style_group_help() {
        for group in COMMAND_GROUPS {
            let args = group
                .path
                .iter()
                .map(|part| (*part).into())
                .collect::<Vec<String>>();
            let mut out = Vec::new();
            let mut err = Vec::new();

            assert_eq!(run(&args, &mut out, &mut err), 0, "{:?}", group.path);
            assert!(err.is_empty(), "{:?}", group.path);
            let output = String::from_utf8(out).unwrap();
            let command = group_command_name(group);
            assert!(output.starts_with(&format!("{}\n\nUsage:\n  melt", group.short)));
            assert!(
                output.contains(&format!(
                    "Use \"melt{} [command] --help\"",
                    if group.path.is_empty() {
                        String::new()
                    } else {
                        format!(" {command}")
                    }
                )),
                "{:?}",
                group.path
            );
        }
    }

    #[test]
    fn bare_group_help_ignores_global_output_and_verbose_options() {
        let args = [
            "camera".into(),
            "--output".into(),
            "json".into(),
            "--verbose".into(),
        ];
        let mut out = Vec::new();
        let mut err = Vec::new();

        assert_eq!(run(&args, &mut out, &mut err), 0);
        assert!(err.is_empty());
        assert_eq!(
            String::from_utf8(out).unwrap(),
            concat!(
                "Camera operations on a named printer\n\n",
                "Usage:\n  melt camera [command]\n\n",
                "Available Commands:\n",
                "  snapshot    Capture one still image from a printer camera\n",
                "  stream      Stream camera feed from a printer via a local HTTP server\n\n",
                "Flags:\n  -h, --help   help for camera\n\n",
                "Global Flags:\n",
                "      --output string   output format: human or json (default \"human\")\n",
                "  -v, --verbose         show detailed progress output\n\n",
                "Use \"melt camera [command] --help\" for more information about a command.\n"
            )
        );
    }

    #[test]
    fn nested_bare_group_help_accepts_invalid_output() {
        let args = [
            "printer".into(),
            "tls".into(),
            "--output=not-a-format".into(),
        ];
        let mut out = Vec::new();
        let mut err = Vec::new();

        assert_eq!(run(&args, &mut out, &mut err), 0);
        assert!(err.is_empty());
        let output = String::from_utf8(out).unwrap();
        assert!(output.starts_with("Manage TLS settings for a printer profile\n\n"));
        assert!(output.contains("  refresh     Re-pin or disable TLS certificate"));
    }

    #[test]
    fn unknown_bare_group_flag_uses_the_group_json_error_envelope() {
        let args = [
            "camera".into(),
            "--unknown".into(),
            "--output".into(),
            "json".into(),
        ];
        let mut out = Vec::new();
        let mut err = Vec::new();

        assert_eq!(run(&args, &mut out, &mut err), 2);
        assert!(err.is_empty());
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains(r#""code": "config_error""#));
        assert!(output.contains(r#""command": "camera""#));
    }

    #[test]
    fn version_json_is_an_envelope() {
        let args = ["version".into(), "--output".into(), "json".into()];
        let mut out = Vec::new();
        assert_eq!(run(&args, &mut out, &mut Vec::new()), 0);
        assert!(String::from_utf8(out).unwrap().contains(r#""ok": true"#));
    }

    #[test]
    fn invalid_command_uses_the_json_error_envelope() {
        let args = ["unknown".into(), "--output".into(), "json".into()];
        let mut out = Vec::new();
        assert_eq!(run(&args, &mut out, &mut Vec::new()), 2);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains(r#""code": "config_error""#)
        );
    }

    #[test]
    fn printer_list_keeps_the_existing_json_data_shape() {
        let profiles = vec![NamedProfile {
            name: "garage".into(),
            profile: Profile {
                driver: "moonraker".into(),
                host: "printer.local".into(),
                serial: String::new(),
                model: String::new(),
                timeout: "10s".into(),
                insecure: false,
                created: String::new(),
                updated: String::new(),
            },
        }];
        let mut out = Vec::new();

        assert_eq!(
            write_printer_list(OutputFormat::Json, profiles, &mut out),
            0
        );

        let output = String::from_utf8(out).unwrap();
        assert!(output.contains(r#""command": "printer list""#));
        assert!(output.contains(r#""profiles": ["#));
        assert!(output.contains(r#""name": "garage""#));
        assert!(!output.contains(r#""serial""#));
    }

    #[test]
    fn printer_drivers_keeps_the_existing_json_data_shape() {
        let mut out = Vec::new();

        assert_eq!(write_drivers(OutputFormat::Json, &mut out), 0);

        let output = String::from_utf8(out).unwrap();
        assert!(output.contains(r#""command": "printer drivers""#));
        assert!(output.contains(r#""drivers": ["#));
        assert!(output.contains("bambu-lan"));
    }

    #[test]
    fn unsupported_driver_status_uses_the_capability_error_contract() {
        let error = driver_error(DriverError::UnsupportedOperation(
            drivers::Driver::BambuLan,
            drivers::Operation::Status,
        ));

        assert_eq!(error.exit_code, 5);
        assert_eq!(error.code, "capability_unsupported");
    }

    #[test]
    fn moonraker_timeout_uses_the_connection_exit_contract() {
        let error = driver_error(DriverError::Moonraker(melt_core::moonraker::Error::Timeout));

        assert_eq!(error.exit_code, 4);
        assert_eq!(error.code, "timeout");
    }

    #[test]
    fn unsigned_bambu_commands_use_the_capability_exit_contract() {
        let error = driver_error(DriverError::Bambu(
            melt_core::bambu::TransportError::UnsignedCommand,
        ));

        assert_eq!(error.exit_code, 5);
        assert_eq!(error.code, "signing_required");
        assert!(
            error
                .message
                .contains("Melt does not support signed mutations")
        );
        assert!(!error.message.contains("signed client"));
    }

    #[test]
    fn printer_add_parses_the_existing_profile_flags() {
        let raw = [
            "--driver".into(),
            "moonraker".into(),
            "--host".into(),
            "printer.local".into(),
            "--serial".into(),
            "unused".into(),
            "--model".into(),
            "P1S".into(),
            "--timeout".into(),
            "5s".into(),
            "--insecure".into(),
        ];
        let flags = raw.iter().collect::<Vec<_>>();

        let request = parse_add_request("Garage", &flags).unwrap();

        assert_eq!(request.name, "Garage");
        assert_eq!(request.driver, "moonraker");
        assert_eq!(request.host, "printer.local");
        assert_eq!(request.model, "P1S");
        assert_eq!(request.timeout, "5s");
        assert!(request.insecure);
    }

    #[test]
    fn profile_removal_requires_yes_when_not_interactive() {
        let args = [
            "printer".into(),
            "remove".into(),
            "garage".into(),
            "--output".into(),
            "json".into(),
        ];
        let mut out = Vec::new();

        assert_eq!(run(&args, &mut out, &mut Vec::new()), 2);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("non-interactive mode requires --yes")
        );
    }

    #[test]
    fn global_verbose_flag_does_not_change_command_dispatch() {
        let args = ["--verbose".into(), "version".into(), "--output=json".into()];
        let mut out = Vec::new();

        assert_eq!(run(&args, &mut out, &mut Vec::new()), 0);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains(r#""command": "version""#)
        );
    }

    #[test]
    fn camera_stream_rejects_non_mjpeg_format_before_loading_a_profile() {
        let args = [
            "camera".into(),
            "stream".into(),
            "garage".into(),
            "--format".into(),
            "h264".into(),
            "--output".into(),
            "json".into(),
        ];
        let mut out = Vec::new();

        assert_eq!(run(&args, &mut out, &mut Vec::new()), 2);
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains(r#""command": "camera stream""#));
        assert!(output.contains(r#"invalid --format \"h264\": must be \"mjpeg\" or omitted"#));
    }

    #[test]
    fn discovery_rejects_drivers_without_the_capability_without_scanning() {
        let args = [
            "printer".into(),
            "discover".into(),
            "--driver".into(),
            "moonraker".into(),
            "--output".into(),
            "json".into(),
        ];
        let mut out = Vec::new();

        assert_eq!(run(&args, &mut out, &mut Vec::new()), 5);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("capability_unsupported")
        );
    }

    #[test]
    fn discover_rejects_protocol_trace_since_it_has_no_resolved_profile() {
        let args = [
            "printer".into(),
            "discover".into(),
            "--driver".into(),
            "bambu-lan".into(),
            "--protocol-trace".into(),
            "/tmp/trace.jsonl".into(),
            "--output".into(),
            "json".into(),
        ];
        let mut out = Vec::new();

        assert_eq!(run(&args, &mut out, &mut Vec::new()), 5);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("capability_unsupported")
        );
    }

    #[test]
    fn tls_refresh_requires_explicit_confirmation_when_non_interactive() {
        let args = [
            "printer".into(),
            "tls".into(),
            "refresh".into(),
            "garage".into(),
            "--output".into(),
            "json".into(),
        ];
        let mut out = Vec::new();

        assert_eq!(run(&args, &mut out, &mut Vec::new()), 2);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("non-interactive mode requires --yes")
        );
    }

    #[test]
    fn unsafe_file_roots_are_rejected_before_loading_a_profile() {
        let args = [
            "files".into(),
            "download".into(),
            "garage".into(),
            "config:/moonraker.conf".into(),
            "--output".into(),
            "json".into(),
        ];
        let mut out = Vec::new();

        assert_eq!(run(&args, &mut out, &mut Vec::new()), 2);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("unsupported file root")
        );
    }

    #[test]
    fn file_deletion_requires_yes_when_not_interactive() {
        let args = [
            "files".into(),
            "delete".into(),
            "garage".into(),
            "gcodes:/cube.gcode".into(),
            "--output".into(),
            "json".into(),
        ];
        let mut out = Vec::new();

        assert_eq!(run(&args, &mut out, &mut Vec::new()), 2);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("non-interactive mode requires --yes")
        );
    }

    #[test]
    fn file_deletion_rejects_the_storage_root_before_loading_a_profile() {
        let args = [
            "files".into(),
            "delete".into(),
            "garage".into(),
            "gcodes:/".into(),
            "--yes".into(),
            "--output".into(),
            "json".into(),
        ];
        let mut out = Vec::new();

        assert_eq!(run(&args, &mut out, &mut Vec::new()), 2);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("cannot delete a directory")
        );
    }

    #[test]
    fn file_deletion_rejects_duplicate_paths_before_loading_a_profile() {
        let args = [
            "files".into(),
            "delete".into(),
            "garage".into(),
            "gcodes:/cube.gcode".into(),
            "gcodes:/cube.gcode".into(),
            "--yes".into(),
            "--output".into(),
            "json".into(),
        ];
        let mut out = Vec::new();

        assert_eq!(run(&args, &mut out, &mut Vec::new()), 2);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("duplicate device path")
        );
    }

    #[test]
    fn batch_download_requires_a_directory_and_rejects_name_collisions() {
        let directory = tempfile::tempdir().unwrap();
        let paths = vec![
            (
                "/models/cube.gcode".to_owned(),
                "gcodes:/models/cube.gcode".to_owned(),
            ),
            (
                "/backup/cube.gcode".to_owned(),
                "gcodes:/backup/cube.gcode".to_owned(),
            ),
        ];

        let error = download_destinations(&paths, Some("download.gcode"), false).unwrap_err();
        assert!(error.message.contains("existing directory"));

        let error = download_destinations(&paths, Some(directory.path().to_str().unwrap()), false)
            .unwrap_err();
        assert!(error.message.contains("would overwrite"));
    }

    #[test]
    fn batch_download_maps_each_file_to_the_requested_directory() {
        let directory = tempfile::tempdir().unwrap();
        let paths = vec![
            (
                "/models/cube.gcode".to_owned(),
                "gcodes:/models/cube.gcode".to_owned(),
            ),
            (
                "/models/benchy.gcode".to_owned(),
                "gcodes:/models/benchy.gcode".to_owned(),
            ),
        ];

        let destinations =
            download_destinations(&paths, Some(directory.path().to_str().unwrap()), false).unwrap();
        assert_eq!(
            destinations,
            vec![
                directory.path().join("cube.gcode"),
                directory.path().join("benchy.gcode")
            ]
        );
    }

    #[test]
    fn snapshot_writes_atomically_without_clobbering() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("snapshot.jpg");
        fs::write(&destination, b"original").unwrap();

        assert!(write_snapshot_file(&destination, b"replacement", false).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        assert_eq!(
            write_snapshot_file(&destination, b"replacement", true).unwrap(),
            11
        );
        assert_eq!(fs::read(&destination).unwrap(), b"replacement");
        assert!(write_snapshot_file(&destination, &[], true).is_err());
    }

    #[test]
    fn stream_requests_require_loopback_host_and_one_client() {
        assert!(is_camera_stream_request(
            "GET /stream HTTP/1.1\r\nHost: 127.0.0.1:8080\r\n\r\n"
        ));
        assert!(is_camera_stream_request(
            "GET /stream HTTP/1.1\r\nHost: localhost:8080\r\n\r\n"
        ));
        assert!(is_camera_stream_request(
            "GET /stream HTTP/1.1\r\nHost: [::1]:8080\r\n\r\n"
        ));
        assert!(!is_camera_stream_request(
            "GET /stream HTTP/1.1\r\nHost: printer.example\r\n\r\n"
        ));
        assert!(!is_camera_stream_request(
            "GET /other HTTP/1.1\r\nHost: localhost\r\n\r\n"
        ));
        assert!(!is_camera_stream_request(
            "POST /stream HTTP/1.1\r\nHost: localhost\r\n\r\n"
        ));

        let active = AtomicBool::new(false);
        assert!(claim_stream_client(&active));
        assert!(!claim_stream_client(&active));
        active.store(false, Ordering::Release);
        assert!(claim_stream_client(&active));
    }

    #[test]
    fn stream_duration_accepts_go_style_compound_and_fractional_values() {
        assert_eq!(
            parse_stream_duration("1m30s"),
            Some(Duration::from_secs(90))
        );
        assert_eq!(
            parse_stream_duration("1.5s"),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(
            parse_stream_duration("+500ms"),
            Some(Duration::from_millis(500))
        );
        assert_eq!(parse_stream_duration("0s"), None);
        assert_eq!(parse_stream_duration("-1s"), None);
        assert_eq!(parse_stream_duration("1m30"), None);
    }

    #[test]
    fn stream_request_reader_accepts_fragmented_headers() {
        struct FragmentedReader<'a> {
            chunks: &'a [&'a [u8]],
            index: usize,
        }

        impl Read for FragmentedReader<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let Some(chunk) = self.chunks.get(self.index) else {
                    return Ok(0);
                };
                self.index += 1;
                buffer[..chunk.len()].copy_from_slice(chunk);
                Ok(chunk.len())
            }
        }

        let chunks = [
            b"GET /stream HTTP/1.1\r\nHo".as_slice(),
            b"st: [::1]:8080\r\n".as_slice(),
            b"\r\n".as_slice(),
        ];
        let mut reader = FragmentedReader {
            chunks: &chunks,
            index: 0,
        };
        let header = read_camera_stream_header(&mut reader).unwrap();

        assert!(is_camera_stream_request(
            std::str::from_utf8(&header).unwrap()
        ));
    }

    #[test]
    fn lights_accept_chamber_aliases_and_exact_states() {
        for alias in ["chamber", "chamber-light", "chamber_light", "CHAMBER"] {
            assert_eq!(normalize_light(alias).unwrap(), "chamber_light");
        }
        for alias in ["aux", "aux-light", "aux_light", "AUX"] {
            assert_eq!(normalize_light(alias).unwrap(), "aux_light");
        }
        assert_eq!(normalize_light("work-light").unwrap(), "work-light");
        assert!(normalize_light("bad light").is_err());
        assert_eq!(
            melt_core::moonraker::LightState::parse("on"),
            Some(melt_core::moonraker::LightState::On)
        );
        assert!(melt_core::moonraker::LightState::parse("ON").is_none());
    }
}
