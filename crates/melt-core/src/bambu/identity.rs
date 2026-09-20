use serde::{Deserialize, Serialize};

use super::{FirmwareInventory, FirmwareModule, ModelFamily};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CanonicalModel {
    A1,
    A1Mini,
    A2L,
    P1P,
    P1S,
    P2S,
    X1,
    X1Carbon,
    X1E,
    X2D,
    H2D,
    H2DPro,
    H2S,
    H2C,
    #[default]
    Unknown,
}

/// One physical model, keyed by every spelling a Bambu printer can present it
/// under on the LAN. `code` is Bambu's own `model_id`, broadcast in
/// `DevModel.bambu.com`; `product_name` is the marketing name reported by
/// `info.get_version`; `sn_prefix` is the leading three characters of the
/// physical serial.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelEntry {
    pub canonical: CanonicalModel,
    pub code: &'static str,
    /// Legacy `DevModel` spellings for the same machine.
    pub aliases: &'static [&'static str],
    pub product_name: &'static str,
    pub sn_prefix: &'static str,
}

/// Bambu's own `resources/printers/*.json` values. Community tables invert
/// `00M`/`00W`, `030`/`039`, and `C11`/`C12`; the vendor files win those ties.
const MODELS: &[ModelEntry] = &[
    ModelEntry {
        canonical: CanonicalModel::X1Carbon,
        code: "BL-P001",
        aliases: &["3DPrinter-X1-Carbon"],
        product_name: "Bambu Lab X1 Carbon",
        sn_prefix: "00M",
    },
    ModelEntry {
        canonical: CanonicalModel::X1,
        code: "BL-P002",
        aliases: &["3DPrinter-X1"],
        product_name: "Bambu Lab X1",
        sn_prefix: "00W",
    },
    ModelEntry {
        canonical: CanonicalModel::P1P,
        code: "C11",
        aliases: &[],
        product_name: "Bambu Lab P1P",
        sn_prefix: "01S",
    },
    ModelEntry {
        canonical: CanonicalModel::P1S,
        code: "C12",
        aliases: &[],
        product_name: "Bambu Lab P1S",
        sn_prefix: "01P",
    },
    ModelEntry {
        canonical: CanonicalModel::X1E,
        code: "C13",
        aliases: &[],
        product_name: "Bambu Lab X1E",
        sn_prefix: "03W",
    },
    ModelEntry {
        canonical: CanonicalModel::A1Mini,
        code: "N1",
        aliases: &[],
        product_name: "Bambu Lab A1 mini",
        sn_prefix: "030",
    },
    ModelEntry {
        canonical: CanonicalModel::A1,
        code: "N2S",
        aliases: &[],
        product_name: "Bambu Lab A1",
        sn_prefix: "039",
    },
    ModelEntry {
        canonical: CanonicalModel::X2D,
        code: "N6",
        aliases: &[],
        product_name: "Bambu Lab X2D",
        sn_prefix: "20P",
    },
    ModelEntry {
        canonical: CanonicalModel::P2S,
        code: "N7",
        aliases: &[],
        product_name: "Bambu Lab P2S",
        sn_prefix: "22E",
    },
    ModelEntry {
        canonical: CanonicalModel::A2L,
        code: "N9",
        aliases: &[],
        product_name: "Bambu Lab A2L",
        sn_prefix: "26A",
    },
    // O1C and O1C2 are the same marketed printer and share a serial prefix.
    // Bambu's own loader keeps whichever it indexed first; nothing in Melt
    // routes on the variant, so both rows resolve to one canonical model.
    ModelEntry {
        canonical: CanonicalModel::H2C,
        code: "O1C",
        aliases: &[],
        product_name: "Bambu Lab H2C",
        sn_prefix: "31B",
    },
    ModelEntry {
        canonical: CanonicalModel::H2C,
        code: "O1C2",
        aliases: &[],
        product_name: "Bambu Lab H2C",
        sn_prefix: "31B",
    },
    ModelEntry {
        canonical: CanonicalModel::H2D,
        code: "O1D",
        aliases: &[],
        product_name: "Bambu Lab H2D",
        sn_prefix: "094",
    },
    ModelEntry {
        canonical: CanonicalModel::H2DPro,
        code: "O1E",
        aliases: &[],
        product_name: "Bambu Lab H2D Pro",
        sn_prefix: "239",
    },
    ModelEntry {
        canonical: CanonicalModel::H2S,
        code: "O1S",
        aliases: &[],
        product_name: "Bambu Lab H2S",
        sn_prefix: "093",
    },
];

pub fn models() -> &'static [ModelEntry] {
    MODELS
}

impl CanonicalModel {
    /// Marketing name, as Bambu firmware reports it in `product_name`.
    pub fn display_name(self) -> &'static str {
        MODELS
            .iter()
            .find(|entry| entry.canonical == self)
            .map_or("Unknown", |entry| entry.product_name)
    }

    /// Bambu's `model_id`. `H2C` deliberately reports only the first of its
    /// two interchangeable codes.
    pub fn code(self) -> Option<&'static str> {
        MODELS
            .iter()
            .find(|entry| entry.canonical == self)
            .map(|entry| entry.code)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelIdentity {
    /// Exact value advertised by the printer or supplied by the user.
    pub raw: String,
    /// Separator-insensitive value used only for matching.
    pub normalized: String,
    pub canonical: CanonicalModel,
    pub family: ModelFamily,
}

fn normalize(raw: &str) -> String {
    raw.trim().to_ascii_uppercase().replace(['-', '_', ' '], "")
}

impl ModelIdentity {
    /// The marketing name when the model is recognized, the raw value
    /// verbatim when it is not. Empty only when nothing identified the
    /// printer at all, so a caller can decide what to show in its place.
    pub fn display_name(&self) -> &str {
        match self.canonical {
            CanonicalModel::Unknown => self.raw.as_str(),
            canonical => canonical.display_name(),
        }
    }

    /// Accepts a Bambu `model_id` (`C12`), a legacy `DevModel` alias
    /// (`3DPrinter-X1-Carbon`), a marketing name (`Bambu Lab P1S`), or the
    /// bare marketing suffix a user is likely to type (`P1S`).
    pub fn parse(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let normalized = normalize(&raw);
        let canonical = MODELS
            .iter()
            .find(|entry| {
                normalize(entry.code) == normalized
                    || normalize(entry.product_name) == normalized
                    || entry
                        .aliases
                        .iter()
                        .any(|alias| normalize(alias) == normalized)
            })
            .map(|entry| entry.canonical)
            .or_else(|| marketing_suffix(&normalized))
            .unwrap_or_default();
        Self::new(raw, normalized, canonical)
    }

    /// Resolves the model from the leading three characters of a physical
    /// serial. The pinned certificate binds that serial to the printer, so
    /// this works offline and survives firmware that never names itself.
    pub fn from_serial(serial: &str) -> Self {
        let serial = serial.trim().to_ascii_uppercase();
        let canonical = serial
            .get(..3)
            .and_then(|prefix| {
                MODELS
                    .iter()
                    .find(|entry| entry.sn_prefix == prefix)
                    .map(|entry| entry.canonical)
            })
            .unwrap_or_default();
        let raw = canonical.code().unwrap_or_default().to_owned();
        let normalized = normalize(&raw);
        Self::new(raw, normalized, canonical)
    }

    fn new(raw: String, normalized: String, canonical: CanonicalModel) -> Self {
        // A `model_id` such as `C12` or `O1D` says nothing about the family on
        // its own, so a resolved model derives it from the marketing name.
        let family = match canonical {
            CanonicalModel::Unknown => ModelFamily::from_model(&normalized),
            canonical => ModelFamily::from_model(
                normalize(canonical.display_name())
                    .strip_prefix("BAMBULAB")
                    .unwrap_or_default(),
            ),
        };
        Self {
            raw,
            normalized,
            canonical,
            family,
        }
    }
}

/// The marketing suffix without the vendor prefix, which is what a user types
/// into `--model` and what existing Melt profiles already store.
fn marketing_suffix(normalized: &str) -> Option<CanonicalModel> {
    MODELS
        .iter()
        .find(|entry| {
            normalize(entry.product_name)
                .strip_prefix("BAMBULAB")
                .is_some_and(|suffix| suffix == normalized)
        })
        .map(|entry| entry.canonical)
        .or(match normalized {
            "X1C" => Some(CanonicalModel::X1Carbon),
            _ => None,
        })
}

/// Which LAN channel named the printer, in descending order of trust.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelSource {
    /// `info.get_version` module `product_name`.
    VersionProductName,
    /// `info.get_version` module `project_name`, sent by legacy firmware.
    VersionProjectName,
    /// Leading three characters of the configured, TLS-bound serial.
    SerialPrefix,
    /// The model string stored in the profile.
    Configured,
    #[default]
    None,
}

/// A lower-trust channel that named a different model. Retained so a
/// disagreement is visible in diagnostics; it never changes the resolved
/// model, and it never carries the serial it was derived from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConflict {
    pub source: ModelSource,
    pub canonical: CanonicalModel,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelDetection {
    pub identity: ModelIdentity,
    pub source: ModelSource,
    pub conflicts: Vec<ModelConflict>,
}

/// Resolves the model from every LAN channel available to this session.
///
/// SSDP is deliberately excluded. `DevModel.bambu.com` carries the model code,
/// but it is an unauthenticated broadcast any host on the network can forge,
/// while the serial prefix is derived from the serial the pinned certificate
/// binds to this physical printer. SSDP stays advisory input to discovery.
///
/// The highest-trust channel that resolves wins; every lower channel that
/// named a different model is recorded as a conflict rather than resolved, so
/// a stale `--model` is visible without demoting a confidently detected
/// printer. An unrecognized name is still returned verbatim with an unknown
/// canonical model, because new firmware must stay operable and diagnosable.
///
/// The configured string is the one channel that can go stale, so it decides
/// only when the printer itself named nothing. A model this table has never
/// seen still outranks a recognized one typed by hand: routing an unreleased
/// printer as a P1S is a misclassification, and it would erase the only
/// evidence a new row could be added from.
pub fn detect(serial: &str, configured: &str, firmware: &FirmwareInventory) -> ModelDetection {
    let named = |pick: fn(&FirmwareModule) -> Option<&String>| {
        firmware
            .modules
            .iter()
            .filter(|module| !module.is_accessory())
            .find_map(pick)
            .map(ModelIdentity::parse)
            .unwrap_or_default()
    };
    let candidates = [
        (
            ModelSource::VersionProductName,
            named(|module| module.product.as_ref()),
        ),
        (
            ModelSource::VersionProjectName,
            named(|module| module.project.as_ref()),
        ),
        (
            ModelSource::SerialPrefix,
            ModelIdentity::from_serial(serial),
        ),
        (ModelSource::Configured, ModelIdentity::parse(configured)),
    ];

    let resolved = candidates
        .iter()
        .find(|(_, identity)| identity.canonical != CanonicalModel::Unknown);
    let reported = candidates
        .iter()
        .filter(|(source, _)| *source != ModelSource::Configured)
        .find(|(_, identity)| !identity.raw.trim().is_empty());
    let chosen = match (resolved, reported) {
        (Some(candidate), _) if candidate.0 != ModelSource::Configured => Some(candidate),
        (_, Some(candidate)) => Some(candidate),
        (candidate, None) => candidate,
    };
    let Some((source, identity)) = chosen else {
        // Only the configured string exists and it matched nothing. Keep it
        // so an unmapped model is inventoried rather than erased.
        let (source, identity) = candidates
            .into_iter()
            .find(|(_, identity)| !identity.raw.trim().is_empty())
            .unwrap_or_default();
        return ModelDetection {
            identity,
            source,
            conflicts: Vec::new(),
        };
    };
    let conflicts = candidates
        .iter()
        .filter(|(candidate, other)| {
            *candidate != *source
                && other.canonical != CanonicalModel::Unknown
                && other.canonical != identity.canonical
        })
        .map(|(source, other)| ModelConflict {
            source: *source,
            canonical: other.canonical,
        })
        .collect();
    ModelDetection {
        identity: identity.clone(),
        source: *source,
        conflicts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unknown_internal_identifiers() {
        let identity = ModelIdentity::parse("  future_dev_42 ");
        assert_eq!(identity.raw, "  future_dev_42 ");
        assert_eq!(identity.normalized, "FUTUREDEV42");
        assert_eq!(identity.canonical, CanonicalModel::Unknown);
        assert_eq!(identity.family, ModelFamily::Unknown);
    }

    #[test]
    fn canonicalizes_only_explicit_aliases() {
        assert_eq!(
            ModelIdentity::parse("X1-C").canonical,
            CanonicalModel::X1Carbon
        );
        assert_eq!(ModelIdentity::parse("P1S").family, ModelFamily::P1);
    }

    #[test]
    fn resolves_every_model_id_code() {
        for entry in models() {
            assert_eq!(
                ModelIdentity::parse(entry.code).canonical,
                entry.canonical,
                "{}",
                entry.code
            );
            assert_eq!(
                ModelIdentity::parse(entry.product_name).canonical,
                entry.canonical,
                "{}",
                entry.product_name
            );
            assert_eq!(
                ModelIdentity::from_serial(&format!("{}ABC1234567890", entry.sn_prefix)).canonical,
                entry.canonical,
                "{}",
                entry.sn_prefix
            );
        }
    }

    #[test]
    fn accepts_legacy_x1_ssdp_spellings() {
        assert_eq!(
            ModelIdentity::parse("3DPrinter-X1-Carbon").canonical,
            CanonicalModel::X1Carbon
        );
        assert_eq!(
            ModelIdentity::parse("3DPrinter-X1").canonical,
            CanonicalModel::X1
        );
    }

    /// Community tables invert these three pairs. Bambu's own `sn_prefix`
    /// values are authoritative; this test guards against re-importing the
    /// wrong table.
    #[test]
    fn rejects_the_known_inverted_community_mappings() {
        assert_eq!(
            ModelIdentity::from_serial("00M00000000000").canonical,
            CanonicalModel::X1Carbon
        );
        assert_eq!(
            ModelIdentity::from_serial("00W00000000000").canonical,
            CanonicalModel::X1
        );
        assert_eq!(
            ModelIdentity::from_serial("03000000000000").canonical,
            CanonicalModel::A1Mini
        );
        assert_eq!(
            ModelIdentity::from_serial("03900000000000").canonical,
            CanonicalModel::A1
        );
        assert_eq!(ModelIdentity::parse("C11").canonical, CanonicalModel::P1P);
        assert_eq!(ModelIdentity::parse("C12").canonical, CanonicalModel::P1S);
    }

    #[test]
    fn both_h2c_codes_share_one_canonical_model() {
        assert_eq!(ModelIdentity::parse("O1C").canonical, CanonicalModel::H2C);
        assert_eq!(ModelIdentity::parse("O1C2").canonical, CanonicalModel::H2C);
        assert_eq!(
            ModelIdentity::from_serial("31B00000000000").canonical,
            CanonicalModel::H2C
        );
    }

    #[test]
    fn derives_the_family_from_the_canonical_model_not_the_code() {
        assert_eq!(ModelIdentity::parse("C12").family, ModelFamily::P1);
        assert_eq!(ModelIdentity::parse("O1D").family, ModelFamily::H2);
        assert_eq!(ModelIdentity::parse("N2S").family, ModelFamily::A1);
        assert_eq!(ModelIdentity::parse("N1").family, ModelFamily::A1);
        assert_eq!(ModelIdentity::parse("BL-P001").family, ModelFamily::X1);
        assert_eq!(ModelIdentity::parse("N9").family, ModelFamily::A2);
        assert_eq!(ModelIdentity::parse("N6").family, ModelFamily::X2);
        assert_eq!(ModelIdentity::parse("N7").family, ModelFamily::P2);
        assert_eq!(ModelIdentity::from_serial("094X").family, ModelFamily::H2);
    }

    #[test]
    fn a_short_or_unmapped_serial_stays_unknown() {
        for serial in ["", "00", "ZZZ00000000000"] {
            let identity = ModelIdentity::from_serial(serial);
            assert_eq!(identity.canonical, CanonicalModel::Unknown, "{serial}");
            assert!(identity.raw.is_empty(), "{serial}");
        }
    }

    #[test]
    fn table_is_internally_consistent() {
        for entry in models() {
            assert_eq!(entry.sn_prefix.len(), 3, "{}", entry.code);
            assert!(
                entry.product_name.starts_with("Bambu Lab "),
                "{}",
                entry.code
            );
            assert_eq!(entry.canonical.display_name(), entry.product_name);
            assert_ne!(entry.canonical, CanonicalModel::Unknown);
            assert_ne!(
                ModelIdentity::parse(entry.code).family,
                ModelFamily::Unknown,
                "{}",
                entry.code
            );
        }
        for (index, entry) in models().iter().enumerate() {
            for other in models().iter().skip(index + 1) {
                assert_ne!(entry.code, other.code, "duplicate code {}", entry.code);
                for alias in entry.aliases {
                    assert_ne!(*alias, other.code, "alias collides with a code: {alias}");
                    assert!(
                        !other.aliases.contains(alias),
                        "duplicate alias: {alias} on {}",
                        entry.code
                    );
                }
                // Only the two interchangeable H2C codes may share a prefix.
                if entry.sn_prefix == other.sn_prefix {
                    assert_eq!(
                        entry.canonical, other.canonical,
                        "prefix {} maps to two models",
                        entry.sn_prefix
                    );
                }
            }
        }
    }

    #[test]
    fn unknown_has_no_code_and_a_placeholder_name() {
        assert_eq!(CanonicalModel::Unknown.code(), None);
        assert_eq!(CanonicalModel::Unknown.display_name(), "Unknown");
    }

    fn inventory(info: serde_json::Value) -> FirmwareInventory {
        FirmwareInventory::from_version_info(&info)
    }

    #[test]
    fn prefers_product_name_over_every_other_channel() {
        let detection = detect(
            "22E8BJ610801473",
            "P1S",
            &inventory(serde_json::json!({"module":[
                {"name":"ota","product_name":"Bambu Lab P2S","sw_ver":"01.02.00.00"}
            ]})),
        );
        assert_eq!(detection.identity.canonical, CanonicalModel::P2S);
        assert_eq!(detection.source, ModelSource::VersionProductName);
        assert_eq!(detection.identity.raw, "Bambu Lab P2S");
        assert_eq!(
            detection.conflicts,
            [ModelConflict {
                source: ModelSource::Configured,
                canonical: CanonicalModel::P1S,
            }]
        );
    }

    /// The A1 fixture: no `product_name` anywhere, `project_name` on every
    /// module, and an AMS Lite whose own `project_name` is blank.
    #[test]
    fn falls_back_to_project_name_on_legacy_firmware() {
        let detection = detect(
            "",
            "",
            &inventory(serde_json::json!({"module":[
                {"name":"ota","project_name":"N2S","sw_ver":"01.05.00.00","hw_ver":"OTA"},
                {"name":"esp32","project_name":"N2S","sw_ver":"01.13.33.99","hw_ver":"AP05"},
                {"name":"ams_f1/0","project_name":"","hw_ver":"AMS_F102"}
            ]})),
        );
        assert_eq!(detection.identity.canonical, CanonicalModel::A1);
        assert_eq!(detection.source, ModelSource::VersionProjectName);
        assert!(detection.conflicts.is_empty());
    }

    #[test]
    fn never_reads_the_model_from_an_accessory() {
        let detection = detect(
            "03912345678901",
            "",
            &inventory(serde_json::json!({"module":[
                {"name":"ams_f1/0","product_name":"AMS Lite (1)"},
                {"name":"n3s/128","product_name":"AMS HT (1)"},
                {"name":"ota","product_name":"Bambu Lab A1"}
            ]})),
        );
        assert_eq!(detection.identity.canonical, CanonicalModel::A1);
        assert_eq!(detection.source, ModelSource::VersionProductName);
    }

    /// X1-series firmware names itself in neither field, so the serial is the
    /// only thing that separates an X1 from an X1 Carbon.
    #[test]
    fn separates_x1_from_x1_carbon_by_serial_when_firmware_is_silent() {
        let silent = inventory(serde_json::json!({"module":[
            {"name":"ota","sw_ver":"01.01.01.00","hw_ver":"","sn":""},
            {"name":"rv1126","hw_ver":"AP05","sw_ver":"00.00.14.74"},
            {"name":"xm","hw_ver":"","sn":"","sw_ver":"00.00.00.00"}
        ]}));
        let carbon = detect("00M00000000000", "", &silent);
        assert_eq!(carbon.identity.canonical, CanonicalModel::X1Carbon);
        assert_eq!(carbon.source, ModelSource::SerialPrefix);
        assert_eq!(
            detect("00W00000000000", "", &silent).identity.canonical,
            CanonicalModel::X1
        );
    }

    #[test]
    fn a_configured_model_is_the_last_resort_not_the_first() {
        let detection = detect("", "C12", &FirmwareInventory::default());
        assert_eq!(detection.identity.canonical, CanonicalModel::P1S);
        assert_eq!(detection.source, ModelSource::Configured);
        assert!(detection.conflicts.is_empty());
    }

    #[test]
    fn an_unresolvable_printer_stays_unknown_without_conflicts() {
        let detection = detect("", "", &FirmwareInventory::default());
        assert_eq!(detection.identity.canonical, CanonicalModel::Unknown);
        assert_eq!(detection.source, ModelSource::None);
        assert!(detection.identity.raw.is_empty());
        assert!(detection.conflicts.is_empty());
    }

    /// An unmapped model must stay inventoried and operable: keep what the
    /// printer said, and say which channel said it.
    #[test]
    fn an_unmapped_product_name_is_preserved_verbatim() {
        let detection = detect(
            "ZZZ00000000000",
            "",
            &inventory(serde_json::json!({"module":[
                {"name":"ota","product_name":"Bambu Lab H3X","sw_ver":"09.00.00.00"}
            ]})),
        );
        assert_eq!(detection.identity.canonical, CanonicalModel::Unknown);
        assert_eq!(detection.identity.raw, "Bambu Lab H3X");
        assert_eq!(detection.source, ModelSource::VersionProductName);
        assert!(detection.conflicts.is_empty());
    }

    /// A model this table has never seen still outranks a recognized one
    /// typed by hand. Resolving the configured string here would route an
    /// unreleased printer as a P1S and erase the only evidence a new row
    /// could be added from.
    #[test]
    fn an_unmapped_report_outranks_a_recognized_configured_string() {
        let detection = detect(
            "ZZZ00000000000",
            "P1S",
            &inventory(serde_json::json!({"module":[
                {"name":"ota","product_name":"Bambu Lab H3X","sw_ver":"09.00.00.00"}
            ]})),
        );
        assert_eq!(detection.identity.canonical, CanonicalModel::Unknown);
        assert_eq!(detection.identity.raw, "Bambu Lab H3X");
        assert_eq!(detection.source, ModelSource::VersionProductName);
        assert_eq!(
            detection.conflicts,
            [ModelConflict {
                source: ModelSource::Configured,
                canonical: CanonicalModel::P1S,
            }]
        );
    }

    /// Both firmware names are the printer's own authenticated report, so an
    /// unrecognized `product_name` must not suppress a `project_name` the
    /// table does know.
    #[test]
    fn an_unmapped_product_name_yields_to_a_known_project_name() {
        let detection = detect(
            "ZZZ00000000000",
            "",
            &inventory(serde_json::json!({"module":[
                {"name":"ota","product_name":"Bambu Lab H3X","project_name":"N2S"}
            ]})),
        );
        assert_eq!(detection.identity.canonical, CanonicalModel::A1);
        assert_eq!(detection.source, ModelSource::VersionProjectName);
        assert!(detection.conflicts.is_empty());
    }

    /// The configured string still decides when the printer named nothing.
    #[test]
    fn the_configured_string_decides_a_silent_printer() {
        let detection = detect("ZZZ00000000000", "P1S", &FirmwareInventory::default());
        assert_eq!(detection.identity.canonical, CanonicalModel::P1S);
        assert_eq!(detection.source, ModelSource::Configured);
        assert!(detection.conflicts.is_empty());
    }

    #[test]
    fn an_unknown_configured_string_is_not_a_conflict() {
        let detection = detect(
            "01P00000000000",
            "future_dev_42",
            &FirmwareInventory::default(),
        );
        assert_eq!(detection.identity.canonical, CanonicalModel::P1S);
        assert_eq!(detection.source, ModelSource::SerialPrefix);
        assert!(detection.conflicts.is_empty());
    }

    #[test]
    fn agreeing_channels_are_not_conflicts() {
        let detection = detect(
            "01P00000000000",
            "Bambu Lab P1S",
            &inventory(serde_json::json!({"module":[
                {"name":"ota","product_name":"Bambu Lab P1S","project_name":"C12"}
            ]})),
        );
        assert_eq!(detection.source, ModelSource::VersionProductName);
        assert!(detection.conflicts.is_empty());
    }

    #[test]
    fn every_disagreeing_channel_is_recorded() {
        let detection = detect(
            "00M00000000000",
            "A1",
            &inventory(serde_json::json!({"module":[
                {"name":"ota","product_name":"Bambu Lab H2D","project_name":"O1S"}
            ]})),
        );
        assert_eq!(detection.identity.canonical, CanonicalModel::H2D);
        assert_eq!(
            detection.conflicts,
            [
                ModelConflict {
                    source: ModelSource::VersionProjectName,
                    canonical: CanonicalModel::H2S,
                },
                ModelConflict {
                    source: ModelSource::SerialPrefix,
                    canonical: CanonicalModel::X1Carbon,
                },
                ModelConflict {
                    source: ModelSource::Configured,
                    canonical: CanonicalModel::A1,
                },
            ]
        );
    }

    /// Conflicts reach diagnostics, so they must not carry the serial they
    /// were derived from.
    #[test]
    fn a_serial_prefix_conflict_does_not_carry_the_serial() {
        let detection = detect(
            "00MSECRETSERIAL",
            "",
            &inventory(serde_json::json!({"module":[
                {"name":"ota","product_name":"Bambu Lab A1"}
            ]})),
        );
        let serialized = serde_json::to_string(&detection.conflicts).unwrap();
        assert!(!serialized.contains("SECRET"), "{serialized}");
        assert!(!serialized.contains("00M"), "{serialized}");
    }
}
