---
sidebar_label: Set up the Apple toolchain
---

# Set up the Apple toolchain

First [install the SweetPad CLI on Linux](install.md). The steps below assume a package with Swift Build, the macro plugin, and LLVM tools under `libexec/sweetpad/`.

To build Apple apps, choose **one SDK source**.

- **Use an Xcode download on Linux.** If you have the archive, give it to `sweetpad setup --sdk`. To download and import it in one step, use `sweetpad setup --xcode`. [Use an Xcode archive](#download-xcode-on-linux)
- **Use an existing Xcode installation.** Export the SDKs on the Mac, then give the exported archive to `sweetpad setup --sdk` on Linux. [Export the SDKs on a Mac](#export-it-on-your-mac)

Both routes include the **import** step. Setup extracts the headers, frameworks, and libraries from your chosen archive and installs them as a Darwin SDK bundle. Later builds reuse that bundle.

Keep the archive, SDK, and Apple session private. Use them under the applicable license. [Apple’s Xcode agreement](https://www.apple.com/legal/sla/docs/xcode.pdf)

## What setup installs

| Component | What `sweetpad setup` does on Linux |
| --- | --- |
| System packages | Checks for host build tools, certificates, Git, Python, and the system libraries used by Swift and Swift Build. If packages are missing, setup prints the exact install command for your distribution. Add `--install-system-packages` to let setup install them with APT or pacman. |
| Linux Swift compiler | Reuses Swift 6.4.2 from `PATH`, swiftly, or a previous setup. Otherwise, it downloads Swift from swift.org, checks its signature, and installs it in `~/swift/swift-6.4.2-RELEASE`. It also checks for Swift’s `llvm-ar` archiver. |
| Apple SDKs | Imports your Mac export or Xcode archive into `~/.swiftpm/swift-sdks/darwin.artifactbundle`. Setup reuses a matching bundle on subsequent runs. |
| Shell environment | Writes `~/.config/sweetpad/linux-env.sh`. SweetPad finds the default compiler and SDK automatically. Source the file to run `swift` directly from your shell. |

After the import, setup builds an unsigned sample iPhone app. This checks that the compiler can use the imported headers and that the linker can produce an iOS executable. It requires neither signing credentials nor a phone.
To prepare just the host packages and Swift, run `sweetpad setup --no-sdk`. This skips SDK import and the sample build.

## Supported hosts

| Distribution | Architectures | System packages |
| --- | --- | --- |
| Ubuntu 24.04 | x64, ARM64 | APT |
| Debian 12 or 13 | x64, ARM64 | APT |
| Arch Linux | x64 | pacman |

Every supported host can build ARM64 iPhone apps. On other glibc distributions, setup warns and tries Ubuntu’s Swift toolchain with the packages already installed. On Alpine, use an Ubuntu or Debian container.

On Ubuntu or Debian, install the CA certificates first.

```bash
sudo apt-get install ca-certificates
```

On Arch, update the system before installing the certificates.

```bash
sudo pacman -Syu
sudo pacman -S --needed ca-certificates
```

## Get the SDK

### Use an Xcode archive on Linux {#download-xcode-on-linux}

**If you already have the Xcode archive**, pass it directly to setup.

```bash
sweetpad setup --sdk "$HOME/Downloads/Xcode_27.2.xip"
```

Setup also accepts an `.aea` archive. For an archive without an embedded key, add `--aea-key-file`. See `sweetpad setup --help` for key formats.

**If you need to download Xcode**, sign in to Apple and give setup the release number.

```bash
sweetpad auth login --apple-id you@example.com
sweetpad setup --xcode 27.2
```

For a beta, use the release name, such as `--xcode '27.3 beta'`. Setup downloads the archive and imports its SDKs in the same run. Apps can build from this source. Doctor warns about the localized App Intents phrases that require a Mac export.

SweetPad asks for your password and two-factor code. It saves the session in `~/.local/state/sweetpad/apple-session.json`. Terminal sign-in requires an account that supports password and two-factor code authentication. Accounts using security keys, federated login, or legacy two-step verification cannot sign in this way.

### Export it on your Mac

On the Mac with Xcode installed, export the SDKs. SweetPad writes a checksum beside the archive.

```bash
sweetpad sdk export --xcode /Applications/Xcode.app --output-file "$HOME/apple-sdk.tar.gz"
```

For a separate Linux host, copy both files from the Mac.

```bash
scp "$HOME/apple-sdk.tar.gz" "$HOME/apple-sdk.tar.gz.sha256" linux-host:~/
```

On Linux, pass the exported archive to setup.

```bash
sweetpad setup --sdk "$HOME/apple-sdk.tar.gz"
```

An OrbStack VM can read the archive directly from the Mac. Skip the copy and pass the Mac path, such as `/mnt/mac/Users/YOUR_NAME/apple-sdk.tar.gz`. Keep the checksum beside the archive.

The export includes SDKs, runtime libraries, test runners, and localized App Intents phrases. Use the Mac export for localized shortcut training. Add `--dry-run` to the export command to see how much space it needs.

## Setup options

- **Install system packages.** Add `--install-system-packages` to install missing dependencies. For non-interactive runs that need sudo, run `sudo -v` first or configure passwordless sudo. This also applies to `--json` and redirected stderr. Use `--skip-system-packages` to continue with the packages already installed.
- **Switch SDKs.** Add `--replace-sdk` when setup reports that a different SDK is already installed. Setup keeps the old SDK until the new import succeeds.
- **Change install paths.** Use `--sdk-path` for the SDK and `--env-file` for the shell environment file. With a custom SDK path, source the environment file to set `SWEETPAD_DARWIN_SDK`.
- **Import only.** If Swift 6.4.2 is already installed, `sweetpad sdk import` can import an archive on its own.

### Containers

The current SweetPad image includes the pinned Linux Swift compiler and host packages. It uses Bash for `/bin/sh`, which lets Xcode shell-script phases use Bash syntax with Docker’s default capabilities. For a custom build image, install Bash and add `RUN ln -sf bash /bin/sh` to the Dockerfile. On native Linux hosts with a different `/bin/sh`, the packaged bubblewrap helper maps it to Bash during builds.

SweetPad reads Xcode’s cpio archives directly. It supplies the ZIP, file identification, and local rsync operations that SwiftPM, Flutter, and CocoaPods use. Build scripts can call system utilities on `PATH` for additional options.

### Use a saved Apple session

To keep a separate session for local downloads, use the same file at login and setup.

```bash
sweetpad auth login --apple-id you@example.com --session-file "$HOME/private/apple-session.json"
sweetpad setup --xcode 27.2 --session-file "$HOME/private/apple-session.json"
```

Keep the session file private with mode `600`. You can also set its path with `SWEETPAD_APPLE_SESSION_FILE`. When Apple expires the session, sign in again. To delete a session file, use `sweetpad auth logout`.

## Read the doctor report

```bash
sweetpad doctor --target ios
```

Doctor checks the host, Swift, Swift Build and its patches, the macro plugin, the SDK, the linker, and disk space. Signing and the iPhone adapter are optional for unsigned builds. For Mac apps, use `--target macos`. In JSON output, read `data.summary.problems` and each check’s `status` and `hint`.

To install and run an app on a phone, see [Connect an iPhone](ubuntu-usb.md) and [Set up signing](signing-and-testing.md).
