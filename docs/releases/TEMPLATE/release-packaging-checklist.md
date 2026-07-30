# Release packaging checklist

Release tag: <tag>
Commit: <commit SHA>
Workflow: <release workflow URL>

## Automated checks

- [ ] `make release-qualification` passed for this commit.
- [ ] The release workflow built and retained every required package artifact.

## External/manual evidence

- [ ] Each target installer was installed, launched, and uninstalled on its target OS; record platform, artifact, person, date, and result.
- [ ] The packaged `polimero version --output json` command succeeded on each target OS; record platform, person, date, and result.
- [ ] A configured Moonraker printer reported status and accepted one non-destructive control; record device, person, date, and result.
- [ ] Emergency stop behavior and unsupported-capability errors were observed on that printer; record device, person, date, and result.
