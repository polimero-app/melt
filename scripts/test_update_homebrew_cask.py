"""Release metadata must be complete and trusted before updating the tap."""

import unittest

from update_homebrew_cask import render_cask


def release_metadata(tag="2026.11.04"):
    return {
        "tag_name": tag,
        "draft": False,
        "assets": [
            {
                "name": f"Melt_2026.11.4_{arch}.dmg",
                "browser_download_url": f"https://github.com/polimero-app/melt/releases/download/{tag}/Melt_2026.11.4_{arch}.dmg",
                "digest": f"sha256:{digest * 64}",
            }
            for arch, digest in [("aarch64", "a"), ("x86_64", "b")]
        ],
    }


def change_installer_version(release):
    asset = release["assets"][0]
    asset["name"] = asset["name"].replace("2026.11.4", "2026.11.5")
    asset["browser_download_url"] = asset["browser_download_url"].replace("2026.11.4", "2026.11.5")


class CaskReleaseTests(unittest.TestCase):
    def test_nightly_and_same_day_retry_use_exact_release_tag(self):
        for tag in ["2026.11.04", "2026.11.04.2"]:
            with self.subTest(tag=tag):
                cask = render_cask(release_metadata(tag))
                self.assertIn(f'version "{tag}"', cask)
                self.assertIn('Melt_2026.11.4_#{arch}.dmg', cask)
                self.assertIn(f'arm:   "{"a" * 64}"', cask)
                self.assertIn(f'intel: "{"b" * 64}"', cask)
                self.assertIn('binary "#{appdir}/Melt.app/Contents/MacOS/melt"', cask)

    def test_rejects_incomplete_or_untrusted_release(self):
        mutations = {
            "draft": lambda release: release.update(draft=True),
            "invalid tag": lambda release: release.update(tag_name='2026.11.04#{unsafe}'),
            "missing architecture": lambda release: release["assets"].pop(),
            "duplicate architecture": lambda release: release["assets"].append(release["assets"][0]),
            "missing digest": lambda release: release["assets"][0].pop("digest"),
            "null digest": lambda release: release["assets"][0].update(digest=None),
            "invalid digest": lambda release: release["assets"][0].update(digest="sha256:bad"),
            "foreign URL": lambda release: release["assets"][0].update(browser_download_url="https://example.com/melt.dmg"),
            "wrong version": change_installer_version,
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label):
                release = release_metadata()
                mutate(release)
                with self.assertRaises(ValueError):
                    render_cask(release)

    def test_ignores_other_platform_installers(self):
        release = release_metadata()
        release["assets"].append({"name": "Melt_2026.11.4_amd64.deb"})
        self.assertEqual(render_cask(release), render_cask(release_metadata()))


if __name__ == "__main__":
    unittest.main()
