---
sidebar_label: Connect an iPhone
---

# Run an app on an iPhone from Linux

To install and launch an app from Linux, set up the [Apple toolchain](sdk-transfer.md) and [development signing](signing-and-testing.md#configure-development-signing). Pair the iPhone with the Linux host that will install the app.

## Connect the iPhone

On Ubuntu or Debian, install the USB service and Python virtual environment support.

```bash
sudo apt-get install python3-venv usbmuxd
```

On Arch, install `python` and `usbmuxd` with pacman. On each distribution, run these commands on Linux to prepare the device adapter.

```bash
python3 -m venv "$HOME/.local/share/sweetpad/device-tools"
"$HOME/.local/share/sweetpad/device-tools/bin/python" -m pip install pymobiledevice3==11.50.0
export SWEETPAD_DEVICE_PYTHON="$HOME/.local/share/sweetpad/device-tools/bin/python"
sudo systemctl start usbmuxd
```

Set `SWEETPAD_DEVICE_PYTHON` in each shell that runs device commands. Enable Developer Mode on the iPhone. Unlock it and connect it with a USB data cable.

<details>
<summary>Attach the phone to an OrbStack VM on a Mac</summary>

On the Mac, find the phone’s USB ID and attach it to the Linux VM. Replace `sweetpad-linux` with your VM name.

```bash
orb usb list
orb usb attach IPHONE_USB_ID -m sweetpad-linux
```

The VM needs its own pairing even if the phone already trusts the Mac. When you finish, run `orb usb detach IPHONE_USB_ID` on the Mac.

</details>

On Linux, start pairing. Unlock the phone and tap **Trust** when prompted.

```bash
"$SWEETPAD_DEVICE_PYTHON" -m pymobiledevice3 lockdown pair
sweetpad device list --json --non-interactive
```

Use the device ID (UDID) from this list for the install and launch commands.

After a restart or first connection, launch and other Instruments operations can need a developer disk image. The USB and Wi-Fi adapters check the mounted image and mount one for the phone’s iOS version if needed. Keep the phone unlocked. The first mount can need internet access to download the image and personalize it through Apple.

## Build, install, and launch

From your app's folder, with your project, scheme, and UDID:

```bash
sweetpad build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/MyApp" > build.json

app_path="$(python3 -c 'import json; print(json.load(open("build.json"))["data"]["productPath"])')"
sweetpad device install "$app_path" \
  --device IPHONE_UDID --json --non-interactive

sweetpad app launch --project MyApp.xcodeproj --scheme MyApp \
  --device-id IPHONE_UDID --derived-data-path "$HOME/builds/MyApp" \
  --json --non-interactive
```

Read `data.productPath` from a successful build. The product name and configuration determine the bundle path.
`device install` installs a signed app or IPA as it is, without building or signing it again.

## Read logs and stop the app

```bash
sweetpad app logs --project MyApp.xcodeproj --device-id IPHONE_UDID \
  --timeout 30s -o ndjson --non-interactive
sweetpad app stop --project MyApp.xcodeproj --device-id IPHONE_UDID \
  --json --non-interactive
```

The adapter streams live messages for the selected app. It cannot retrieve older entries from the device.
To catch startup messages, start the log stream first, then launch the app from another terminal.

## Use Wi-Fi

With the phone trusted over USB, create a separate network pairing:

```bash
sweetpad device pair-network IPHONE_UDID \
  --out "$HOME/.local/share/sweetpad/iphone-network" --json --non-interactive
export SWEETPAD_DEVICE_NETWORK_CONFIG="$HOME/.local/share/sweetpad/iphone-network/devices.json"
sweetpad device list --json --non-interactive
```

Use a new output directory. It holds a private pairing record that lets the Linux host connect directly to the phone. After pairing, USB can be disconnected. The phone and Linux host must be reachable on the same network.
The device commands find the phone through multicast or the system's DNS-SD resolver, or at a host and port the configuration names.
Unset `SWEETPAD_DEVICE_NETWORK_CONFIG` to go back to USB.
`sweetpad help linux-devices` covers the configuration.

## Check the toolchain without a phone

Setup already builds an unsigned device sample after it imports your SDK. To check a simulator build separately, run this on the [configured Linux host](sdk-transfer.md).

```bash
sweetpad project new LinuxApp --bundle-id dev.example.LinuxApp --no-git
sweetpad build --project LinuxApp/LinuxApp.xcodeproj --scheme LinuxApp \
  --destination 'generic/platform=iOS Simulator' --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/LinuxApp" CODE_SIGNING_ALLOWED=NO
```

Running simulator products still needs a Mac.

## Troubleshooting

| Symptom | First check |
| --- | --- |
| No USB device | The cable, the VM attachment, and the usbmuxd service. |
| Pairing fails | Unlock the phone and accept its Trust prompt. |
| Device services fail | Developer Mode, and `pymobiledevice3==11.50.0` in `SWEETPAD_DEVICE_PYTHON`. |
| Installation fails | The profile, bundle identifiers, entitlements, and registered devices. |
| Launch fails after a restart | Unlock the phone once, check Developer Mode, and allow network access for the first developer image mount. |
