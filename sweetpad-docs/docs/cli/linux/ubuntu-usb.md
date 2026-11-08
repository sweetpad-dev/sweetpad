---
sidebar_label: Ubuntu and USB setup
---

# Ubuntu setup and an iPhone over USB

This guide starts with Ubuntu 24.04 ARM64 and ends with a signed app on an iPhone.
Use a Linux VM on Apple hardware for the validated setup.

You need your Xcode SDKs, an Apple Development identity, a provisioning profile, and a USB connection to the phone.
Allocate space for Xcode, extracted SDKs, compiler tools, dependencies, and build products.

## Install the release

Install the runtime and setup utilities:

```bash
sudo apt-get update
sudo apt-get install ca-certificates curl libssl3t64 libgcc-s1 python3-venv usbmuxd
```

Download and verify the CLI archive:

```bash
CLI_VERSION=0.1.18
mkdir -p "$HOME/downloads/sweetpad-$CLI_VERSION"
cd "$HOME/downloads/sweetpad-$CLI_VERSION"
curl -fLO "https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION/sweetpad-cli-$CLI_VERSION-linux-arm64.tar.gz"
curl -fLO "https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION/sweetpad-cli-$CLI_VERSION-linux-arm64.tar.gz.sha256"
sha256sum -c "sweetpad-cli-$CLI_VERSION-linux-arm64.tar.gz.sha256"
tar -xzf "sweetpad-cli-$CLI_VERSION-linux-arm64.tar.gz"
bash install.sh
export PATH="$HOME/.local/bin:$PATH"
sweetpad --version
```

The installer defaults to your local user prefix. It also accepts `--prefix /absolute/path`.
The archive includes a sample project and public toolchain setup sources.
Installing the CLI requires no Rust compiler or SweetPad checkout.

## Prepare the Apple toolchain

Provide your own Xcode copy at a path visible to Ubuntu.
The examples use Apple hardware; obtain and use SDK files under [Apple's Xcode agreement](https://www.apple.com/legal/sla/docs/xcode.pdf).

Run the installed setup script:

```bash
SWEETPAD_SKIP_RUST=1 XCODE_APP=/path/to/Xcode.app \
  bash "$HOME/.local/share/sweetpad/setup/sweetpad-lib/ci/linux-setup.sh"
export PATH="$HOME/.local/bin:$HOME/bin:$HOME/swift/swift-6.4.2-RELEASE/usr/bin:$PATH"
```

The script installs the pinned Swift toolchain, Swift Build, macro plugin, and supporting Linux tools.
It imports SDK data from the Xcode copy that you supply.
Existing setup results can be reused. `SWEETPAD_REFRESH_SDK=1` refreshes a stale extracted SDK.

Persist the PATH entries in your shell configuration after setup.
Keep the Linux Swift toolchain ahead of other Swift installations.

### OrbStack on a Mac

An OrbStack machine can see the Mac's files at the same paths.
The repository setup script can create and provision the Ubuntu machine from macOS:

```bash
git clone --branch cli-v0.1.18 --depth 1 https://github.com/sweetpad-dev/sweetpad.git
cd sweetpad
SWEETPAD_SKIP_RUST=1 bash sweetpad-lib/ci/linux-setup.sh
orb -m sweetpad-linux
```

Install the Linux archive inside that machine using the release steps in this guide.
Mac execution of the setup script also exports localized App Intents resources into the private SDK.

## Build without a phone

Use the packaged sample to confirm the compiler and SDK setup:

```bash
sweetpad build \
  --project "$HOME/.local/share/sweetpad/examples/LinuxApp/LinuxApp.xcodeproj" \
  --scheme LinuxApp --destination 'generic/platform=iOS Simulator' \
  --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/LinuxApp" ARCHS=arm64 CODE_SIGNING_ALLOWED=NO
```

This creates simulator products. Running them still requires a Mac.
For your own app, select its project or workspace and scheme.

## Connect the iPhone

Enable Developer Mode on the iPhone. Connect it with a data-capable USB cable. Then unlock it.
Trust the Linux host when the phone asks.

### Pass USB into a VM

On the Mac, select and attach only the intended iPhone:

```bash
orb usb list
orb usb attach IPHONE_USB_ID -m sweetpad-linux
```

The VM owns that USB connection until detachment.
On a direct Ubuntu installation, the cable already belongs to the Linux host.

### Install the pinned adapter

Run these commands inside Ubuntu:

```bash
python3 -m venv "$HOME/.local/share/sweetpad/device-tools"
"$HOME/.local/share/sweetpad/device-tools/bin/python" -m pip install pymobiledevice3==11.50.0
export SWEETPAD_DEVICE_PYTHON="$HOME/.local/share/sweetpad/device-tools/bin/python"
sudo systemctl start usbmuxd
"$SWEETPAD_DEVICE_PYTHON" -m pymobiledevice3 lockdown pair
sweetpad device list --json --non-interactive
```

Accept the Trust prompt on the phone during pairing.
Use the device UDID from the list in the remaining commands.
The SweetPad adapter expects USB trust to exist; listing a device does not create that trust.

## Build and install the iPhone app

Run these commands from your app's project directory.
Replace the project, scheme, device ID, and product names with your own values.

Configure the identity and profile using [the signing guide](signing-and-testing.md#configure-development-signing).
Then build for the device SDK:

```bash
sweetpad build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/MyApp" ARCHS=arm64

sweetpad device install "$HOME/builds/MyApp/Build/Products/Debug-iphoneos/MyApp.app" \
  --device IPHONE_UDID --json --non-interactive

sweetpad app launch --project MyApp.xcodeproj --scheme MyApp \
  --device-id IPHONE_UDID --derived-data-path "$HOME/builds/MyApp" \
  --json --non-interactive
```

If your project changes the product name or configuration, use the product path from the build output.
Installation accepts an existing signed app or IPA; it does not compile or sign it again.

## Read logs and stop the app

Use a bounded log stream:

```bash
sweetpad app logs --project MyApp.xcodeproj --device-id IPHONE_UDID \
  --timeout 30s -o ndjson --non-interactive
sweetpad app stop --project MyApp.xcodeproj --device-id IPHONE_UDID \
  --json --non-interactive
```

Logs follow the app recorded by its device launch.
The adapter supports live scoped logs, not device log history.
For a startup marker, subscribe to logs and relaunch the app from another terminal.

## Return USB to the Mac

After the VM session, detach the selected device on macOS:

```bash
orb usb detach IPHONE_USB_ID
```

If Mac tools do not see the phone again, reconnect the cable.

## Troubleshooting

| Symptom | First check |
| --- | --- |
| No USB device | Confirm the cable, VM attachment, and local usbmuxd service. |
| Pairing fails | Unlock the phone and accept its Trust prompt. |
| Device services fail | Confirm Developer Mode and the pinned Python adapter. |
| App installation fails | Check the profile, bundle identifiers, entitlements, and registered device. |
| The wrong transport is selected | Remove `SWEETPAD_DEVICE_NETWORK_CONFIG` from this shell to select USB. |
| A macro cannot load | Rerun toolchain setup to refresh the macro launcher and its Swift link. |

Run `sweetpad help linux-devices` for Wi-Fi pairing and discovery.
