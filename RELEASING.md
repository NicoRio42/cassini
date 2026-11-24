# Releasing Cassini

Releases are built by [dist](https://axodotdev.github.io/cargo-dist/). Its configuration is [dist-workspace.toml](dist-workspace.toml), and it generates [.github/workflows/release.yml](.github/workflows/release.yml). Don't edit that file by hand: change the configuration, then run `dist generate` (install dist with `cargo install cargo-dist --version <cargo-dist-version> --locked`). The `plan` job fails on pull requests when the workflow is out of date.

## Making a release

The version in `Cargo.toml` is the source of truth: changing it on `main` makes a release.

1. Bump `version` in `Cargo.toml`, run `cargo build --locked` so `Cargo.lock` follows, and merge to `main`. A version with a suffix, like `0.18.0-rc.1`, makes a prerelease.
2. [release-on-version-bump.yml](.github/workflows/release-on-version-bump.yml) runs on every push to `main` that changes `Cargo.toml`. If there is no tag for the version yet, it starts the `Release` workflow with the version as the tag; otherwise it does nothing, so editing `Cargo.toml` without changing the version is harmless.
3. The `Release` workflow then:
   - builds the binaries for `x86_64-unknown-linux-gnu` (on `ubuntu-22.04`, so it needs glibc 2.35 or later), `aarch64-apple-darwin` (`macos-15`), `x86_64-apple-darwin` (`macos-15-intel`) and `x86_64-pc-windows-msvc` (`windows-2022`), failing if `Cargo.lock` is out of date;
   - creates the tag on the commit it built, and the GitHub Release with the archives, their SHA-256 checksums, and the `cassini-installer.sh` and `cassini-installer.ps1` install scripts;
   - for a non-prerelease, pushes the Docker image `<DOCKER_USERNAME>/cassini:<version>` and `:latest` to Docker Hub ([publish-docker.yml](.github/workflows/publish-docker.yml)), using the `DOCKER_USERNAME` and `DOCKER_PASSWORD` repository secrets.
4. Edit the release notes on GitHub. dist writes them from a `CHANGELOG.md` when there is one.

Pushing a tag by hand doesn't start a release. To release without a push to `main` (for example to retry a failed release), run the `Release` workflow from the Actions tab with the version as the tag; `dry-run` builds without publishing. The tag must equal the version in `Cargo.toml` at the commit it runs on.

If a second push with the same new version lands before the first release has created its tag, both start a release, and the second fails when it creates the GitHub Release.

Nothing runs the binaries in CI: test a release candidate by hand (a version like `0.18.0-rc.1` publishes it without the Docker image).

## Runners

GitHub retires runner images regularly: `macos-13` and `macos-14` stopped running the old release workflows. When a runner goes away, change it in `[dist.github-custom-runners]`, then run `dist generate`. `macos-15-intel` is the last Intel macOS image; after it, `x86_64-apple-darwin` has to be cross-compiled on an arm64 runner.

The Windows binary links the Visual C++ runtime dynamically (`msvc-crt-static = false`), because skia-binaries has no prebuilt Skia for the static runtime. It needs `VCRUNTIME140.dll`, which most Windows machines have; the installation guide says what to do otherwise.

## Code signing (not set up)

The binaries are not signed. Signing needs accounts that only the project owner can create; dist supports both kinds once the secrets exist.

- **macOS:** Gatekeeper blocks unsigned executables that a browser downloaded (the quarantine attribute). The install script downloads with `curl`, which doesn't set that attribute, so command-line installs work. The installation guide gives the `xattr` command for browser downloads. Signing and notarizing needs an Apple Developer Program membership (99 USD a year) and a "Developer ID Application" certificate. dist can then sign the binaries: set `macos-sign = true` in `dist-workspace.toml` and add the `CODESIGN_CERTIFICATE`, `CODESIGN_CERTIFICATE_PASSWORD` and `CODESIGN_IDENTITY` secrets (see `macos-sign` in the [dist config reference](https://axodotdev.github.io/cargo-dist/book/reference/config.html)). dist doesn't notarize: that would be an extra `xcrun notarytool submit` step on the macOS archives, with an App Store Connect API key.
- **Windows:** unsigned executables work, but SmartScreen warns about them when they come from a browser download, until they gain reputation. Signing needs a code signing certificate, for example through SSL.com eSigner (`ssldotcom-windows-sign = "prod"`) or Azure Artifact Signing (`azure-windows-sign`), with that service's secrets ([dist docs](https://axodotdev.github.io/cargo-dist/book/supplychain-security/signing/windows.html)).

## Homebrew (not set up)

dist can also publish a Homebrew formula. That needs a tap repository (for example `NicoRio42/homebrew-tap`) and a `HOMEBREW_TAP_TOKEN` secret that can push to it. Then add `"homebrew"` to `installers`, `tap = "NicoRio42/homebrew-tap"` and `"homebrew"` to `publish-jobs`, and run `dist generate`.
