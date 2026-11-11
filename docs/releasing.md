# Releasing

This document is the checklist for a release. Only the maintainer makes a release.
For the definition of each term, see [glossary.md](glossary.md).

The release workflow `.github/workflows/release.yml` never publishes a release. It makes a draft pre-release.
A person tests the draft on a real pad. The person publishes the draft only when the test is correct.

## What a release contains

| Asset | Contents |
|---|---|
| `kb16-mixer-<version>-windows-x64.zip` | `kb16-mixer.exe`, `firmware/kb16-mixer-firmware.bin`, `flash.ps1`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, `README.txt` |
| `SHA256SUMS` | The SHA-256 hash of the zip, of the firmware image and of the exe |
| Build attestation | A signed record from GitHub. It shows that the workflow made the zip from the tagged commit. |

`scripts/package-release.py` makes the zip and `SHA256SUMS`.
The script refuses a firmware image with a bad vector table or a size above `0x1D400` bytes. It also refuses an exe without the `MZ` header.
The zip is reproducible. The same commit and the same Python build give the same bytes.

## Checklist

### 1. Prepare the version

1. Make sure that all CI checks on `main` are green.
2. Set the new version in each of these four files:
   - `app/Cargo.toml`
   - `firmware/Cargo.toml`
   - `pad-core/Cargo.toml`
   - `protocol/Cargo.toml`
3. Run `cargo check` in each of the four crate folders. Cargo then writes the new versions into each `Cargo.lock`.
4. In `CHANGELOG.md`, move the entries of the "Unreleased" section to a new section.
5. Give the new section a heading with the version and the date, for example `## [0.1.0] - 2026-11-20`.
6. Open a pull request with these changes.
7. Merge the pull request when CI is green.

The firmware sends its version as the USB release number. Thus the major version must be 0 to 99. The minor version and the patch version must be 0 to 9.

### 2. Make the draft

1. Update your local `main` branch.
2. Make the tag on the merge commit: `git tag v0.1.0`.
3. Push the tag: `git push origin v0.1.0`.
4. Open the **Actions** tab on GitHub. Find the "Release" run for the tag.
5. Wait until all jobs of the run are green.

The workflow does these checks before it builds:

- The version of the tag is the version in each of the four `Cargo.toml` files.
- `CHANGELOG.md` has a section for the version. The heading has a date.
- The tagged commit is on `main`.

The workflow then builds the firmware image and the app with `--locked`. The build scripts remove the paths of the build machine from the binaries.
The workflow makes the zip, `SHA256SUMS` and the attestation. It then makes a draft pre-release with the changelog section as the notes.

### 3. Test the draft

1. Open the **Releases** page. Find the draft.
2. Download the zip and `SHA256SUMS` from the draft.
3. Open PowerShell in your download folder.
4. Get the hash of the zip: `Get-FileHash .\kb16-mixer-0.1.0-windows-x64.zip -Algorithm SHA256`.
5. Compare the hash with the line for the zip in `SHA256SUMS`. The two values must be the same.
6. Check the attestation: `gh attestation verify .\kb16-mixer-0.1.0-windows-x64.zip --repo Dokuganrryu/kb16-mixer`.
7. Unzip the zip to a new folder.
8. Put a test pad into the bootloader. See [install.md](install.md).
9. Flash the firmware from the zip with `flash.ps1`. Use only the files from the draft.
10. Unplug the pad. Plug it in again.
11. Run `kb16-mixer.exe --check` from the unzipped folder.
12. Make sure that the `pad` line shows `OK` and the new firmware version.
13. Make sure that the last line is `Result: OK`.
14. Do the tests in the table of [testing.md](testing.md#verified-on-the-real-pad) that apply to the change.
15. Write the results, the date and the version into that table. Merge that change through a pull request.

Do not publish the draft if one of these steps fails.

### 4. Publish

1. Open the draft on the **Releases** page.
2. Keep the mark "Set as a pre-release" for each 0.x version.
3. Click **Publish release**.

## Repair a bad release

A bad draft is not public. A published release is public. The repair is different for each.

For a bad draft:

1. Delete the draft on the **Releases** page.
2. Delete the tag: `git push origin --delete v0.1.0`, then `git tag -d v0.1.0`.
3. Fix the problem through a pull request.
4. Make the tag again on the new commit on `main`.

For a bad published release:

1. Do not move or delete the published tag. People and scripts can already use it.
2. Edit the release notes. Write what is wrong at the top.
3. Fix the problem through a pull request.
4. Make a new release with a new patch version, for example `v0.1.1`.

## Test the workflow without a release

You can start the workflow by hand. Use the **Run workflow** button of the "Release" workflow in the **Actions** tab.
The option `dry_run` is on by default. A run by hand builds and packages the files, and keeps them as workflow artifacts.
With `dry_run` on, the run makes no attestation. A run by hand never makes a release, because a release needs a tag.

GitHub shows the **Run workflow** button only for a workflow file that is on `main`.

CI also packages a zip in each run, with the binaries of that run (the job "Release zip (dry run)").
