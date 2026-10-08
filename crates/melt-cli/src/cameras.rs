//! `melt camera` profile, link, and external-camera media commands. In the
//! commands added here a positional argument is always a camera name and a
//! printer is always `--printer`.

use std::io::{IsTerminal, Write};

use melt_core::{
    AppError,
    cameras::{self, CameraError, CameraRequest, CameraSummary},
    config::config_dir,
    keychain::SystemKeychain,
};
use serde::Serialize;

use super::{
    OutputFormat, ParsedOptions, config_error, parse_options, read_secret_file, write_error,
    write_success,
};

pub(crate) fn run_profile_command(
    format: OutputFormat,
    action: &str,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let command = format!("camera {action}");
    match action {
        "add" => respond(
            &command,
            format,
            add(args),
            human_summary("Camera added"),
            out,
            err,
        ),
        "edit" => respond(
            &command,
            format,
            edit(args),
            human_summary("Camera updated"),
            out,
            err,
        ),
        "link" => respond(
            &command,
            format,
            link(args),
            human_summary("Camera linked"),
            out,
            err,
        ),
        "unlink" => respond(
            &command,
            format,
            unlink(args),
            human_summary("Camera unlinked"),
            out,
            err,
        ),
        "list" => respond(
            &command,
            format,
            list(args),
            |data: &CameraList, out| {
                if data.cameras.is_empty() {
                    return writeln!(out, "No cameras configured.");
                }
                for camera in &data.cameras {
                    writeln!(out, "{}", summary_line(camera))?;
                }
                Ok(())
            },
            out,
            err,
        ),
        "remove" => {
            let result = remove(args, err);
            respond(
                &command,
                format,
                result,
                |removed: &cameras::CameraRemoveResult, out| {
                    writeln!(out, "Camera removed: {}", removed.name)?;
                    for warning in &removed.warnings {
                        writeln!(out, "Warning: {}", warning.message)?;
                    }
                    Ok(())
                },
                out,
                err,
            )
        }
        "select" => respond(
            &command,
            format,
            select(args),
            |selected: &cameras::SelectedFeed, out| match &selected.camera {
                Some(camera) => writeln!(
                    out,
                    "Printer {} now shows camera {camera}",
                    selected.printer
                ),
                None => writeln!(
                    out,
                    "Printer {} now shows its built-in camera",
                    selected.printer
                ),
            },
            out,
            err,
        ),
        _ => unreachable!("dispatch matches only known camera profile commands"),
    }
}

/// Renders human text eagerly so the JSON and human paths share one result.
fn respond<T: Serialize>(
    command: &str,
    format: OutputFormat,
    result: Result<T, AppError>,
    human: impl FnOnce(&T, &mut dyn Write) -> std::io::Result<()>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    match result {
        Ok(data) => {
            let mut text = Vec::new();
            if matches!(format, OutputFormat::Human) && human(&data, &mut text).is_err() {
                return 1;
            }
            write_success(command, format, data, |out| out.write_all(&text), out)
        }
        Err(error) => write_error(command, format, error, out, err),
    }
}

fn human_summary(
    verb: &'static str,
) -> impl FnOnce(&CameraSummary, &mut dyn Write) -> std::io::Result<()> {
    move |camera, out| writeln!(out, "{verb}: {}", summary_line(camera))
}

fn summary_line(camera: &CameraSummary) -> String {
    let printers = if camera.printers.is_empty() {
        "-".to_owned()
    } else {
        camera
            .printers
            .iter()
            .map(|printer| {
                if camera.selected_by.contains(printer) {
                    format!("{printer} (selected)")
                } else {
                    printer.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{}\t{}\t{}:{}\tprinters: {printers}",
        camera.name, camera.driver, camera.host, camera.rtsp_port
    )
}

#[derive(Serialize)]
struct CameraList {
    cameras: Vec<CameraSummary>,
}

fn add(args: &[&String]) -> Result<CameraSummary, AppError> {
    let (positionals, options) = parse_options(
        args,
        &[],
        &[
            "driver",
            "host",
            "username",
            "password-file",
            "rtsp-port",
            "timeout",
        ],
    )
    .map_err(AppError::usage)?;
    let name = one_camera("camera add", &positionals)?;
    let request = CameraRequest {
        name: name.to_owned(),
        driver: required(&options, "driver")?,
        host: required(&options, "host")?,
        username: required(&options, "username")?,
        password: read_secret_file("--password-file", &required(&options, "password-file")?)
            .map_err(AppError::usage)?,
        rtsp_port: rtsp_port(&options)?,
        timeout: options.value("timeout").unwrap_or("10s").to_owned(),
    };
    let dir = config_dir().map_err(config_error)?;
    cameras::create(dir, &SystemKeychain, request, cameras::verify).map_err(camera_error)
}

fn edit(args: &[&String]) -> Result<CameraSummary, AppError> {
    let (positionals, options) = parse_options(
        args,
        &[],
        &[
            "name",
            "host",
            "username",
            "password-file",
            "rtsp-port",
            "timeout",
        ],
    )
    .map_err(AppError::usage)?;
    let current = one_camera("camera edit", &positionals)?;
    if options.values.is_empty() {
        return Err(AppError::usage(
            "camera edit requires at least one option to change",
        ));
    }
    let dir = config_dir().map_err(config_error)?;
    let existing = cameras::profile(&dir, current).map_err(camera_error)?;
    let request = CameraRequest {
        name: options.value("name").unwrap_or(current).to_owned(),
        driver: existing.driver,
        host: options.value("host").map_or(existing.host, str::to_owned),
        username: options
            .value("username")
            .map_or(existing.username, str::to_owned),
        password: match options.value("password-file") {
            Some(path) => read_secret_file("--password-file", path).map_err(AppError::usage)?,
            None => String::new(),
        },
        rtsp_port: match options.value("rtsp-port") {
            Some(_) => rtsp_port(&options)?,
            None => existing.rtsp_port,
        },
        timeout: options
            .value("timeout")
            .map_or(existing.timeout, str::to_owned),
    };
    cameras::update(dir, &SystemKeychain, current, request, cameras::verify).map_err(camera_error)
}

fn list(args: &[&String]) -> Result<CameraList, AppError> {
    let (positionals, _) = parse_options(args, &[], &[]).map_err(AppError::usage)?;
    if !positionals.is_empty() {
        return Err(AppError::usage("camera list takes no arguments"));
    }
    let dir = config_dir().map_err(config_error)?;
    Ok(CameraList {
        cameras: cameras::list(dir).map_err(camera_error)?,
    })
}

fn remove(args: &[&String], err: &mut dyn Write) -> Result<cameras::CameraRemoveResult, AppError> {
    let (positionals, options) = parse_options(args, &["yes"], &[]).map_err(AppError::usage)?;
    let name = one_camera("camera remove", &positionals)?;
    if !options.enabled("yes") {
        if !std::io::stdin().is_terminal() {
            return Err(AppError::usage("non-interactive mode requires --yes"));
        }
        let _ = write!(
            err,
            "Remove camera {name}, its printer links, and its stored password? Type 'yes' to continue: "
        );
        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .map_err(|_| AppError {
                exit_code: 1,
                code: "internal_error",
                message: "cannot read confirmation".into(),
            })?;
        if answer.trim_end() != "yes" {
            return Err(AppError::usage("confirmation declined; camera not removed"));
        }
    }
    let dir = config_dir().map_err(config_error)?;
    cameras::remove(dir, &SystemKeychain, name).map_err(camera_error)
}

fn link(args: &[&String]) -> Result<CameraSummary, AppError> {
    let (positionals, options) =
        parse_options(args, &["select"], &["printer"]).map_err(AppError::usage)?;
    let camera = one_camera("camera link", &positionals)?;
    let printer = required(&options, "printer")?;
    let dir = config_dir().map_err(config_error)?;
    cameras::link(dir, &printer, camera, options.enabled("select")).map_err(camera_error)
}

fn unlink(args: &[&String]) -> Result<CameraSummary, AppError> {
    let (positionals, options) = parse_options(args, &[], &["printer"]).map_err(AppError::usage)?;
    let camera = one_camera("camera unlink", &positionals)?;
    let printer = required(&options, "printer")?;
    let dir = config_dir().map_err(config_error)?;
    cameras::unlink(dir, &printer, camera).map_err(camera_error)
}

fn select(args: &[&String]) -> Result<cameras::SelectedFeed, AppError> {
    let (positionals, options) =
        parse_options(args, &["builtin"], &["printer", "camera"]).map_err(AppError::usage)?;
    if !positionals.is_empty() {
        return Err(AppError::usage(
            "camera select takes --printer and a source, not names",
        ));
    }
    let camera = match (options.value("camera"), options.enabled("builtin")) {
        (Some(camera), false) => Some(camera),
        (None, true) => None,
        _ => {
            return Err(AppError::usage(
                "camera select requires exactly one of --camera or --builtin",
            ));
        }
    };
    let printer = required(&options, "printer")?;
    let dir = config_dir().map_err(config_error)?;
    cameras::select(dir, &printer, camera).map_err(camera_error)
}

fn one_camera<'a>(command: &str, positionals: &'a [String]) -> Result<&'a str, AppError> {
    match positionals {
        [name] => Ok(name),
        [] => Err(AppError::usage(format!("{command} requires a camera name"))),
        _ => Err(AppError::usage(format!(
            "{command} expects exactly one camera name"
        ))),
    }
}

fn required(options: &ParsedOptions, name: &str) -> Result<String, AppError> {
    options
        .value(name)
        .map(str::to_owned)
        .ok_or_else(|| AppError::usage(format!("--{name} is required")))
}

fn rtsp_port(options: &ParsedOptions) -> Result<Option<u16>, AppError> {
    options
        .value("rtsp-port")
        .map(|value| {
            value
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| AppError::usage("--rtsp-port must be between 1 and 65535"))
        })
        .transpose()
}

pub(crate) fn camera_error(error: CameraError) -> AppError {
    let message = error.to_string();
    let (exit_code, code) = match error {
        CameraError::Config(error) => return config_error(error),
        CameraError::Authentication | CameraError::DigestRequired => (3, "authentication_failed"),
        CameraError::MissingSecret => (3, "secret_not_found"),
        CameraError::Secret(_) => (3, "secret_store_failed"),
        CameraError::Timeout => (4, "timeout"),
        CameraError::Connect(_) | CameraError::Stream(_) => (4, "connection_failed"),
        CameraError::UnsupportedCodec => (5, "capability_unsupported"),
        CameraError::RollbackFailed => (1, "internal_error"),
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

    #[test]
    fn camera_errors_follow_the_exit_contract() {
        let cases = [
            (CameraError::InvalidName, 2, "config_error"),
            (CameraError::NotFound("porch".into()), 2, "config_error"),
            (CameraError::Authentication, 3, "authentication_failed"),
            (CameraError::DigestRequired, 3, "authentication_failed"),
            (CameraError::MissingSecret, 3, "secret_not_found"),
            (CameraError::Timeout, 4, "timeout"),
            (
                CameraError::Stream("RTSP PLAY failed".into()),
                4,
                "connection_failed",
            ),
            (CameraError::UnsupportedCodec, 5, "capability_unsupported"),
            (CameraError::RollbackFailed, 1, "internal_error"),
        ];
        for (error, exit_code, code) in cases {
            let name = format!("{error:?}");
            let mapped = camera_error(error);
            assert_eq!((mapped.exit_code, mapped.code), (exit_code, code), "{name}");
        }
    }
}
