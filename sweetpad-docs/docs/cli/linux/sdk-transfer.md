---
sidebar_label: Set up the Apple toolchain
---

# Set up the Apple toolchain

Linux builds need Apple's SDKs from Xcode, plus a Linux Swift toolchain and Swift Build.
`sweetpad setup` installs all of them from one source: an SDK archive exported on your Mac, or an Xcode download.

Use Linux in a VM on Apple hardware, under [Apple's Xcode agreement](https://www.apple.com/legal/sla/docs/xcode.pdf).
Keep the archive, the extracted SDK, and your Apple session private, and out of public artifacts and container layers.

## Supported hosts

| Distribution | Architectures | System packages |
| --- | --- | --- |
| Ubuntu 24.04 | x64, ARM64 | APT |
| Debian 12 or 13 | x64, ARM64 | APT |
| Arch Linux | x64 | pacman |

Every host builds ARM64 iPhone apps.
On other distributions, setup warns and tries Ubuntu's Swift toolchain with the packages already installed.
Setup refuses Alpine, because the Apple toolchain needs glibc. Run it in an Ubuntu or Debian container there.

Install the prerequisites first. On Ubuntu or Debian:

```bash
sudo apt-get install ca-certificates
```

On Arch, update the whole system first, because Arch doesn't support partial upgrades:

```bash
sudo pacman -Syu
sudo pacman -S --needed ca-certificates
```

## Get the SDK

### Export it on your Mac

```bash
sweetpad sdk export --xcode /Applications/Xcode.app --output-file "$HOME/apple-sdk.tar.gz"
scp "$HOME/apple-sdk.tar.gz" "$HOME/apple-sdk.tar.gz.sha256" linux-host:~/
```

The archive holds the SDKs, runtime libraries, test runners, and localized App Intents phrases, but none of Xcode's programs.
Add `--dry-run` to see how much space it needs.
An OrbStack machine reads the Mac's files at the same paths, so it doesn't need the copy.

### Download Xcode on Linux

Sign in once from a terminal:

```bash
sweetpad auth login --apple-id you@example.com
```

SweetPad asks for your password and two-factor code, and saves only the session, in `~/.local/state/sweetpad/apple-session.json`.
Treat that file as a credential.
Accounts that sign in with a security key, federated login, or legacy two-step verification can't sign in from a terminal.

Setup then downloads the release you name with `--xcode`.
You can also pass a `.xip` or `.aea` file you downloaded yourself to `--sdk`.
For an `.aea` file without an embedded key, add `--aea-key-file`; `sweetpad setup --help` lists the key formats.

A download doesn't include the localized App Intents phrases that a Mac export carries.
Apps still build, `doctor` warns, and localized shortcut training needs an export from a Mac.

## Run setup

```bash
sweetpad setup --sdk "$HOME/apple-sdk.tar.gz"
sweetpad doctor --target ios
```

Use `--xcode 27.2` in place of `--sdk` to download that release; a beta needs its name, as in `--xcode '27.3 beta'`.

Setup installs the system packages and the pinned Swift toolchain.
When Swift 6.4.2 is already on `PATH` or installed with swiftly, setup uses it; otherwise it downloads the release from swift.org and checks its signature.
It then imports the SDK and builds a sample iPhone app to check the result.
Swift Build, the macro plugin and the LLVM linker ship prebuilt in the SweetPad package.

sweetpad finds the toolchain and the SDK on its own, so builds need nothing on `PATH`.
Setup also writes `~/.config/sweetpad/linux-env.sh`, which puts Swift on `PATH` for running `swift` by hand.

Setup skips every step whose result is already installed and current, so it's safe to run again.
To switch to another archive or Xcode release, add `--replace-sdk`. The old SDK stays until the new one imports.

With `--json` or `--non-interactive`, or when stderr isn't a terminal, sudo can't ask for a password.
Configure passwordless sudo or run `sudo -v` first.
`--skip-system-packages` uses the packages already installed instead.

### Containers

Ubuntu and Debian builds run Xcode's scripts through bubblewrap, so that `/bin/sh` is Bash.
Docker must allow mount namespaces, for example with `--cap-add SYS_ADMIN --security-opt seccomp=unconfined`.
Where that isn't allowed, use a VM. `sweetpad doctor` checks it.

### CI with a saved Apple session

Create a separate session file in an interactive terminal:

```bash
sweetpad auth login --apple-id you@example.com \
  --session-file "$HOME/private/apple-session.json"
```

Store it in your CI secret store, restore it with mode `600`, and pass it to setup:

```bash
sweetpad setup --xcode 27.2 \
  --session-file "$RUNNER_TEMP/apple-session.json" --json --non-interactive
```

`SWEETPAD_APPLE_SESSION_FILE` sets the same path.
Apple decides when a session expires. Sign in again to replace the secret, and use `sweetpad auth logout` to delete a session file.

### Other locations

`--sdk-path` and `--env-file` put the SDK and the environment file somewhere else, for example one per CI runner.
An SDK outside the default location needs `SWEETPAD_DARWIN_SDK` set to it, which the environment file does.
With Swift 6.4.2 installed, `sweetpad sdk import` imports an archive without the rest of setup.
`sweetpad setup --no-sdk` installs the host packages and Swift alone, for a machine that builds no Apple apps yet.

## Read the doctor report

`sweetpad doctor --target ios` checks the host, Swift, Swift Build and its patches, the macro plugin, the SDK, the linker, and disk space.
Signing and the iPhone adapter appear separately and don't block unsigned builds.
In JSON, read `data.summary.problems` and each check's `status` and `hint`.
Use `--target macos` for Mac apps.

Next, [connect an iPhone](ubuntu-usb.md) and [set up signing](signing-and-testing.md).
