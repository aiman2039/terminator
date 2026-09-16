# Native GitHub releases

`.github/workflows/release.yaml` builds directly on GitHub-hosted machines with
Rust 1.97.1 and the committed Cargo.lock. It does not use Docker.

| Platform | Runner | Release asset |
| --- | --- | --- |
| macOS Apple Silicon | `macos-15` | `terminator-vVERSION-macos.dmg` |
| Linux x86-64 | `ubuntu-24.04` | `terminator-vVERSION-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `ubuntu-24.04-arm` | `terminator-vVERSION-aarch64-unknown-linux-gnu.tar.gz` |

Each package includes the GUI, daemon, hook executable, and the existing
packager's resources and dependency notices. The macOS ZIP is an intermediate
Apple Silicon app; assembly embeds Sparkle and does not lipo extra slices.
Linux archives contain a `terminator/` directory. `SHA256SUMS.txt` covers the
DMG, both Linux archives and `appcast.xml`.
Neovim is not bundled; install Neovim 0.10+ for CodeDiff reviews.

## Build dependencies and caches

Separate macOS and Linux jobs use `.github/actions/release-build` and run
the three native builds concurrently. macOS assembly depends only on the Apple
Silicon job; final draft upload still requires Linux and macOS to succeed. That
macOS job also uploads its development `xtask` binary in a tar archive to retain
executable permissions. Assembly and DMG creation run that exact same-revision
tool, with no additional Cargo/toolchain setup or assembly compiler cache.

The packager builds only `terminator`, `terminator-daemon`, and `terminator-hook`
and their dependencies. It does not compile a second optimized `xtask` binary.
Native builds retain their existing runner/architecture-specific Rust cache keys
for registry and compiler reuse. The legacy `target/package` directory is removed
after restore in the disposable runner checkout. All new package, inspection and
slice outputs live under `RUNNER_TEMP`, outside the compiler cache. Existing
assembly destinations remain protected against overwriting.

Each build uploads `cargo-timings-TARGET` HTML reports for both the tooling and
application builds, including available reports when a build fails. Cargo timing
reports can also be generated locally with `cargo xtask package --timings`.
Restoring a dependency cache does not imply that changed workspace sources or
release links can be skipped. Compare reports and cache restore/save times on
the next hosted run before claiming a speedup.

## Trigger a release

Commit the intended `[workspace.package].version` in `Cargo.toml` and update
`Cargo.lock` as needed. Merge the workflow onto the default branch before using
the Actions manual-run UI.

- **Actions → Release → Run workflow:** select the intended branch or tag. The
  workflow creates `v<workspace version>` at that revision if absent, and builds
  it in the same run. An existing tag must point to that exact commit.

The workflow runs only through this manual trigger. Branch pushes, tag pushes,
and publishing a GitHub release do not start it.

All builds use the resolved commit SHA. Version mismatches fail before building.
To retry an older release manually, select its tag rather than the current branch.
The workflow never moves existing tags. Bump the version for a new revision.

The upload job runs only after all packages and macOS assembly pass their checks.
It creates a draft if needed, uploads the DMG, Linux archives, appcast and
checksums, and verifies the asset inventory. Draft assets can be replaced on
retry; published bytes are never replaced. New versions containing a hyphen are
marked prerelease. Publication additionally requires `SIGNED_UPDATE_VALIDATED`
and the `production-release` environment. See [the update rollout guide](UPDATES.md).

GitHub operations use the built-in `GITHUB_TOKEN`. Tag preparation and publication have
`contents: write`; build jobs have read-only repository access. Repository rules
must allow the token to create release tags and releases. Tag creation stays in
the same workflow because events made with `GITHUB_TOKEN` do not start another
push-triggered workflow. Publishing a release does not start another build;
reruns are serialized for that tag.

## macOS signing and notarization

Configure these repository Actions secrets before a manual release:

- `APPLE_CERTIFICATE_P12_BASE64`: base64-encoded Developer ID Application
  certificate and matching private key exported as a password-protected PKCS#12 file.
- `APPLE_CERTIFICATE_PASSWORD`: that file's export password.
- `APPLE_ID`: the Apple Account email used for notarization.
- `APPLE_APP_SPECIFIC_PASSWORD`: an app-specific password for that account.
- `APPLE_TEAM_ID`: the developer team that issued the certificate.

The assembly job imports the certificate into a temporary keychain after embedding
Sparkle into the Apple Silicon app. It signs the nested Sparkle executables and bundles, the three
application executables and the app with hardened runtime and secure timestamps.
It notarizes/staples the app, creates the DMG, then signs and notarizes/staples the
DMG before generating the signed Sparkle appcast.
Missing credentials, signing errors, or notarization failures block publication.
The temporary keychain and certificate file are removed even after step failure.
No signing secrets are stored in the repository, build cache, or release assets.
Local `cargo xtask package` builds retain ad-hoc signing.

See [Apple's notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)
and [GitHub's certificate setup](https://docs.github.com/en/actions/how-tos/deploy/deploy-to-third-party-platforms/sign-xcode-applications).

## Support boundaries

- Hosted macOS releases are Apple Silicon only. Intel Macs are not in the DMG;
  they can still be built from source and are unvalidated.
- The release workflow requires Developer ID signing and Apple notarization.
  The packager declares macOS 12 as its minimum, but these runner-built releases have not been
  validated on that older OS.
- Linux packages are dynamically linked GNU/Linux builds produced on Ubuntu
  24.04. Use Ubuntu 24.04 or a compatible newer distribution; older glibc systems
  and Alpine/musl are not supported by these assets. Runtime desktop libraries
  and an XDG portal backend are described in the [setup guide](REFERENCE.md#build-and-run).
- Windows needs a port of Unix sockets, process/session handling, and packaging.
  It cannot be added simply by extending the build matrix. BSD, mobile, and web
  targets are not currently supported or validated.
- DEB/RPM, AppImage, and Flatpak are not generated by this workflow.

The workflow checks native target identity, archive contents, executable bits,
macOS code signatures, and Linux executable library resolution. These checks do
not exercise GUI behavior, real PTYs, Neovim, or live providers. Run the project's
test and native validation tasks before tagging; see `docs/VALIDATION.md`.
