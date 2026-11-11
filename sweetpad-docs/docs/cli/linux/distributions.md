---
sidebar_label: Linux distributions
---

# Debian and Arch setup

The setup changes on this page require a development build after CLI 0.1.21.
Released CLI packages run on x64 and ARM64 Linux hosts.
Apple app builds also need compatible Swift tools and your private SDK export.

| Distribution | Automatic setup | Host compiler |
| --- | --- | --- |
| Ubuntu 24.04, x64 or ARM64 | APT | Signed Ubuntu 24.04 Swift archive. |
| Debian 12 or 13, x64 or ARM64 | APT | Signed Swift archive for that Debian release. |
| Arch Linux, x64 | pacman | Signed Ubuntu Swift archive with a local ncurses adjustment. |

Arch Linux ARM is a separate port and needs separate validation.
Other distributions produce a doctor warning. Required tools must still pass their checks.

## Prepare Debian

Follow [Install the CLI](install.md). APT, local Debian packages, and user-managed installs work on Debian.
Install the prerequisites for setup:

```bash
sudo apt-get update
sudo apt-get install ca-certificates python3 bash
```

Export and transfer the private SDK files with the [SDK export guide](sdk-transfer.md).
Then configure the build tools:

```bash
sweetpad setup --sdk "$HOME/apple-sdk.tar.gz"
source "$HOME/.config/sweetpad/linux-env.sh"
sweetpad doctor --target ios
```

Setup selects Debian's Swift archive and the distribution's default GCC and LLVM packages.
It does not require Ubuntu's GCC 13 or LLVM 18 package names.

## Prepare Arch

Update the system before setup. Arch does not support partial upgrades:

```bash
sudo pacman -Syu
sudo pacman -S --needed ca-certificates python bash
```

Install Sweetpad with [Packslip, Mise, or the tarball](install.md).
Then use the same SDK transfer and setup commands shown for Debian.

Setup installs dependencies with `pacman -S --needed` and does not update the whole system automatically.
It uses `libxml2-legacy` and adjusts ncurses dependencies inside the downloaded Swift toolchain.
It does not replace system libraries.
This follows the compatibility approach in [Arch's Swift package recipe](https://aur.archlinux.org/cgit/aur.git/tree/PKGBUILD?h=swift-bin).

## Manage system packages yourself

If your administrator supplies the host dependencies, skip package installation:

```bash
sweetpad setup --sdk "$HOME/apple-sdk.tar.gz" --skip-system-packages
```

This option still downloads and checks the pinned tools, imports the SDK, and builds the sample.
It does not skip toolchain validation.
On other distributions, setup warns, skips automatic package installation, and tries the Ubuntu 24.04 Swift archive with your existing dependencies.
Missing or incompatible libraries still cause setup to fail with diagnostics.

For an incompatible host, run the build tools in an Ubuntu or Debian container.
The [GHCR image](install.md#ghcr-container-image) supplies the CLI for your build image.
Keep Apple SDKs and signing files private.

Ubuntu and Debian builds use bubblewrap to give Xcode scripts Bash at `/bin/sh`.
Docker must allow mount namespaces. The private validation containers use `--cap-add SYS_ADMIN --security-opt seccomp=unconfined`.
If your container service forbids these permissions, use a Linux VM.
`sweetpad doctor` checks this shell mapping and reports namespace restrictions before app compilation.

## Validation coverage

Public CI installs host dependencies and verifies Swift and xtool on fresh containers.
It also compiles and runs a Foundation program and a Swift package.
These checks use no Apple SDKs and do not prove that an iPhone app builds or runs.
Private SDK checks cover app compilation separately.

For iPhone connections, continue with [USB setup](ubuntu-usb.md#connect-the-iphone).
Debian uses `apt-get install python3-venv usbmuxd`; Arch uses `pacman -S usbmuxd` and its system Python for virtual environments.
Device validation on a new distribution is separate from compiler validation.
