use std::{collections::HashSet, fs, path::Path};

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
    exit_code: i32,
    stdout: String,
    stderr: String,
    ignore_json_pointers: Vec<String>,
}

#[test]
fn contract_fixtures_reference_public_commands_and_valid_envelopes() {
    let inventory: Value = read_json("inventory.json");
    assert_eq!(inventory["schemaVersion"], 1);
    let commands = inventory["commands"].as_array().expect("commands array");
    assert_eq!(commands.len(), 36);
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
    assert_eq!(manifest.fixtures.len(), 5);
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

        let output = read_json(&fixture.stdout);
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
        for pointer in fixture.ignore_json_pointers {
            assert!(
                output.pointer(&pointer).is_some(),
                "{pointer} does not exist in {}",
                fixture.stdout
            );
        }
    }
}

fn read_json(relative_path: &str) -> Value {
    let path = Path::new(FIXTURE_ROOT).join(relative_path);
    serde_json::from_slice(&fs::read(path).expect("fixture file")).expect("valid JSON fixture")
}
