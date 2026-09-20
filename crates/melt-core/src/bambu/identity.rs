use serde::{Deserialize, Serialize};

use super::ModelFamily;

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
}
