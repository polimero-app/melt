use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use thiserror::Error;

use super::{CanonicalModel, ModelFamily};

pub const MAX_PUBLIC_CATALOGUE_BYTES: usize = 2 << 20;
const PUBLIC_CATALOGUE_HOSTS: [&str; 2] = ["bambulab.com", "www.bambulab.com"];
/// Bambu ships at most one stable release per model per day and serves these
/// pages behind Cloudflare, so a short TTL would buy nothing and risk being
/// rate limited.
const PUBLIC_CATALOGUE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Error)]
pub enum PublicCatalogueError {
    #[error("the printer model is not allowlisted for public firmware lookup")]
    UnsupportedModel,
    #[error("public firmware catalogue URL is not allowlisted")]
    InvalidUrl,
    #[error("public firmware catalogue request failed")]
    Request(#[from] reqwest::Error),
    #[error("public firmware catalogue returned an unsuccessful status")]
    Status,
    #[error("public firmware catalogue response is too large")]
    TooLarge,
    #[error("public firmware catalogue did not contain a stable firmware version")]
    VersionNotFound,
    #[error("public firmware catalogue response could not be read")]
    Read,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicFirmwareRelease {
    pub model: CanonicalModel,
    pub version: String,
    pub url: String,
}

pub fn public_catalogue_url(model: CanonicalModel) -> Option<&'static str> {
    public_catalogue_urls(model).first().copied()
}

/// Returns the model-specific official page first, optionally followed by a
/// shared family page that publishes a release history for that family.
///
/// A fallback is only safe when the page is not another model's own page:
/// Bambu ships distinct, non-interchangeable firmware per model, so reading
/// `.../a1` for an A1 mini would report a different printer's version as this
/// printer's public-stable release. `public_catalogue_family_pages_are_shared`
/// locks that invariant in.
///
/// Every slug here was confirmed to answer 200 to Melt's own user agent on
/// 2026-09-20. bambulab.com blocks some tooling user agents, so this is not
/// checkable from CI; a slug that rots degrades to
/// `publicCatalogueUnavailable`, never to another model's firmware version.
pub fn public_catalogue_urls(model: CanonicalModel) -> &'static [&'static str] {
    match model {
        CanonicalModel::A1 => &["https://bambulab.com/en-us/support/firmware-download/a1"],
        CanonicalModel::A1Mini => &["https://bambulab.com/en-us/support/firmware-download/a1-mini"],
        CanonicalModel::A2L => &["https://bambulab.com/en-us/support/firmware-download/a2l"],
        CanonicalModel::P1P => &[
            "https://bambulab.com/en-us/support/firmware-download/p1p",
            "https://bambulab.com/en-us/support/firmware-download/p1",
        ],
        CanonicalModel::P1S => &[
            "https://bambulab.com/en-us/support/firmware-download/p1s",
            "https://bambulab.com/en-us/support/firmware-download/p1",
        ],
        CanonicalModel::P2S => &["https://bambulab.com/en-us/support/firmware-download/p2s"],
        CanonicalModel::X1 => &["https://bambulab.com/en-us/support/firmware-download/x1"],
        CanonicalModel::X1Carbon => &["https://bambulab.com/en-us/support/firmware-download/x1c"],
        CanonicalModel::X1E => &["https://bambulab.com/en-us/support/firmware-download/x1e"],
        CanonicalModel::X2D => &["https://bambulab.com/en-us/support/firmware-download/x2d"],
        CanonicalModel::H2D => &["https://bambulab.com/en-us/support/firmware-download/h2d"],
        CanonicalModel::H2DPro => &["https://bambulab.com/en-us/support/firmware-download/h2d-pro"],
        CanonicalModel::H2S => &["https://bambulab.com/en-us/support/firmware-download/h2s"],
        CanonicalModel::H2C => &["https://bambulab.com/en-us/support/firmware-download/h2c"],
        CanonicalModel::Unknown => &[],
    }
}

struct CachedRelease {
    release: PublicFirmwareRelease,
    fetched_at: Instant,
}

fn fresh(entry: &CachedRelease) -> bool {
    entry.fetched_at.elapsed() < PUBLIC_CATALOGUE_TTL
}

/// One slot per canonical model, shared by every profile of that model, so a
/// result is reused across printers instead of refetched per printer.
type Slot = Arc<Mutex<Option<CachedRelease>>>;

fn slot(model: CanonicalModel) -> Slot {
    static SLOTS: OnceLock<Mutex<HashMap<CanonicalModel, Slot>>> = OnceLock::new();
    let slots = SLOTS.get_or_init(Mutex::default);
    let mut slots = slots.lock().unwrap_or_else(|error| error.into_inner());
    slots.entry(model).or_default().clone()
}

/// Fetches the latest public stable release for a model, reusing a cached
/// result for [`PUBLIC_CATALOGUE_TTL`].
///
/// The per-model slot is held across the fetch, so concurrent callers for the
/// same model wait for the first request and share its result rather than
/// each issuing their own. A waiting caller can therefore block for longer
/// than its own `timeout`, which is the point: one request per model.
pub fn fetch_public_firmware(
    model: CanonicalModel,
    timeout: Duration,
) -> Result<PublicFirmwareRelease, PublicCatalogueError> {
    let slot = slot(model);
    let mut cached = slot.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(entry) = cached.as_ref().filter(|entry| fresh(entry)) {
        return Ok(entry.release.clone());
    }
    // ponytail: successes only, so a transient Cloudflare block or parse
    // failure retries on the next check instead of being pinned for a day.
    // Add negative caching with backoff if those retries ever read as
    // rate limiting.
    let release = fetch_uncached(model, timeout)?;
    *cached = Some(CachedRelease {
        release: release.clone(),
        fetched_at: Instant::now(),
    });
    Ok(release)
}

fn fetch_uncached(
    model: CanonicalModel,
    timeout: Duration,
) -> Result<PublicFirmwareRelease, PublicCatalogueError> {
    let urls = public_catalogue_urls(model);
    if urls.is_empty() {
        return Err(PublicCatalogueError::UnsupportedModel);
    }
    let client = Client::builder()
        .timeout(timeout)
        .redirect(Policy::none())
        .user_agent("Melt firmware availability/1.0")
        .build()?;
    let mut last_error = PublicCatalogueError::Status;
    for url in urls {
        let parsed = reqwest::Url::parse(url).map_err(|_| PublicCatalogueError::InvalidUrl)?;
        if parsed.scheme() != "https"
            || !parsed
                .host_str()
                .is_some_and(|host| PUBLIC_CATALOGUE_HOSTS.contains(&host))
        {
            return Err(PublicCatalogueError::InvalidUrl);
        }
        let mut response = match client.get(parsed).send() {
            Ok(response) => response,
            Err(error) => {
                last_error = PublicCatalogueError::Request(error);
                continue;
            }
        };
        if !response.status().is_success() {
            last_error = PublicCatalogueError::Status;
            continue;
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_PUBLIC_CATALOGUE_BYTES as u64)
        {
            last_error = PublicCatalogueError::TooLarge;
            continue;
        }
        let mut body = Vec::new();
        if response
            .by_ref()
            .take((MAX_PUBLIC_CATALOGUE_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .is_err()
        {
            last_error = PublicCatalogueError::Read;
            continue;
        }
        if body.len() > MAX_PUBLIC_CATALOGUE_BYTES {
            last_error = PublicCatalogueError::TooLarge;
            continue;
        }
        let html = String::from_utf8_lossy(&body);
        if let Some(version) = parse_public_firmware_version(&html, model) {
            return Ok(PublicFirmwareRelease {
                model,
                version,
                url: (*url).to_owned(),
            });
        }
        last_error = PublicCatalogueError::VersionNotFound;
    }
    Err(last_error)
}

/// The shape every Bambu firmware version takes, on the public pages and in
/// the `sw_ver` a printer reports alike: four groups of exactly two digits.
/// A looser shape reads page furniture as firmware -- an inline SVG path
/// (`a.938.938 0 011.323.078l5`) once reported `011.323.078` for an A1 mini.
const VERSION_SHAPE: &[u8] = b"dd.dd.dd.dd";

/// Extracts the highest firmware version the page publishes *for this model*.
///
/// Every download page embeds the whole catalogue, keyed by the same
/// `devModel` code the printer reports, and the release notes quote the
/// recommended versions of attached accessories. Reading the page at large
/// therefore reports whichever product happens to carry the highest number --
/// on the A1 mini page, an AMS 2 Pro's `04.00.21.87`. Only the entries
/// published under this model's own code count.
pub fn parse_public_firmware_version(html: &str, model: CanonicalModel) -> Option<String> {
    let family = model_family(model)?;
    // Bambu ships one binary per family and does not publish every model
    // separately: the P1S takes the P1P's `C11`, the X1 the X1 Carbon's
    // `BL-P001`. Widen to the family only when the model publishes nothing of
    // its own, so an A1 mini never picks up the A1's release.
    highest_published(html, |entry| entry.canonical == model)
        .or_else(|| highest_published(html, |entry| model_family(entry.canonical) == Some(family)))
}

/// `None` for a model with no family, the one case where widening the search
/// would sweep in every printer Bambu sells.
fn model_family(model: CanonicalModel) -> Option<ModelFamily> {
    match crate::bambu::ModelIdentity::parse(model.display_name()).family {
        ModelFamily::Unknown => None,
        family => Some(family),
    }
}

fn highest_published(
    html: &str,
    mut wanted: impl FnMut(&crate::bambu::ModelEntry) -> bool,
) -> Option<String> {
    crate::bambu::models()
        .iter()
        .filter(|entry| wanted(entry))
        .flat_map(|entry| std::iter::once(entry.code).chain(entry.aliases.iter().copied()))
        .flat_map(|code| model_download_versions(html, code))
        .max_by(|left, right| {
            crate::bambu::FirmwareVersion::parse(left)
                .numeric_cmp(&crate::bambu::FirmwareVersion::parse(right))
        })
}

/// Versions from the catalogue entries published under one `devModel` code.
///
/// The code is matched as a prefix so a hardware revision counts as the same
/// machine -- the H2C is published as `O1C2-V2`. The URL is not a reliable
/// anchor on its own: the X2D is keyed `N6` but served from `/X2D/`.
fn model_download_versions(html: &str, code: &str) -> Vec<String> {
    let anchor = format!("\"devModel\":\"{code}");
    html.match_indices(&anchor)
        .filter_map(|(index, _)| {
            let rest = html.get(index + anchor.len()..)?;
            // Either the code ends here or a hardware revision follows it.
            if !rest.starts_with('"') && !rest.starts_with('-') {
                return None;
            }
            // Stay inside this entry. The same key also introduces a product
            // blurb, which carries no version at all.
            let entry = rest.split('}').next()?;
            let version = entry.split("\"version\":\"").nth(1)?.split('"').next()?;
            is_version(version).then(|| version.to_owned())
        })
        .collect()
}

fn is_version(text: &str) -> bool {
    text.len() == VERSION_SHAPE.len()
        && text
            .bytes()
            .zip(VERSION_SHAPE)
            .all(|(byte, shape)| match shape {
                b'.' => byte == b'.',
                _ => byte.is_ascii_digit(),
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlists_known_models_and_rejects_unknown() {
        assert!(public_catalogue_url(CanonicalModel::P1S).is_some());
        assert!(public_catalogue_url(CanonicalModel::Unknown).is_none());
        assert_eq!(
            public_catalogue_url(CanonicalModel::A1Mini),
            Some("https://bambulab.com/en-us/support/firmware-download/a1-mini")
        );
        assert!(
            public_catalogue_urls(CanonicalModel::P1S)
                .contains(&"https://bambulab.com/en-us/support/firmware-download/p1")
        );
    }

    /// Every model ships its own firmware, so a fallback page must never be
    /// another model's own catalogue page. Without this, a failed scrape of
    /// `.../a1-mini` would silently report the A1's version for an A1 mini.
    #[test]
    fn public_catalogue_family_pages_are_shared() {
        let models = super::super::models()
            .iter()
            .map(|entry| entry.canonical)
            .chain([CanonicalModel::Unknown])
            .collect::<Vec<_>>();
        let primaries: Vec<&str> = models
            .iter()
            .filter_map(|model| public_catalogue_url(*model))
            .collect();
        for model in models {
            assert_eq!(
                public_catalogue_url(model).is_some(),
                model != CanonicalModel::Unknown,
                "{model:?} has no official catalogue page"
            );
            for fallback in public_catalogue_urls(model).iter().skip(1) {
                assert!(
                    !primaries.contains(fallback),
                    "{model:?} falls back to another model's page: {fallback}"
                );
            }
        }
    }

    /// A cache hit must answer without touching the network, and an entry
    /// older than the TTL must stop counting as one.
    #[test]
    fn public_catalogue_results_are_cached_per_model_until_the_ttl_expires() {
        let model = CanonicalModel::H2C;
        let release = PublicFirmwareRelease {
            model,
            version: "01.02.03.04".into(),
            url: public_catalogue_url(model).unwrap().to_owned(),
        };
        let slot = slot(model);
        *slot.lock().unwrap() = Some(CachedRelease {
            release: release.clone(),
            fetched_at: Instant::now(),
        });

        // A zero timeout makes any real request fail, so a success here can
        // only have come from the cache.
        assert_eq!(
            fetch_public_firmware(model, Duration::ZERO).unwrap(),
            release
        );

        if let Some(expired) =
            Instant::now().checked_sub(PUBLIC_CATALOGUE_TTL + Duration::from_secs(1))
        {
            assert!(!fresh(&CachedRelease {
                release,
                fetched_at: expired,
            }));
        }
    }

    /// An abbreviated copy of the payload every download page embeds: the
    /// whole catalogue keyed by `devModel`, plus release notes that quote the
    /// recommended versions of attached accessories.
    const CATALOGUE: &str = r##"
      <div class="version">Version 01.08.00.00</div><div class="time">2026/05/13</div>
      <svg><path d="M7.19 3.674a.938.938 0 011.323.078l5 5.625a.938.938 0 010 1.246l-5 5.625a.9"/></svg>
      {"devModel":"N1","name":"Bambu Lab A1 mini","desc":"The printer for everyone"}
      {"devModel":"N1","url":"https://public-cdn.bblmw.com/upgrade/device/offline/N1/01.07.02.00/45ea644d25/offline-ota-n1_v01.07.02.00.zip","name":"offline-ota-n1_v01.07.02.00.zip","md5":"0","version":"01.07.02.00","state":1,"id":1,"release_notes_en":"# Version 01.07.02.00"}
      {"devModel":"N1","url":"https://public-cdn.bblmw.com/upgrade/device/offline/N1/01.08.00.00/45ea644d25/offline-ota-n1_v01.08.00.00.zip","name":"offline-ota-n1_v01.08.00.00.zip","md5":"0","version":"01.08.00.00","state":1,"id":2,"release_notes_en":"# Version 01.08.00.00\n\u5907\u6ce8 011.323.078\n  - AMS: 01.00.06.87\n  - AMS 2 Pro: 04.00.21.87"}
      {"devModel":"BL-P001","url":"https://public-cdn.bblmw.com/upgrade/device/offline/BL-P001/01.12.00.00/33fb5e89fd/offline-ota-p001_v01.12.00.00.zip","name":"offline-ota-p001_v01.12.00.00.zip","md5":"0","version":"01.12.00.00","state":1,"id":3}
      {"devModel":"O1C2-V2","url":"https://public-cdn.bblmw.com/upgrade/device/offline/O1C/01.02.00.00/9b1c10b10e/offline-ota-o1c_v01.02.00.00.zip","name":"offline-ota-o1c_v01.02.00.00.zip","md5":"0","version":"01.02.00.00","state":1,"id":4}
    "##;

    /// The A1 mini reported `011.323.078` as its firmware, read out of an
    /// inline SVG path. Every page also embeds the whole catalogue and quotes
    /// accessory versions in its release notes, so reading the page at large
    /// finds numbers larger than the model's own.
    #[test]
    fn reads_only_the_versions_published_for_the_requested_model() {
        assert_eq!(
            parse_public_firmware_version(CATALOGUE, CanonicalModel::A1Mini).as_deref(),
            Some("01.08.00.00")
        );
        assert_eq!(
            parse_public_firmware_version(CATALOGUE, CanonicalModel::X1Carbon).as_deref(),
            Some("01.12.00.00")
        );
    }

    /// The H2C is published under a hardware revision of its code, and the
    /// X2D is keyed `N6` while being served from `/X2D/`, so neither the
    /// exact code nor the URL path works alone.
    #[test]
    fn matches_a_code_that_carries_a_hardware_revision() {
        assert_eq!(
            parse_public_firmware_version(CATALOGUE, CanonicalModel::H2C).as_deref(),
            Some("01.02.00.00")
        );
    }

    /// A model missing from the catalogue takes its family's firmware, which
    /// is how Bambu actually ships: there is no `C12` or `BL-P002` entry
    /// because the P1S runs the P1P's binary and the X1 the X1 Carbon's.
    #[test]
    fn a_model_bambu_does_not_publish_takes_its_family_release() {
        assert_eq!(
            parse_public_firmware_version(CATALOGUE, CanonicalModel::X1).as_deref(),
            Some("01.12.00.00")
        );
    }

    /// A model the page does not publish has no version, rather than the
    /// highest one belonging to some other printer.
    #[test]
    fn a_model_missing_from_the_catalogue_has_no_version() {
        assert_eq!(
            parse_public_firmware_version(CATALOGUE, CanonicalModel::P2S),
            None
        );
        assert_eq!(
            parse_public_firmware_version(CATALOGUE, CanonicalModel::Unknown),
            None
        );
    }
}
