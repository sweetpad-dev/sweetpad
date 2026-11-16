---
sidebar_label: Connect an iPhone
---

# Run an app on an iPhone from Linux

This page takes a Linux host with the [Apple toolchain](sdk-transfer.md) and [signing](signing-and-testing.md) set up, and ends with your app running on an iPhone.

## Check the toolchain without a phone

A generated sample confirms that the compiler and SDK work:

```bash
sweetpad project new LinuxApp --bundle-id dev.example.LinuxApp --no-git
sweetpad build --project LinuxApp/LinuxApp.xcodeproj --scheme LinuxApp \
  --destination 'generic/platform=iOS Simulator' --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/LinuxApp" CODE_SIGNING_ALLOWED=NO
```

Running simulator products still needs a Mac.

## Connect the iPhone

Turn on Developer Mode on the iPhone, connect it with a data cable, unlock it, and trust the Linux host when the phone asks.

In a VM, pass the phone through from the Mac first. With OrbStack:

```bash
orb usb list
orb usb attach IPHONE_USB_ID -m sweetpad-linux
```

The VM owns the phone until you run `orb usb detach IPHONE_USB_ID` on the Mac.

Then install the device adapter on Linux and pair:

```bash
sudo apt-get install python3-venv usbmuxd
python3 -m venv "$HOME/.local/share/sweetpad/device-tools"
"$HOME/.local/share/sweetpad/device-tools/bin/python" -m pip install pymobiledevice3==11.50.0
export SWEETPAD_DEVICE_PYTHON="$HOME/.local/share/sweetpad/device-tools/bin/python"
sudo systemctl start usbmuxd
"$SWEETPAD_DEVICE_PYTHON" -m pymobiledevice3 lockdown pair
sweetpad device list --json --non-interactive
```

Accept the Trust prompt on the phone while it pairs, and note the device's UDID from the list.
On Arch, install `usbmuxd` with pacman and use the system Python.

## Build, install, and launch

From your app's folder, with your project, scheme, and UDID:

```bash
sweetpad build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/MyApp"

sweetpad device install "$HOME/builds/MyApp/Build/Products/Debug-iphoneos/MyApp.app" \
  --device IPHONE_UDID --json --non-interactive

sweetpad app launch --project MyApp.xcodeproj --scheme MyApp \
  --device-id IPHONE_UDID --derived-data-path "$HOME/builds/MyApp" \
  --json --non-interactive
```

The build's JSON output gives the product path, which changes with the product name and configuration.
`device install` installs a signed app or IPA as it is, without building or signing it again.

## Read logs and stop the app

```bash
sweetpad app logs --project MyApp.xcodeproj --device-id IPHONE_UDID \
  --timeout 30s -o ndjson --non-interactive
sweetpad app stop --project MyApp.xcodeproj --device-id IPHONE_UDID \
  --json --non-interactive
```

Logs are live only: the adapter can't read the device's log history.
To catch startup messages, start the log stream first, then launch the app from another terminal.

## Use Wi-Fi

With the phone trusted over USB, create a separate network pairing:

```bash
sweetpad device pair-network IPHONE_UDID \
  --out "$HOME/.local/share/sweetpad/iphone-network" --json --non-interactive
export SWEETPAD_DEVICE_NETWORK_CONFIG="$HOME/.local/share/sweetpad/iphone-network/devices.json"
sweetpad device list --json --non-interactive
```

The new folder holds private credentials.
The device commands find the phone through multicast or the system's DNS-SD resolver, or at a host and port the configuration names.
Unset `SWEETPAD_DEVICE_NETWORK_CONFIG` to go back to USB.
`sweetpad help linux-devices` covers the configuration.

## Troubleshooting

| Symptom | First check |
| --- | --- |
| No USB device | The cable, the VM attachment, and the usbmuxd service. |
| Pairing fails | Unlock the phone and accept its Trust prompt. |
| Device services fail | Developer Mode, and `pymobiledevice3==11.50.0` in `SWEETPAD_DEVICE_PYTHON`. |
| Installation fails | The profile, bundle identifiers, entitlements, and registered devices. |
| A macro can't load | Run `sweetpad setup` again. |
