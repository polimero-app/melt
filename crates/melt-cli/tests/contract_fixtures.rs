use std::{collections::HashSet, env, ffi::OsString, fs, path::Path, sync::Mutex};

use serde::Deserialize;
use serde_json::Value;

const FIXTURE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/cli-contract");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u8,
    fixtures: Vec<Fixture>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    id: String,
    command: String,
    args: Vec<String>,
    exit_code: i32,
    stdout: String,
    stderr: String,
    ignore_json_pointers: Vec<String>,
}

static CONFIG_ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn contract_fixtures_match_the_cli_envelope_and_exit_contract() {
    let _lock = CONFIG_ENV_LOCK.lock().unwrap();
    let _config_dir = ConfigDirGuard::set(Path::new(FIXTURE_ROOT).join("config/empty"));
    let inventory: Value = read_json("inventory.json");
    assert_eq!(inventory["schemaVersion"], 1);
    let commands = inventory["commands"].as_array().expect("commands array");
    assert_eq!(commands.len(), 37);
    assert!(
        commands
            .iter()
            .all(|command| command["implementation"] == "implemented"),
        "every public command path must be implemented"
    );
    let command_paths: HashSet<_> = commands
        .iter()
        .map(|command| command["path"].as_str().expect("command path"))
        .collect();
    assert_eq!(
        command_paths.len(),
        commands.len(),
        "duplicate command path"
    );

    let manifest: Manifest = serde_json::from_value(read_json("manifest.json")).expect("manifest");
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.fixtures.len(), 8);
    let mut fixture_ids = HashSet::new();

    for fixture in manifest.fixtures {
        assert!(
            fixture_ids.insert(fixture.id.clone()),
            "duplicate fixture ID"
        );
        assert!(
            command_paths.contains(fixture.command.as_str()),
            "{} is not in the public command inventory",
            fixture.command
        );
        assert!((0..=5).contains(&fixture.exit_code));
        assert!(
            fixture.stderr.is_empty(),
            "JSON fixtures must not use stderr"
        );
        assert!(
            !fixture.stdout.contains(".."),
            "fixture paths must stay within the fixture root"
        );

        let mut output = read_json(&fixture.stdout);
        let ok = output["ok"].as_bool().expect("envelope ok");
        assert_eq!(output["meta"]["command"], fixture.command);
        if ok {
            assert!(output["data"].is_object() || output["data"].is_array());
            assert!(output["error"].is_null());
        } else {
            assert!(output["data"].is_null());
            assert!(output["error"]["code"].is_string());
            assert!(output["error"]["message"].is_string());
            assert!(output["error"]["details"].is_object());
        }
        for pointer in &fixture.ignore_json_pointers {
            assert!(
                output.pointer(pointer).is_some(),
                "{pointer} does not exist in {}",
                fixture.stdout
            );
            remove_pointer(&mut output, pointer);
        }

        let mut actual_stdout = Vec::new();
        let mut actual_stderr = Vec::new();
        let actual_exit = melt_cli::run(&fixture.args, &mut actual_stdout, &mut actual_stderr);
        assert_eq!(actual_exit, fixture.exit_code, "{} exit code", fixture.id);
        assert_eq!(
            actual_stderr,
            fixture.stderr.as_bytes(),
            "{} stderr",
            fixture.id
        );
        let mut actual: Value =
            serde_json::from_slice(&actual_stdout).expect("CLI must emit JSON for JSON fixture");
        for pointer in &fixture.ignore_json_pointers {
            remove_pointer(&mut actual, pointer);
        }
        assert_eq!(actual, output, "{} stdout", fixture.id);
    }
}

struct ConfigDirGuard(Option<OsString>);

impl ConfigDirGuard {
    fn set(path: impl AsRef<Path>) -> Self {
        let previous = env::var_os("MELT_CONFIG_DIR");
        // The integration fixture is serialized above, so the process-wide
        // config location cannot leak between fixture executions.
        unsafe { env::set_var("MELT_CONFIG_DIR", path.as_ref()) };
        Self(previous)
    }
}

impl Drop for ConfigDirGuard {
    fn drop(&mut self) {
        match self.0.take() {
            Some(value) => unsafe { env::set_var("MELT_CONFIG_DIR", value) },
            None => unsafe { env::remove_var("MELT_CONFIG_DIR") },
        }
    }
}

fn remove_pointer(value: &mut Value, pointer: &str) {
    let mut segments = pointer
        .strip_prefix('/')
        .expect("JSON pointer starts with a slash")
        .split('/')
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
        .collect::<Vec<_>>();
    let last = segments.pop().expect("JSON pointer has a member");
    let mut parent = value;
    for segment in segments {
        parent = parent
            .get_mut(&segment)
            .expect("fixture pointer parent exists");
    }
    match parent {
        Value::Object(values) => {
            values.remove(&last).expect("fixture pointer member exists");
        }
        Value::Array(values) => {
            values.remove(last.parse::<usize>().expect("array pointer index"));
        }
        _ => panic!("fixture pointer parent is not a container"),
    }
}

fn read_json(relative_path: &str) -> Value {
    let path = Path::new(FIXTURE_ROOT).join(relative_path);
    serde_json::from_slice(&fs::read(path).expect("fixture file")).expect("valid JSON fixture")
}
