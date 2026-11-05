# Homebrew installation

Last-updated: 2026-11-04

The macOS cask installs the published nightly `Melt.app` on Apple Silicon or
Intel and links its executable as `melt` in Homebrew's `bin` directory. No Rust
or Bun build is needed. Homebrew installs the matching runtime dependency:
`openssl@4` on Apple Silicon and `openssl@3` on Intel. Linux users should use
the `deb` or `rpm` installers from
[GitHub Releases](https://github.com/polimero-app/melt/releases).

## Install and run

With [Homebrew](https://brew.sh/) installed:

```sh
brew tap polimero-app/melt https://github.com/polimero-app/melt.git
brew install --cask polimero-app/melt/melt
open -a Melt
melt version --output json
```

The explicit tap URL is required because the cask lives in this source
repository rather than a separate `homebrew-melt` repository. The fully
qualified cask name selects this project's package.

Running `melt` without arguments opens the desktop; command arguments select
the headless CLI. The app uses the same configuration and keychain as a manual
installation. Quit an existing Melt instance before upgrading or removing it.

## Upgrade or remove

```sh
brew update
brew upgrade --cask polimero-app/melt/melt
```

To remove the app and CLI link:

```sh
brew uninstall --cask polimero-app/melt/melt
brew untap polimero-app/melt
```

Uninstall preserves printer profiles, preferences, and keychain credentials.
See [configuration](configuration.md) for their locations.

## Maintaining the cask

[`Casks/melt.rb`](../Casks/melt.rb) pins a published nightly tag, both DMG
SHA-256 checksums, and the bundle's CLI path. It uses the full release tag as
the Homebrew version, including any same-day retry suffix, while the DMG
filename carries Tauri's normalized date version.

After all nightly package jobs succeed, the publish job makes the release
public, reads its GitHub API asset digests, regenerates the cask, and commits
only `Casks/melt.rb` to the default branch using the workflow's `GITHUB_TOKEN`.
The repository must allow that token to push this update under its branch
rules. If a push fails, rerun the failed publish job after resolving the branch
rule or push conflict. Installer cleanup runs only after the cask update
succeeds, keeping the previous pinned installer available when an update fails.
The nightly plan ignores cask-only changes when deciding whether to package
again.

To regenerate the cask manually from a published nightly:

```sh
gh api repos/polimero-app/melt/releases/tags/2026.11.04 > /tmp/melt-release.json
python3 scripts/update_homebrew_cask.py /tmp/melt-release.json
make homebrew-check
```

The generator rejects draft releases, missing architectures or checksums,
unexpected download URLs, duplicate installers, and mismatched bundle versions.
Review and commit the generated cask. Tagged `v*` packaging does not publish
GitHub release assets and does not update the cask.

CI validates release metadata handling and, on both macOS architectures,
checks cask style, installs the published package, runs
`melt version --output json`, and uninstalls it. To exercise a cask change
locally on macOS, copy it into the installed tap before running those commands:

```sh
cp Casks/melt.rb "$(brew --repository polimero-app/melt)/Casks/melt.rb"
brew style --cask polimero-app/melt/melt
brew reinstall --cask polimero-app/melt/melt
melt version --output json
```

The Homebrew cask follows the nightly prerelease channel. Installation through
Homebrew does not add application signing, an in-app updater, or manual
installer/hardware qualification; see [release qualification](release-qualification.md).
