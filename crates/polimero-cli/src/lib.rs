//! CLI rendering for shared Polimero operations.

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

use polimero_core::{
    AppError, app_info,
    config::{Config, ConfigError, NamedProfile, config_dir},
    drivers::{self, DriverError},
    keychain::{SERVICE, SecretError, SecretStore, SystemKeychain, account},
    profiles::{self, ProfileError},
};
use serde::Serialize;

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
            ("fans", "Fan control operations on a named printer"),
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
        path: &["files"],
        short: "File operations on a named printer",
        commands: &[
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
struct Meta<'a> {
    command: &'a str,
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
        Err(error) => return write_error("polimero", OutputFormat::Human, error, out, err),
    };
    let command = command_name(&invocation.command);

    match invocation.command.as_slice() {
        [version] if version.as_str() == "version" => write_success(
            "version",
            invocation.format,
            VersionData {
                version: app_info().version,
                commit: "unknown",
                go_version: "rust",
                platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
                modes: ["gui", "headless"],
            },
            |out| writeln!(out, "polimero version {}", app_info().version),
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
        [status, rest @ ..] if status.as_str() == "status" => {
            printer_status(invocation.format, rest, out, err)
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
        [jobs, action, rest @ ..]
            if jobs.as_str() == "jobs"
                && matches!(action.as_str(), "start" | "pause" | "resume" | "cancel") =>
        {
            job_action(invocation.format, action, rest, out, err)
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
            "polimero",
            invocation.format,
            AppError::usage(format!("unknown command {command:?} for \"polimero\"")),
            out,
            err,
        ),
    }
}

fn run_bare_group(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Option<i32> {
    let mut path = Vec::new();
    let mut unknown_flag = None;
    let mut json_requested = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--output" => {
                let value = args.get(index + 1)?;
                json_requested |= value == "json";
                index += 2;
            }
            "--verbose" | "-v" | "--help" | "-h" => index += 1,
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

fn group_command_name(group: &CommandGroup) -> String {
    match group.path {
        [] => "polimero".into(),
        path => path.join(" "),
    }
}

fn write_group_help(group: &CommandGroup, out: &mut dyn Write) -> i32 {
    let command = group_command_name(group);
    let mut help = format!("{}\n\nUsage:\n  polimero", group.short);
    if !group.path.is_empty() {
        help.push(' ');
        help.push_str(&command);
    }
    help.push_str(" [command]\n\nAvailable Commands:\n");
    for (name, description) in group.commands {
        help.push_str(&format!("  {name:<11} {description}\n"));
    }
    if group.path.is_empty() {
        help.push_str(
            "\nFlags:\n  -h, --help            help for polimero\n      --output string   output format: human or json (default \"human\")\n  -v, --verbose         show detailed progress output\n      --version         version for polimero\n",
        );
    } else {
        help.push_str(&format!(
            "\nFlags:\n  -h, --help   help for {}\n\nGlobal Flags:\n      --output string   output format: human or json (default \"human\")\n  -v, --verbose         show detailed progress output\n",
            group.path.last().expect("non-root groups have a name")
        ));
    }
    help.push_str(&format!(
        "\nUse \"polimero{} [command] --help\" for more information about a command.\n",
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
                    code: "internal-error",
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
        timeout: "10s".into(),
        insecure: false,
        access_code: String::new(),
    };
    let mut index = 0;
    while index < flags.len() {
        let flag = flags[index].as_str();
        match flag {
            "--insecure" => request.insecure = true,
            "--driver" | "--host" | "--serial" | "--timeout" | "--access-code-file" => {
                let value = flags
                    .get(index + 1)
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                match flag {
                    "--driver" => request.driver = (*value).clone(),
                    "--host" => request.host = (*value).clone(),
                    "--serial" => request.serial = (*value).clone(),
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
            let state = status.state;
            write_success(
                "status",
                format,
                StatusData {
                    profile: printer.name,
                    driver: printer.driver_kind.name(),
                    status,
                    capabilities: printer.driver_kind.capabilities(),
                },
                |out| writeln!(out, "STATE\t{state:?}"),
                out,
            )
        }
        Err(error) => write_error("status", format, driver_error(error), out, err),
    }
}

#[derive(Serialize)]
struct StatusData {
    profile: String,
    driver: &'static str,
    #[serde(flatten)]
    status: polimero_core::moonraker::Status,
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

fn connection_options(options: &ParsedOptions) -> Result<ConnectionOptions, AppError> {
    if options.value("protocol-trace").is_some() {
        return Err(AppError {
            exit_code: 5,
            code: "capability-unsupported",
            message: "protocol tracing is not available in the Rust driver yet".into(),
        });
    }
    Ok(ConnectionOptions {
        timeout: options.value("timeout").map(str::to_owned),
        insecure: options.enabled("insecure"),
    })
}

fn resolve_printer(
    requested_name: &str,
    connection: ConnectionOptions,
    operation: drivers::Operation,
) -> Result<ResolvedPrinter, AppError> {
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
    let driver = drivers::profile(&profile).map_err(driver_error)?;
    let driver_kind = driver.driver();
    if !driver_kind.supports(operation) {
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
                    code: "secret-not-found",
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
                    code: "secret-store-failed",
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
                    code: "secret-not-found",
                    message: "required printer TLS fingerprint is unavailable".into(),
                });
            }
            Err(SecretError::Unavailable(_)) => {
                return Err(AppError {
                    exit_code: 3,
                    code: "secret-store-failed",
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
                code: "internal-error",
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
    allowed: &[polimero_core::moonraker::PrinterState],
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
        code: "invalid-printer-state",
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
            code: "internal-error",
            message: "camera snapshot returned empty image data".into(),
        });
    }
    validate_snapshot_destination(destination, overwrite)?;
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|_| AppError {
        exit_code: 1,
        code: "internal-error",
        message: "cannot create snapshot file".into(),
    })?;
    temporary.write_all(image).map_err(|_| AppError {
        exit_code: 1,
        code: "internal-error",
        message: "cannot write snapshot file".into(),
    })?;
    temporary.as_file_mut().sync_all().map_err(|_| AppError {
        exit_code: 1,
        code: "internal-error",
        message: "cannot finalize snapshot file".into(),
    })?;
    let committed = if overwrite {
        temporary.persist(destination)
    } else {
        temporary.persist_noclobber(destination)
    };
    committed.map_err(|_| AppError {
        exit_code: 1,
        code: "internal-error",
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
                        code: "internal-error",
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
                code: "internal-error",
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
    stream: polimero_core::bambu::MjpegStream,
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
    let state = match polimero_core::moonraker::LightState::parse(state_name) {
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
            polimero_core::moonraker::PrinterState::Idle,
            polimero_core::moonraker::PrinterState::Printing,
            polimero_core::moonraker::PrinterState::Paused,
            polimero_core::moonraker::PrinterState::Error,
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
            let display = if result.light == "chamber" {
                "Chamber".into()
            } else {
                result.light.clone()
            };
            let state = match result.state {
                polimero_core::moonraker::LightState::On => "on",
                polimero_core::moonraker::LightState::Off => "off",
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
        "chamber" | "chamber-light" | "chamber_light" => Ok("chamber".into()),
        _ => Ok(light.into()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LightSetData {
    profile: String,
    driver: &'static str,
    light: String,
    state: polimero_core::moonraker::LightState,
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
    match polimero_core::bambu::discover(timeout) {
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
                code: "connection-failed",
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
                    code: "internal-error",
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
                    writeln!(out, "ROOT\tWRITABLE\tDESCRIPTION")?;
                    for root in &human_roots {
                        writeln!(
                            out,
                            "{}\t{}\t{}",
                            root.name, root.writable, root.description
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
        |out| writeln!(out, "File listing retrieved."),
        out,
    )
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
}

fn file_capabilities(capabilities: drivers::Capabilities) -> FileCapabilities {
    FileCapabilities {
        file_list: capabilities.file_list,
        file_download: capabilities.file_download,
        file_upload: capabilities.file_upload,
    }
}

#[derive(Serialize)]
struct FileRootsData {
    profile: String,
    driver: &'static str,
    roots: Vec<polimero_core::moonraker::FileRoot>,
    warnings: Vec<String>,
    capabilities: FileCapabilities,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FilePathData {
    device_path: String,
    entries: Vec<polimero_core::moonraker::FileEntry>,
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
    let (name, requested_path) = match positionals.as_slice() {
        [name, path] => (name.as_str(), path.as_str()),
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
    if let Err(error) = validate_known_file_root(requested_path) {
        return write_error("files download", format, error, out, err);
    }
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error("files download", format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::FileDownload) {
        Ok(printer) => printer,
        Err(error) => return write_error("files download", format, error, out, err),
    };
    let device_path = match parse_device_path(requested_path, file_root(printer.driver_kind)) {
        Ok(path) => path,
        Err(error) => return write_error("files download", format, error, out, err),
    };
    if device_path.0 == "/" {
        return write_error(
            "files download",
            format,
            AppError::usage("cannot download a directory; specify a file path"),
            out,
            err,
        );
    }
    let destination = match download_destination(options.value("to"), &device_path.0) {
        Ok(path) => path,
        Err(error) => return write_error("files download", format, error, out, err),
    };
    if destination.exists() && !options.enabled("overwrite") {
        return write_error(
            "files download",
            format,
            AppError::usage(format!(
                "destination file already exists: {} (use --overwrite to replace)",
                destination.display()
            )),
            out,
            err,
        );
    }
    let parent = destination
        .parent()
        .expect("download destination has a parent");
    let mut temporary = match tempfile::NamedTempFile::new_in(parent) {
        Ok(file) => file,
        Err(_) => {
            return write_error(
                "files download",
                format,
                AppError {
                    exit_code: 1,
                    code: "internal-error",
                    message: "cannot create destination file".into(),
                },
                out,
                err,
            );
        }
    };
    let transferred = match drivers::download_to(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        &device_path.0,
        temporary.as_file_mut(),
    ) {
        Ok(transferred) => transferred,
        Err(error) => return write_error("files download", format, driver_error(error), out, err),
    };
    if temporary.as_file_mut().sync_all().is_err() {
        return write_error(
            "files download",
            format,
            AppError {
                exit_code: 1,
                code: "internal-error",
                message: "cannot finalize downloaded file".into(),
            },
            out,
            err,
        );
    }
    let commit = if options.enabled("overwrite") {
        temporary.persist(&destination)
    } else {
        temporary.persist_noclobber(&destination)
    };
    if commit.is_err() {
        return write_error(
            "files download",
            format,
            AppError {
                exit_code: 1,
                code: "internal-error",
                message: "cannot move downloaded file into place".into(),
            },
            out,
            err,
        );
    }
    let destination = destination.display().to_string();
    write_success(
        "files download",
        format,
        FileTransferData {
            profile: printer.name,
            driver: printer.driver_kind.name(),
            source: device_path.1,
            destination: destination.clone(),
            bytes_transferred: transferred,
            warnings: Vec::new(),
            capabilities: file_capabilities(printer.driver_kind.capabilities()),
        },
        |out| writeln!(out, "Downloaded file to {destination}."),
        out,
    )
}

fn download_destination(destination: Option<&str>, device_path: &str) -> Result<PathBuf, AppError> {
    let name = device_path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AppError::usage("cannot download a directory"))?;
    let destination = destination
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(name));
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
                code: "capability-unsupported",
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
            Ok(plate) => polimero_core::bambu::JobStartOptions {
                plate: Some(plate),
                skip_leveling: options.enabled("skip-leveling"),
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
        None => polimero_core::bambu::JobStartOptions {
            plate: None,
            skip_leveling: options.enabled("skip-leveling"),
        },
    };
    let allowed = match action {
        "start" => &[polimero_core::moonraker::PrinterState::Idle][..],
        "pause" => &[polimero_core::moonraker::PrinterState::Printing][..],
        "resume" => &[polimero_core::moonraker::PrinterState::Paused][..],
        "cancel" => &[
            polimero_core::moonraker::PrinterState::Printing,
            polimero_core::moonraker::PrinterState::Paused,
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
    state: polimero_core::moonraker::PrinterState,
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
        &[polimero_core::moonraker::PrinterState::Idle],
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
) -> Result<polimero_core::moonraker::TemperatureTargets, AppError> {
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
    let targets = polimero_core::moonraker::TemperatureTargets {
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
    targets: polimero_core::moonraker::TemperatureTargets,
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
        &[polimero_core::moonraker::PrinterState::Idle],
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

fn home_axes(value: Option<&str>) -> Result<Vec<polimero_core::moonraker::Axis>, AppError> {
    let value = value.unwrap_or("x,y,z");
    let mut axes = Vec::new();
    for axis in value
        .split(',')
        .map(str::trim)
        .filter(|axis| !axis.is_empty())
    {
        let axis = match axis.to_ascii_lowercase().as_str() {
            "x" => polimero_core::moonraker::Axis::X,
            "y" => polimero_core::moonraker::Axis::Y,
            "z" => polimero_core::moonraker::Axis::Z,
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
        &[polimero_core::moonraker::PrinterState::Idle],
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

fn jog_delta(options: &ParsedOptions) -> Result<polimero_core::moonraker::JogDelta, AppError> {
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
    let delta = polimero_core::moonraker::JogDelta {
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
    state: polimero_core::moonraker::MotionState,
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
            polimero_core::moonraker::PrinterState::Idle,
            polimero_core::moonraker::PrinterState::Printing,
            polimero_core::moonraker::PrinterState::Paused,
            polimero_core::moonraker::PrinterState::Error,
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
            polimero_core::moonraker::PrinterState::Printing,
            polimero_core::moonraker::PrinterState::Paused,
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
            code: "capability-unsupported",
            message: error.to_string(),
        },
        DriverError::Moonraker(polimero_core::moonraker::Error::Authentication) => AppError {
            exit_code: 3,
            code: "authentication-failed",
            message: "printer authentication failed".into(),
        },
        DriverError::Moonraker(polimero_core::moonraker::Error::Timeout) => AppError {
            exit_code: 4,
            code: "timeout",
            message: "printer operation timed out".into(),
        },
        DriverError::Moonraker(polimero_core::moonraker::Error::Transport(_)) => AppError {
            exit_code: 4,
            code: "connection-failed",
            message: "printer connection failed".into(),
        },
        DriverError::Moonraker(polimero_core::moonraker::Error::HttpStatus(status))
            if matches!(status.as_u16(), 502..=504) =>
        {
            AppError {
                exit_code: 4,
                code: "connection-failed",
                message: "printer connection failed".into(),
            }
        }
        DriverError::Moonraker(polimero_core::moonraker::Error::Unsupported(_)) => AppError {
            exit_code: 5,
            code: "capability-unsupported",
            message: error.to_string(),
        },
        DriverError::Moonraker(polimero_core::moonraker::Error::InvalidDevicePath)
        | DriverError::Moonraker(polimero_core::moonraker::Error::InvalidTemperatureTarget)
        | DriverError::Moonraker(polimero_core::moonraker::Error::InvalidJog)
        | DriverError::Moonraker(polimero_core::moonraker::Error::InvalidSpeedProfile)
        | DriverError::Moonraker(polimero_core::moonraker::Error::FileAlreadyExists)
        | DriverError::Moonraker(polimero_core::moonraker::Error::DirectoryDestination) => {
            AppError::usage(error.to_string())
        }
        DriverError::Moonraker(polimero_core::moonraker::Error::LocalIo(_)) => AppError {
            exit_code: 1,
            code: "internal-error",
            message: "local file operation failed".into(),
        },
        DriverError::Bambu(
            polimero_core::bambu::TransportError::Authentication
            | polimero_core::bambu::TransportError::UnsignedCommand
            | polimero_core::bambu::TransportError::Pin(_),
        ) => AppError {
            exit_code: 3,
            code: "authentication-failed",
            message: "printer authentication failed".into(),
        },
        DriverError::Bambu(polimero_core::bambu::TransportError::Timeout) => AppError {
            exit_code: 4,
            code: "timeout",
            message: "printer operation timed out".into(),
        },
        DriverError::Bambu(
            polimero_core::bambu::TransportError::Connection
            | polimero_core::bambu::TransportError::Tls
            | polimero_core::bambu::TransportError::MissingCertificate
            | polimero_core::bambu::TransportError::FileTransfer,
        ) => AppError {
            exit_code: 4,
            code: "connection-failed",
            message: "printer connection failed".into(),
        },
        DriverError::Bambu(
            polimero_core::bambu::TransportError::Unsupported(_)
            | polimero_core::bambu::TransportError::InvalidSpeedProfile,
        ) => AppError {
            exit_code: 5,
            code: "capability-unsupported",
            message: error.to_string(),
        },
        DriverError::Bambu(
            polimero_core::bambu::TransportError::MissingAccessCode
            | polimero_core::bambu::TransportError::InvalidAccessCode
            | polimero_core::bambu::TransportError::InvalidDevicePath
            | polimero_core::bambu::TransportError::FileAlreadyExists
            | polimero_core::bambu::TransportError::DirectoryDestination
            | polimero_core::bambu::TransportError::InvalidTemperatureTarget
            | polimero_core::bambu::TransportError::InvalidMotion,
        ) => AppError::usage(error.to_string()),
        DriverError::Bambu(polimero_core::bambu::TransportError::LocalIo) => AppError {
            exit_code: 1,
            code: "internal-error",
            message: "local file operation failed".into(),
        },
        DriverError::Bambu(_) => AppError {
            exit_code: 1,
            code: "printer-unavailable",
            message: "printer request failed".into(),
        },
        DriverError::Moonraker(_) => AppError {
            exit_code: 1,
            code: "printer-unavailable",
            message: "printer request failed".into(),
        },
        DriverError::Camera(_) => AppError {
            exit_code: 1,
            code: "printer-unavailable",
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
                    code: "internal-error",
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

    let dir = match polimero_core::config::config_dir() {
        Ok(dir) => dir,
        Err(error) => {
            return write_error(
                "printer remove",
                format,
                AppError {
                    exit_code: 1,
                    code: "internal-error",
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
            code: "secret-store-failed",
            message: "keychain operation failed".into(),
        },
        ProfileError::Config(ConfigError::ProfileAlreadyExists) => {
            AppError::usage("profile already exists")
        }
        ProfileError::Config(_) | ProfileError::RollbackFailed => AppError {
            exit_code: 1,
            code: "internal-error",
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
            let _ = writeln!(out, "DRIVER\tDESCRIPTION");
            for driver in drivers {
                if writeln!(out, "{}\t{}", driver.name, driver.description).is_err() {
                    return 1;
                }
            }
            0
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
                meta: Meta { command },
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
            let _ = writeln!(out, "NAME\tDRIVER\tHOST\tSERIAL\tTIMEOUT\tINSECURE");
            for profile in data {
                if writeln!(
                    out,
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    profile.name,
                    profile.driver,
                    profile.host,
                    profile.serial,
                    profile.timeout,
                    profile.insecure
                )
                .is_err()
                {
                    return 1;
                }
            }
            0
        }
    }
}

fn config_error(error: ConfigError) -> AppError {
    match error {
        ConfigError::UnsupportedVersion { .. } => {
            AppError::usage("unsupported config schema version")
        }
        ConfigError::Malformed(_) => AppError::usage("config file is malformed"),
        _ => AppError {
            exit_code: 1,
            code: "internal-error",
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
            meta: Meta { command },
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
    use super::*;
    use polimero_core::config::Profile;

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
            assert!(output.starts_with(&format!("{}\n\nUsage:\n  polimero", group.short)));
            assert!(
                output.contains(&format!(
                    "Use \"polimero{} [command] --help\"",
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
                "Usage:\n  polimero camera [command]\n\n",
                "Available Commands:\n",
                "  snapshot    Capture one still image from a printer camera\n",
                "  stream      Stream camera feed from a printer via a local HTTP server\n\n",
                "Flags:\n  -h, --help   help for camera\n\n",
                "Global Flags:\n",
                "      --output string   output format: human or json (default \"human\")\n",
                "  -v, --verbose         show detailed progress output\n\n",
                "Use \"polimero camera [command] --help\" for more information about a command.\n"
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
        assert!(output.contains(r#""code": "config-error""#));
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
                .contains(r#""code": "config-error""#)
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
        assert_eq!(error.code, "capability-unsupported");
    }

    #[test]
    fn moonraker_timeout_uses_the_connection_exit_contract() {
        let error = driver_error(DriverError::Moonraker(
            polimero_core::moonraker::Error::Timeout,
        ));

        assert_eq!(error.exit_code, 4);
        assert_eq!(error.code, "timeout");
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
            "--timeout".into(),
            "5s".into(),
            "--insecure".into(),
        ];
        let flags = raw.iter().collect::<Vec<_>>();

        let request = parse_add_request("Garage", &flags).unwrap();

        assert_eq!(request.name, "Garage");
        assert_eq!(request.driver, "moonraker");
        assert_eq!(request.host, "printer.local");
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
                .contains("capability-unsupported")
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
            assert_eq!(normalize_light(alias).unwrap(), "chamber");
        }
        assert_eq!(normalize_light("work-light").unwrap(), "work-light");
        assert!(normalize_light("bad light").is_err());
        assert_eq!(
            polimero_core::moonraker::LightState::parse("on"),
            Some(polimero_core::moonraker::LightState::On)
        );
        assert!(polimero_core::moonraker::LightState::parse("ON").is_none());
    }
}
