//! `melt plug` profile and attachment commands. A plug target is always a
//! named option (`--printer` or `--camera`), because both kinds can share a
//! name.

use std::{collections::BTreeMap, io::Write, path::Path};

use melt_core::{
    AppError,
    config::{PlugTarget, config_dir},
    keychain::SystemKeychain,
    plugs::{
        self, DEFAULT_TIMEOUT, DRIVERS, Observation, PlugError, PlugRemoveResult, PlugRequest,
        PlugStatus, PlugSummary, PowerChange, PowerState, ResolvedPlug,
    },
};
use serde::Serialize;
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{
    OutputFormat, ParsedOptions,
    cameras::{required, respond},
    config_error, confirm, confirm_on, mark_network_call, parse_options, read_secret_file,
    write_error_with_details,
};

/// `args` starts with the action (`add`, `list`, ...).
pub(crate) fn run_plug_command(
    format: OutputFormat,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (action, rest) = args.split_first().expect("dispatch passes an action");
    let command = format!("plug {action}");
    match action.as_str() {
        "drivers" => respond(
            &command,
            format,
            drivers(rest),
            |data: &DriverList, out| {
                for driver in &data.drivers {
                    writeln!(out, "{driver}")?;
                }
                Ok(())
            },
            out,
            err,
        ),
        "add" => respond(&command, format, add(rest), summary("Plug added"), out, err),
        "edit" => respond(
            &command,
            format,
            edit(rest),
            summary("Plug updated"),
            out,
            err,
        ),
        "link" => respond(
            &command,
            format,
            link(rest, true),
            summary("Plug linked"),
            out,
            err,
        ),
        "unlink" => respond(
            &command,
            format,
            link(rest, false),
            summary("Plug unlinked"),
            out,
            err,
        ),
        "list" => respond(
            &command,
            format,
            list(rest),
            |data: &PlugList, out| {
                if data.plugs.is_empty() {
                    return writeln!(out, "No plugs configured.");
                }
                for plug in &data.plugs {
                    writeln!(out, "{}", summary_line(plug))?;
                }
                Ok(())
            },
            out,
            err,
        ),
        "remove" => {
            let result = remove(rest, err);
            respond(
                &command,
                format,
                result,
                |removed: &PlugRemoveResult, out| {
                    writeln!(out, "Plug removed: {}", removed.name)?;
                    for warning in &removed.warnings {
                        writeln!(out, "Warning: {}", warning.message)?;
                    }
                    Ok(())
                },
                out,
                err,
            )
        }
        "status" => respond(
            &command,
            format,
            status(rest),
            |data: &PlugStatus, out| {
                let name = display_name(rest, &[]);
                writeln!(
                    out,
                    "Plug {name} is {} ({}, observed {})",
                    state_word(data.relay),
                    data.device.model,
                    data.observed_at
                )
            },
            out,
            err,
        ),
        "on" | "off" => {
            let state = if action.as_str() == "on" {
                PowerState::On
            } else {
                PowerState::Off
            };
            let name = display_name(
                rest,
                if state == PowerState::Off {
                    &["yes"]
                } else {
                    &[]
                },
            );
            let is_terminal = std::io::IsTerminal::is_terminal(&std::io::stdin());
            match power(rest, state, is_terminal, err) {
                Ok(change) => respond(
                    &command,
                    format,
                    Ok(change),
                    |data: &PowerChange, out| {
                        writeln!(
                            out,
                            "Plug {name} is {} (confirmed {})",
                            state_word(data.status.relay),
                            data.status.observed_at
                        )
                    },
                    out,
                    err,
                ),
                Err((error, details)) => {
                    write_error_with_details(&command, format, error, details, out, err)
                }
            }
        }
        _ => unreachable!("dispatch matches only known plug commands"),
    }
}

/// The plug name as stored (profile names are ASCII-lowercased), found among
/// the arguments even when flags precede it.
fn display_name(args: &[&String], flags: &[&str]) -> String {
    parse_options(args, flags, &[])
        .ok()
        .and_then(|(positionals, _)| positionals.first().map(|name| name.to_ascii_lowercase()))
        .unwrap_or_default()
}

fn state_word(state: PowerState) -> &'static str {
    match state {
        PowerState::On => "on",
        PowerState::Off => "off",
    }
}

type Failure = (AppError, BTreeMap<String, Value>);

fn status(args: &[&String]) -> Result<PlugStatus, AppError> {
    let (positionals, _) = parse_options(args, &[], &[]).map_err(AppError::usage)?;
    let name = one_plug("plug status", &positionals)?;
    let dir = config_dir().map_err(config_error)?;
    let plug = plugs::resolve(dir, &SystemKeychain, name).map_err(plug_error)?;
    runtime()?
        .block_on(plugs::status(&plug))
        .map_err(plug_error)
}

fn off_prompt(plug: &PlugSummary) -> String {
    let list = |names: &[String]| {
        if names.is_empty() {
            "none".to_owned()
        } else {
            names.join(", ")
        }
    };
    format!(
        "Turn off plug {}? Power is cut immediately to printers: {}; cameras: {}. Type 'yes' to continue: ",
        plug.name,
        list(&plug.printers),
        list(&plug.cameras)
    )
}

/// Parses, confirms (`off` only) and resolves the secret. Nothing here
/// touches the network, so a refusal leaves `durationMs` unset.
fn prepare_power(
    args: &[&String],
    state: PowerState,
    dir: &Path,
    is_terminal: bool,
    err: &mut dyn Write,
) -> Result<ResolvedPlug, AppError> {
    let command = format!("plug {}", state_word(state));
    let flags: &[&str] = if state == PowerState::Off {
        &["yes"]
    } else {
        &[]
    };
    let (positionals, options) = parse_options(args, flags, &[]).map_err(AppError::usage)?;
    let name = one_plug(&command, &positionals)?.to_ascii_lowercase();
    let name = name.as_str();
    if state == PowerState::Off {
        let summary = plugs::list(dir)
            .map_err(plug_error)?
            .into_iter()
            .find(|plug| plug.name == name)
            .ok_or_else(|| plug_error(PlugError::NotFound(name.to_owned())))?;
        if !options.enabled("yes") {
            confirm_on(
                is_terminal,
                err,
                &off_prompt(&summary),
                "confirmation declined; no power command was sent",
            )?;
        }
    }
    plugs::resolve(dir, &SystemKeychain, name).map_err(plug_error)
}

fn power(
    args: &[&String],
    state: PowerState,
    is_terminal: bool,
    err: &mut dyn Write,
) -> Result<PowerChange, Failure> {
    let early = |error| (error, BTreeMap::new());
    let dir = config_dir().map_err(config_error).map_err(early)?;
    let plug = prepare_power(args, state, &dir, is_terminal, err).map_err(early)?;
    runtime()
        .map_err(early)?
        .block_on(plugs::set_power(&plug, state))
        .map_err(|error| power_failure(&plug.name, error))
}

/// Maps a power error to its CLI error and machine-readable details.
fn power_failure(name: &str, error: PlugError) -> Failure {
    let details = |requested: PowerState, observed: Option<(PowerState, String)>, uncertain| {
        let (state, at) = observed.unzip();
        BTreeMap::from([
            ("requestedState".to_owned(), json!(state_word(requested))),
            ("lastObservedState".to_owned(), json!(state.map(state_word))),
            ("observedAt".to_owned(), json!(at)),
            ("deliveryUncertain".to_owned(), json!(uncertain)),
        ])
    };
    match &error {
        PlugError::PowerUnconfirmed {
            requested,
            last_observed,
        } => {
            let observed = last_observed
                .as_ref()
                .map(|Observation { state, observed_at }| (*state, observed_at.clone()));
            let last = observed
                .as_ref()
                .map_or("unknown", |(state, _)| state_word(*state));
            let message = format!(
                "Sent \"{}\" to plug {name} but could not confirm the result. Last known state: {last}.",
                state_word(*requested)
            );
            let details = details(*requested, observed, true);
            let mut mapped = plug_error(error);
            mapped.message = message;
            (mapped, details)
        }
        PlugError::PowerMismatch {
            requested,
            observed,
        } => {
            let message = format!(
                "Sent \"{}\" to plug {name} but it reports {}.",
                state_word(*requested),
                state_word(*observed)
            );
            let now = OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_default();
            let details = details(*requested, Some((*observed, now)), false);
            let mut mapped = plug_error(error);
            mapped.message = message;
            (mapped, details)
        }
        _ => (plug_error(error), BTreeMap::new()),
    }
}

fn summary(verb: &'static str) -> impl FnOnce(&PlugSummary, &mut dyn Write) -> std::io::Result<()> {
    move |plug, out| writeln!(out, "{verb}: {}", summary_line(plug))
}

fn summary_line(plug: &PlugSummary) -> String {
    let join = |names: &[String]| {
        if names.is_empty() {
            "-".to_owned()
        } else {
            names.join(",")
        }
    };
    format!(
        "{}  {}  printers: {}  cameras: {}",
        plug.name,
        plug.host,
        join(&plug.printers),
        join(&plug.cameras)
    )
}

#[derive(Serialize)]
struct DriverList {
    drivers: Vec<&'static str>,
}

#[derive(Serialize)]
struct PlugList {
    plugs: Vec<PlugSummary>,
}

fn drivers(args: &[&String]) -> Result<DriverList, AppError> {
    let (positionals, _) = parse_options(args, &[], &[]).map_err(AppError::usage)?;
    if !positionals.is_empty() {
        return Err(AppError::usage("plug drivers takes no arguments"));
    }
    Ok(DriverList {
        drivers: DRIVERS.to_vec(),
    })
}

fn one_plug<'a>(command: &str, positionals: &'a [String]) -> Result<&'a str, AppError> {
    match positionals {
        [name] => Ok(name),
        [] => Err(AppError::usage(format!("{command} requires a plug name"))),
        _ => Err(AppError::usage(format!(
            "{command} expects exactly one plug name"
        ))),
    }
}

/// A throwaway current-thread runtime; creating it marks the network call.
fn runtime() -> Result<tokio::runtime::Runtime, AppError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| AppError {
            exit_code: 1,
            code: "internal_error",
            message: "cannot start async runtime".into(),
        })?;
    mark_network_call();
    Ok(runtime)
}

fn verify(plug: &ResolvedPlug) -> Result<(), AppError> {
    runtime()?
        .block_on(plugs::verify(plug))
        .map(drop)
        .map_err(plug_error)
}

fn add(args: &[&String]) -> Result<PlugSummary, AppError> {
    let (positionals, options) = parse_options(
        args,
        &[],
        &["driver", "host", "username", "password-file", "timeout"],
    )
    .map_err(AppError::usage)?;
    let name = one_plug("plug add", &positionals)?;
    let driver = required(&options, "driver")?;
    if !DRIVERS.contains(&driver.as_str()) {
        return Err(AppError::usage(format!(
            "--driver must be one of: {}",
            DRIVERS.join(", ")
        )));
    }
    let request = PlugRequest {
        name: name.to_owned(),
        driver,
        host: required(&options, "host")?,
        username: required(&options, "username")?,
        password: read_secret_file("--password-file", &required(&options, "password-file")?)
            .map_err(AppError::usage)?,
        timeout: options
            .value("timeout")
            .unwrap_or(DEFAULT_TIMEOUT)
            .to_owned(),
    };
    let dir = config_dir().map_err(config_error)?;
    let plug = plugs::prepare_create(&dir, request).map_err(plug_error)?;
    verify(&plug)?;
    plugs::commit_create(&dir, &SystemKeychain, plug).map_err(plug_error)
}

fn edit(args: &[&String]) -> Result<PlugSummary, AppError> {
    let (positionals, options) = parse_options(
        args,
        &[],
        &["name", "host", "username", "password-file", "timeout"],
    )
    .map_err(AppError::usage)?;
    let current = one_plug("plug edit", &positionals)?.to_ascii_lowercase();
    let current = current.as_str();
    if options.values.is_empty() {
        return Err(AppError::usage(
            "plug edit requires at least one option to change",
        ));
    }
    // The core reads an empty password as "keep the stored one".
    let password = match options.value("password-file") {
        Some(path) => {
            let password = read_secret_file("--password-file", path).map_err(AppError::usage)?;
            if password.is_empty() {
                return Err(AppError::usage("--password-file must not be empty"));
            }
            password
        }
        None => String::new(),
    };
    let dir = config_dir().map_err(config_error)?;
    let existing = plugs::list(&dir)
        .map_err(plug_error)?
        .into_iter()
        .find(|plug| plug.name == current)
        .ok_or_else(|| plug_error(PlugError::NotFound(current.to_owned())))?;
    let request = PlugRequest {
        name: options.value("name").unwrap_or(current).to_owned(),
        driver: existing.driver,
        host: options.value("host").map_or(existing.host, str::to_owned),
        username: options
            .value("username")
            .map_or(existing.username, str::to_owned),
        password,
        timeout: options
            .value("timeout")
            .map_or(existing.timeout, str::to_owned),
    };
    let update =
        plugs::prepare_update(&dir, &SystemKeychain, current, request).map_err(plug_error)?;
    if update.needs_verify {
        verify(&update.plug)?;
    }
    plugs::commit_update(&dir, &SystemKeychain, update).map_err(plug_error)
}

/// Exactly one of `--printer` / `--camera`.
fn target(command: &str, options: &ParsedOptions) -> Result<PlugTarget, AppError> {
    match (options.value("printer"), options.value("camera")) {
        (Some(printer), None) => Ok(PlugTarget::Printer(printer.to_owned())),
        (None, Some(camera)) => Ok(PlugTarget::Camera(camera.to_owned())),
        _ => Err(AppError::usage(format!(
            "{command} requires exactly one of --printer or --camera"
        ))),
    }
}

fn link(args: &[&String], attach: bool) -> Result<PlugSummary, AppError> {
    let command = if attach { "plug link" } else { "plug unlink" };
    let (positionals, options) =
        parse_options(args, &[], &["printer", "camera"]).map_err(AppError::usage)?;
    let name = one_plug(command, &positionals)?;
    let target = target(command, &options)?;
    let dir = config_dir().map_err(config_error)?;
    if attach {
        plugs::link(dir, name, &target)
    } else {
        plugs::unlink(dir, name, &target)
    }
    .map_err(plug_error)
}

fn list(args: &[&String]) -> Result<PlugList, AppError> {
    let (positionals, options) =
        parse_options(args, &[], &["printer", "camera"]).map_err(AppError::usage)?;
    if !positionals.is_empty() {
        return Err(AppError::usage("plug list takes no arguments"));
    }
    let filter = match (options.value("printer"), options.value("camera")) {
        (Some(_), Some(_)) => {
            return Err(AppError::usage(
                "use either --printer or --camera, not both",
            ));
        }
        (Some(printer), None) => Some(PlugTarget::Printer(printer.to_owned())),
        (None, Some(camera)) => Some(PlugTarget::Camera(camera.to_owned())),
        (None, None) => None,
    };
    let dir = config_dir().map_err(config_error)?;
    let plugs = match filter {
        Some(target) => plugs::list_for(dir, &target),
        None => plugs::list(dir),
    }
    .map_err(plug_error)?;
    Ok(PlugList { plugs })
}

fn remove(args: &[&String], err: &mut dyn Write) -> Result<PlugRemoveResult, AppError> {
    let (positionals, options) = parse_options(args, &["yes"], &[]).map_err(AppError::usage)?;
    let name = one_plug("plug remove", &positionals)?;
    if !options.enabled("yes") {
        confirm(
            err,
            &format!(
                "Remove plug {name}, its attachments, and its stored password? Type 'yes' to continue: "
            ),
            "confirmation declined; plug not removed",
        )?;
    }
    let dir = config_dir().map_err(config_error)?;
    plugs::remove(dir, &SystemKeychain, name).map_err(plug_error)
}

pub(crate) fn plug_error(error: PlugError) -> AppError {
    let message = error.to_string();
    let (exit_code, code) = match error {
        PlugError::Config(error) => return config_error(error),
        PlugError::Authentication | PlugError::CompatibilityDisabled => {
            (3, "authentication_failed")
        }
        PlugError::Lockout => (3, "authentication_locked"),
        PlugError::MissingSecret => (3, "secret_not_found"),
        PlugError::Secret(_) => (3, "secret_store_failed"),
        PlugError::Timeout => (4, "timeout"),
        PlugError::Connect | PlugError::MalformedResponse => (4, "connection_failed"),
        PlugError::DeviceRejected => (4, "device_rejected"),
        PlugError::PowerUnconfirmed { .. } => (4, "power_change_unconfirmed"),
        PlugError::PowerMismatch { .. } => (4, "power_change_mismatch"),
        PlugError::UnsupportedDevice(_) | PlugError::UnsupportedProtocol => {
            (5, "capability_unsupported")
        }
        PlugError::RollbackFailed => (1, "internal_error"),
        _ => return AppError::usage(message),
    };
    AppError {
        exit_code,
        code,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_error_with_details;
    use melt_core::{
        keychain::SecretError,
        plugs::{Observation, PowerState},
    };
    use serde_json::Value;
    use std::collections::BTreeMap;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn plug_errors_follow_the_exit_contract() {
        let cases = [
            (PlugError::MissingName, 2, "config_error"),
            (PlugError::InvalidName, 2, "config_error"),
            (PlugError::UnknownDriver("x".into()), 2, "config_error"),
            (PlugError::InvalidHost, 2, "config_error"),
            (PlugError::InvalidUsername, 2, "config_error"),
            (PlugError::InvalidPassword, 2, "config_error"),
            (PlugError::InvalidTimeout, 2, "config_error"),
            (PlugError::NotFound("a".into()), 2, "config_error"),
            (PlugError::AlreadyExists("a".into()), 2, "config_error"),
            (PlugError::PrinterNotFound("a".into()), 2, "config_error"),
            (PlugError::CameraNotFound("a".into()), 2, "config_error"),
            (
                PlugError::NotLinked {
                    plug: "a".into(),
                    target: "b".into(),
                },
                2,
                "config_error",
            ),
            (PlugError::ProfileChanged, 2, "config_error"),
            (PlugError::Authentication, 3, "authentication_failed"),
            (PlugError::CompatibilityDisabled, 3, "authentication_failed"),
            (PlugError::Lockout, 3, "authentication_locked"),
            (PlugError::MissingSecret, 3, "secret_not_found"),
            (
                PlugError::Secret(SecretError::Unavailable("x".into())),
                3,
                "secret_store_failed",
            ),
            (PlugError::Timeout, 4, "timeout"),
            (PlugError::Connect, 4, "connection_failed"),
            (PlugError::MalformedResponse, 4, "connection_failed"),
            (PlugError::DeviceRejected, 4, "device_rejected"),
            (
                PlugError::PowerUnconfirmed {
                    requested: PowerState::Off,
                    last_observed: None,
                },
                4,
                "power_change_unconfirmed",
            ),
            (
                PlugError::PowerMismatch {
                    requested: PowerState::Off,
                    observed: PowerState::On,
                },
                4,
                "power_change_mismatch",
            ),
            (
                PlugError::UnsupportedDevice("P100".into()),
                5,
                "capability_unsupported",
            ),
            (PlugError::UnsupportedProtocol, 5, "capability_unsupported"),
            (PlugError::RollbackFailed, 1, "internal_error"),
        ];
        for (error, exit_code, code) in cases {
            let name = format!("{error:?}");
            let mapped = plug_error(error);
            assert_eq!((mapped.exit_code, mapped.code), (exit_code, code), "{name}");
        }
    }

    #[test]
    fn link_requires_exactly_one_target() {
        for (args, ok) in [
            (&["--printer", "p"][..], true),
            (&["--camera", "c"][..], true),
            (&[][..], false),
            (&["--printer", "p", "--camera", "c"][..], false),
        ] {
            let args = strings(args);
            let refs: Vec<&String> = args.iter().collect();
            let (_, options) = parse_options(&refs, &[], &["printer", "camera"]).unwrap();
            let got = target("plug link", &options);
            assert_eq!(got.is_ok(), ok, "{args:?}");
            if let Err(error) = got {
                assert_eq!(error.exit_code, 2);
            }
        }
    }

    #[test]
    fn list_rejects_both_filters() {
        let args = strings(&["--printer", "p", "--camera", "c"]);
        let refs: Vec<&String> = args.iter().collect();
        let Err(error) = list(&refs) else {
            panic!("both filters rejected")
        };
        assert_eq!(error.exit_code, 2);
    }

    #[test]
    fn add_requires_password_file() {
        let args = strings(&[
            "bench",
            "--driver",
            "tapo",
            "--host",
            "h",
            "--username",
            "u",
        ]);
        let refs: Vec<&String> = args.iter().collect();
        let Err(error) = add(&refs) else {
            panic!("password file required")
        };
        assert_eq!(error.exit_code, 2);
        assert_eq!(error.message, "--password-file is required");
    }

    #[test]
    fn remove_without_yes_in_noninteractive_mode_fails() {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            return; // would prompt
        }
        let args = strings(&["bench"]);
        let refs: Vec<&String> = args.iter().collect();
        let Err(error) = remove(&refs, &mut Vec::new()) else {
            panic!("needs --yes")
        };
        assert_eq!(error.exit_code, 2);
        assert_eq!(error.message, "non-interactive mode requires --yes");
    }

    fn plug_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("melt.yaml"),
            "version: 3\nplugs:\n  bench:\n    id: '11111111111111111111111111111111'\n    driver: tapo\n    host: 192.0.2.30\n    username: operator@example.invalid\n    timeout: 10s\n    created: '2026-10-18T12:00:00Z'\n    updated: '2026-10-18T12:00:00Z'\n",
        )
        .unwrap();
        dir
    }

    fn envelope(error: AppError, details: BTreeMap<String, Value>) -> Value {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        write_error_with_details(
            "plug on",
            OutputFormat::Json,
            error,
            details,
            &mut out,
            &mut err,
        );
        serde_json::from_slice(&out).unwrap()
    }

    #[test]
    fn off_without_tty_or_yes_sends_nothing() {
        let dir = plug_dir();
        let args = strings(&["bench"]);
        let refs: Vec<&String> = args.iter().collect();
        let mut err = Vec::new();
        let Err(error) = prepare_power(&refs, PowerState::Off, dir.path(), false, &mut err) else {
            panic!("needs --yes")
        };
        assert_eq!(error.exit_code, 2);
        assert_eq!(error.code, "config_error");
        assert_eq!(error.message, "non-interactive mode requires --yes");
        assert!(err.is_empty(), "no prompt without a terminal");
        assert!(crate::network_duration_ms().is_none(), "no network attempt");
        let json = envelope(error, BTreeMap::new());
        assert!(json["meta"].get("durationMs").is_none());
    }

    #[test]
    fn unknown_plug_is_a_config_error_before_any_prompt() {
        let dir = plug_dir();
        let args = strings(&["ghost"]);
        let refs: Vec<&String> = args.iter().collect();
        let mut err = Vec::new();
        let Err(error) = prepare_power(&refs, PowerState::Off, dir.path(), true, &mut err) else {
            panic!("unknown plug")
        };
        assert_eq!((error.exit_code, error.code), (2, "config_error"));
        assert!(err.is_empty());
    }

    #[test]
    fn off_prompt_names_attachments() {
        let mut plug = summary_for_test();
        assert_eq!(
            off_prompt(&plug),
            "Turn off plug bench? Power is cut immediately to printers: none; cameras: none. Type 'yes' to continue: "
        );
        plug.printers = vec!["a".into(), "b".into()];
        plug.cameras = vec!["porch".into()];
        assert_eq!(
            off_prompt(&plug),
            "Turn off plug bench? Power is cut immediately to printers: a, b; cameras: porch. Type 'yes' to continue: "
        );
    }

    #[test]
    fn plug_names_are_accepted_in_any_case() {
        let dir = plug_dir();
        let args = strings(&["--yes", "Bench"]);
        let refs: Vec<&String> = args.iter().collect();
        // Passes the name lookup; stops at the missing keychain secret, not NotFound.
        let result = prepare_power(&refs, PowerState::Off, dir.path(), false, &mut Vec::new());
        if let Err(error) = result {
            assert!(!error.message.ends_with("not found"), "{}", error.message);
        }
    }

    #[test]
    fn display_name_skips_flags_and_lowercases() {
        let args = strings(&["--yes", "Bench"]);
        let refs: Vec<&String> = args.iter().collect();
        assert_eq!(display_name(&refs, &["yes"]), "bench");
    }

    fn summary_for_test() -> PlugSummary {
        let dir = plug_dir();
        plugs::list(dir.path()).unwrap().remove(0)
    }

    #[test]
    fn unconfirmed_error_carries_details() {
        let (error, details) = power_failure(
            "bench",
            PlugError::PowerUnconfirmed {
                requested: PowerState::On,
                last_observed: Some(Observation {
                    state: PowerState::Off,
                    observed_at: "2026-10-18T12:00:00Z".into(),
                }),
            },
        );
        assert_eq!(
            error.message,
            "Sent \"on\" to plug bench but could not confirm the result. Last known state: off."
        );
        assert_eq!(
            envelope(error, details)["error"],
            serde_json::json!({
                "code": "power_change_unconfirmed",
                "message": "Sent \"on\" to plug bench but could not confirm the result. Last known state: off.",
                "details": {
                    "requestedState": "on",
                    "lastObservedState": "off",
                    "observedAt": "2026-10-18T12:00:00Z",
                    "deliveryUncertain": true
                }
            })
        );
        let (error, details) = power_failure(
            "bench",
            PlugError::PowerUnconfirmed {
                requested: PowerState::Off,
                last_observed: None,
            },
        );
        assert!(error.message.ends_with("Last known state: unknown."));
        let details = envelope(error, details)["error"]["details"].clone();
        assert_eq!(
            details,
            serde_json::json!({
                "requestedState": "off",
                "lastObservedState": null,
                "observedAt": null,
                "deliveryUncertain": true
            })
        );
    }

    #[test]
    fn mismatch_error_carries_details() {
        let (error, details) = power_failure(
            "bench",
            PlugError::PowerMismatch {
                requested: PowerState::On,
                observed: PowerState::Off,
            },
        );
        assert_eq!(
            error.message,
            "Sent \"on\" to plug bench but it reports off."
        );
        let details = envelope(error, details)["error"]["details"].clone();
        assert_eq!(details["requestedState"], "on");
        assert_eq!(details["lastObservedState"], "off");
        assert_eq!(details["deliveryUncertain"], false);
        assert!(details["observedAt"].is_string());
    }

    #[test]
    fn existing_error_details_stay_empty() {
        let (error, details) = power_failure("bench", PlugError::Timeout);
        assert_eq!(error.code, "timeout");
        assert_eq!(
            envelope(error, details)["error"]["details"],
            serde_json::json!({})
        );
    }
}
