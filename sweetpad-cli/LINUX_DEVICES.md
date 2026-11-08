# Linux device transport

The CLI supports physical iPhones through local Linux USB or direct Wi-Fi.
Both paths use userspace RSD tunnels and require `pymobiledevice3==11.50.0`.
App operations authenticate the device UDID before installation, launch, or termination.
They do not require a Mac relay or access to its login keychain.

## Python setup

Create a private Python environment and install the pinned dependency:

```bash
uv venv --python 3.12 "$HOME/.local/share/sweetpad/device-tools"
uv pip install --python "$HOME/.local/share/sweetpad/device-tools/bin/python" \
  pymobiledevice3==11.50.0
export SWEETPAD_DEVICE_PYTHON="$HOME/.local/share/sweetpad/device-tools/bin/python"
```

`SWEETPAD_DEVICE_PYTHON` selects the modern adapter for Linux device commands.
Without this variable or a network configuration, the existing `pymobiledevice3` CLI adapter remains available.

## USB

Install and start `usbmuxd` on Linux. The phone must already trust that Linux host.
The adapter does not create USB trust or dismiss a Trust dialog automatically.

For an OrbStack VM, pass only the selected iPhone through from the Mac:

```bash
orb usb list
orb usb attach IPHONE_USB_ID -m YOUR_LINUX_MACHINE
```

Dedicated passthrough removes the phone from Mac USB transport until detachment.
[OrbStack's USB documentation](https://docs.orbstack.dev/features/usb) describes ownership and supported hosts.

Run device commands inside Linux:

```bash
sweetpad device list --json --non-interactive
sweetpad device install /path/to/MyApp.app --device IPHONE_UDID --json --non-interactive
```

`device install` accepts an existing signed app or IPA. It does not compile or re-sign the product.

After the VM session, return the phone to Mac USB transport:

```bash
orb usb detach IPHONE_USB_ID
```

The validation host required USB re-enumeration after detachment before Mac usbmux listed the phone again.
If Mac device tools remain empty after detachment, reconnect the cable.

## Wi-Fi pairing

With the trusted phone attached through USB, create a separate network record:

```bash
sweetpad device pair-network IPHONE_UDID \
  --out "$HOME/.local/share/sweetpad/iphone-network" --json --non-interactive
export SWEETPAD_DEVICE_NETWORK_CONFIG="$HOME/.local/share/sweetpad/iphone-network/devices.json"
sweetpad device list --json --non-interactive
```

The command enables the host's Wi-Fi connection setting and creates a separate RemotePairing record.
It rejects an existing output directory and preserves the default pairing records.
The output directory uses mode `0700`. The record and configuration use mode `0600`.
The record includes its host identifier, so a private copy can authenticate from another Linux host.

Both hosts must share a network that permits connections to the phone.
The generated configuration contains no fixed address or port:

```json
{
  "devices": [
    {
      "udid": "IPHONE_UDID",
      "pairRecord": "/private/path/remote-record.plist"
    }
  ]
}
```

The adapter first looks for multicast `_remotepairing._tcp` advertisements.
It also queries the system DNS resolver for DNS-SD records when multicast returns no matching device.
This fallback works with OrbStack's DNS proxy without a discovery daemon or packet relay.
DNS queries use bounded retries and TCP fallback for truncated responses.
The pairing record authenticates each advertisement. The RSD handshake then verifies the expected hardware UDID.

For networks without discovery, add both `host` and `port` to the device entry.
The port belongs to RemotePairing, not Lockdown.
Older records without `host_identifier` also require `hostIdentifier`, the UUID of their source host.
The configuration selects network transport explicitly. Failed authentication does not fall back to USB.

## App commands

After installation, launch the app from its existing build products:

```bash
sweetpad app launch --project MyApp.xcodeproj --scheme MyApp \
  --device-id IPHONE_UDID --derived-data-path /path/to/DerivedData \
  --arg 'two words' --env MODE=validation --json --non-interactive
sweetpad app logs --project MyApp.xcodeproj --device-id IPHONE_UDID \
  --until 'Ready to serve' --timeout 30s -o ndjson --non-interactive
sweetpad app stop --project MyApp.xcodeproj --device-id IPHONE_UDID --json --non-interactive
```

The same commands work over modern USB and Wi-Fi transport.
Logs follow the recorded device launch, including products from a custom DerivedData directory.
The stream supports `--level`, `--subsystem`, `--category`, `--until`, and `--timeout`.
The adapter retains only the selected app's process and sender images.
It does not support device log history, arbitrary predicates, or captured stdout as a log source.
The console during `app run` also carries the app's stdout and stderr.
Linux device readiness inspection and device debugging remain separate unsupported commands.

## Validation

The November 8, 2026 checks used an iPhone 13 with iOS 27.2.
Both transports pass discovery, installation, launch, matching-process startup logs, stop, uninstall, and reinstallation through the CLI.
The exported Linux executable remains unchanged. No app compilation or re-signing occurs during these checks.
Automatic Wi-Fi discovery succeeds without a configured phone hostname or port.
The VM uses the DNS-SD fallback because raw LAN multicast does not cross its bridge.
USB checks use physical passthrough and local Linux `usbmuxd`, with no network configuration or host relay.

Local reports are under `sweetpad-lib/corpus/_linux-followups/validation/transport-2026-11-08/`.
