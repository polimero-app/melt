use std::{cmp::Ordering, collections::BTreeMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareVersion {
    pub raw: String,
    pub numeric: Vec<u64>,
    pub suffix: Option<String>,
}

impl FirmwareVersion {
    pub fn parse(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let trimmed = raw.trim();
        let split = trimmed.find(|character: char| character != '.' && !character.is_ascii_digit());
        let (numbers, suffix) = split.map_or((trimmed, None), |index| {
            (&trimmed[..index], Some(trimmed[index..].to_owned()))
        });
        let numeric = numbers
            .split('.')
            .filter(|part| !part.is_empty())
            .map(str::parse)
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default();
        Self {
            raw,
            numeric,
            suffix,
        }
    }

    pub fn numeric_cmp(&self, other: &Self) -> Ordering {
        let length = self.numeric.len().max(other.numeric.len());
        (0..length)
            .map(|index| {
                self.numeric
                    .get(index)
                    .copied()
                    .unwrap_or(0)
                    .cmp(&other.numeric.get(index).copied().unwrap_or(0))
            })
            .find(|ordering| *ordering != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareModule {
    pub name: String,
    pub software: FirmwareVersion,
    pub hardware: Option<String>,
    pub serial: Option<String>,
    pub project: Option<String>,
    pub unknown: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareInventory {
    pub modules: Vec<FirmwareModule>,
}

impl FirmwareInventory {
    pub fn from_version_info(info: &Value) -> Self {
        let modules = info
            .get("module")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .filter_map(|module| {
                let name = module.get("name")?.as_str()?.to_owned();
                let software = FirmwareVersion::parse(
                    module
                        .get("sw_ver")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
                let known = ["name", "sw_ver", "hw_ver", "sn", "project_name"];
                let unknown = module
                    .iter()
                    .filter(|(key, _)| !known.contains(&key.as_str()))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                Some(FirmwareModule {
                    name,
                    software,
                    hardware: module
                        .get("hw_ver")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    serial: module.get("sn").and_then(Value::as_str).map(str::to_owned),
                    project: module
                        .get("project_name")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    unknown,
                })
            })
            .collect();
        Self { modules }
    }

    pub fn software(&self, name: &str) -> Option<&FirmwareVersion> {
        self.modules
            .iter()
            .find(|module| module.name == name)
            .map(|module| &module.software)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_all_modules_and_unknown_fields() {
        let inventory = FirmwareInventory::from_version_info(&json!({"module": [
            {"name":"ota","sw_ver":"01.08.00.00-beta1","hw_ver":"AP05","future":7},
            {"name":"n3f/0","sw_ver":"00.00.06.40"}
        ]}));
        assert_eq!(inventory.modules.len(), 2);
        assert_eq!(inventory.software("ota").unwrap().numeric, [1, 8, 0, 0]);
        assert_eq!(
            inventory.software("ota").unwrap().suffix.as_deref(),
            Some("-beta1")
        );
        assert_eq!(inventory.modules[0].unknown["future"], 7);
    }
}
