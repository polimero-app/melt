use std::{collections::BTreeMap, sync::LazyLock};

use serde::Deserialize;

use super::{CanonicalModel, ModelIdentity};

type Variants = BTreeMap<String, Vec<String>>;

#[derive(Deserialize)]
struct Texts {
    device_hms: BTreeMap<String, Variants>,
    device_error: BTreeMap<String, Variants>,
}

#[derive(Deserialize)]
struct Catalogue {
    en: Texts,
    #[serde(rename = "pt-BR")]
    portuguese: Texts,
    wiki: BTreeMap<String, Variants>,
}

static CATALOGUE: LazyLock<Catalogue> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../data/bambu-errors.json"))
        .expect("checked-in Bambu error catalogue must be valid")
});

pub(super) enum Kind {
    Hms,
    Print,
}

pub(super) struct Description {
    pub message: Option<&'static str>,
    pub translations: BTreeMap<String, String>,
    pub guide_url: Option<String>,
}

pub(super) fn describe(kind: Kind, code: &str, model: CanonicalModel) -> Description {
    let catalogue = &*CATALOGUE;
    let (english, portuguese) = match kind {
        Kind::Hms => (&catalogue.en.device_hms, &catalogue.portuguese.device_hms),
        Kind::Print => (
            &catalogue.en.device_error,
            &catalogue.portuguese.device_error,
        ),
    };
    Description {
        message: select(english.get(code), model),
        translations: select(portuguese.get(code), model)
            .map(|message| ("pt-BR".into(), message.to_owned()))
            .into_iter()
            .collect(),
        guide_url: select(catalogue.wiki.get(code), model)
            .map(|path| format!("https://wiki.bambulab.com{path}")),
    }
}

fn select(variants: Option<&Variants>, model: CanonicalModel) -> Option<&str> {
    let variants = variants?;
    variants
        .iter()
        .find(|(text, models)| {
            !text.trim().is_empty()
                && model != CanonicalModel::Unknown
                && models
                    .iter()
                    .any(|name| ModelIdentity::parse(name).canonical == model)
        })
        .or_else(|| {
            variants
                .iter()
                .find(|(text, models)| !text.trim().is_empty() && models.is_empty())
        })
        .map(|(text, _)| text.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_specific_messages_outrank_generic_entries_in_both_languages() {
        let right = describe(Kind::Hms, "0300020000010001", CanonicalModel::H2D);
        assert!(right.message.unwrap().contains("right nozzle"));
        assert!(right.translations["pt-BR"].contains("correta"));
        for model in [CanonicalModel::P2S, CanonicalModel::Unknown] {
            let generic = describe(Kind::Hms, "0300020000010001", model);
            assert!(!generic.message.unwrap().contains("right nozzle"));
            assert!(!generic.translations["pt-BR"].contains("correta"));
        }
    }

    #[test]
    fn missing_translations_unknown_codes_and_empty_placeholders_remain_explicit() {
        let english_only = describe(Kind::Hms, "0C0003000001001E", CanonicalModel::Unknown);
        assert!(english_only.message.unwrap().starts_with("Smoke detected."));
        assert!(english_only.translations.is_empty());
        for code in ["FFFFFFFFFFFFFFFF", "0300180000000000"] {
            let unknown = describe(Kind::Hms, code, CanonicalModel::Unknown);
            assert!(unknown.message.is_none());
            assert!(unknown.translations.is_empty());
        }
    }

    #[test]
    fn wiki_paths_are_model_specific_before_generic_fallback() {
        let code = "0300010000010007";
        for (model, path) in [
            (CanonicalModel::P2S, "/en/p2s/"),
            (CanonicalModel::H2DPro, "/en/h2/"),
            (CanonicalModel::X2D, "/en/x2d/"),
            (CanonicalModel::Unknown, "/en/x1/"),
        ] {
            assert!(
                describe(Kind::Hms, code, model)
                    .guide_url
                    .unwrap()
                    .contains(path)
            );
        }
    }
}
