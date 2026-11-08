#!/usr/bin/env python3
"""Install, launch, and observe a signed app over direct Linux Wi-Fi transport.

Requires pymobiledevice3==11.50.0 and an existing RemotePairing record outside
the repository. The app must emit the supplied os_log marker at startup.
No compilation, signing, account changes, or Mac device relay occurs here.
"""
import argparse
import asyncio
from contextlib import AsyncExitStack
import hashlib
import ipaddress
import json
from pathlib import Path
import plistlib
import socket


def executable_hashes(app):
    files = set(app.rglob("*.dylib"))
    for info in app.rglob("Info.plist"):
        executable = plistlib.loads(info.read_bytes()).get("CFBundleExecutable")
        if executable and (info.parent / executable).is_file():
            files.add(info.parent / executable)
    return {str(p.relative_to(app)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(files)}


async def validate(args, report, save):
    # Import the pinned device library only after parsing arguments.
    from pymobiledevice3.bonjour import browse_remotepairing
    from pymobiledevice3.remote import tunnel_service
    from pymobiledevice3.remote.remote_service_discovery import RemoteServiceDiscoveryService
    from pymobiledevice3.remote.userspace_tunnel import UserspaceDialPlane
    from pymobiledevice3.services.installation_proxy import InstallationProxyService
    from pymobiledevice3.services.dvt.instruments.dvt_provider import DvtProvider
    from pymobiledevice3.services.dvt.instruments.process_control import ProcessControl
    from pymobiledevice3.services.dvt.instruments.activity_trace_tap import ActivityTraceTap

    info = plistlib.loads((args.app / "Info.plist").read_bytes())
    if info.get("CFBundleIdentifier") != args.bundle_id:
        raise ValueError("the product bundle identifier differs from --bundle-id")
    if not (args.app / "embedded.mobileprovision").is_file():
        raise ValueError("a provisioned, signed device product is required")
    before = executable_hashes(args.app)
    report["executableHashes"] = before
    # Multicast can be unavailable inside a VM even when unicast DNS and TCP work.
    answers = await browse_remotepairing(timeout=3)
    addresses = list(dict.fromkeys(r[4][0] for r in
        socket.getaddrinfo(args.host, args.port, family=socket.AF_INET, type=socket.SOCK_STREAM)))
    addresses = [a for a in addresses if not ipaddress.ip_address(a).is_link_local]
    report["discovery"] = {"host": args.host, "resolvedIPv4Addresses": addresses,
                           "multicastServiceCount": len(answers), "port": args.port}
    save()
    if not addresses:
        raise ValueError("the host has no routable IPv4 address")

    class PrivateRecordProvider(tunnel_service.RemotePairingTunnelService):
        @property
        def pair_record_path(self):
            return args.pair_record

    async with AsyncExitStack() as stack:
        provider = None
        for address in addresses:
            candidate = PrivateRecordProvider(args.device, address, args.port)
            if args.host_identifier:
                candidate.identifier = args.host_identifier
            try:
                await candidate.connect(autopair=False)
            except Exception:
                await candidate.close()
                continue
            provider = candidate
            report["discovery"]["authenticatedAddress"] = address
            break
        if provider is None:
            raise RuntimeError("none of the resolved addresses accepted the existing RemotePairing record")
        stack.push_async_callback(provider.close)
        # This library flag selects its userspace network device. It does not
        # replace socket APIs or relay traffic through another host.
        tunnel_service.USE_USERSPACE_TUNNEL = True
        tunnel = await stack.enter_async_context(provider.start_tcp_tunnel())
        tun = tunnel.client.tun
        tun.set_peer(tunnel.address)
        plane = await stack.enter_async_context(UserspaceDialPlane(tun, tunnel.address))
        rsd = RemoteServiceDiscoveryService((tunnel.address, tunnel.port),
            open_connection=plane.dial, auxiliary_metadata=tunnel.auxiliary_metadata)
        stack.push_async_callback(rsd.close)
        await rsd.connect()
        if rsd.udid != args.device:
            raise RuntimeError("the authenticated device differs from --device")
        report["authenticatedDevice"] = rsd.udid
        save()
        async with InstallationProxyService(rsd) as service:
            await service.install_from_local(str(args.app), developer=True)
            installed = args.bundle_id in await service.get_apps()
        report["installed"] = installed
        save()
        if not installed:
            raise RuntimeError("the app is absent after installation")
        async with DvtProvider(rsd) as dvt, ProcessControl(dvt) as pc, ActivityTraceTap(dvt) as tap:
            async def read_marker():
                async for entry in tap:
                    message = str(getattr(entry, "format_string", "")) + " " + str(getattr(entry, "message", ""))
                    if args.log_marker in message:
                        # Retain only the validation marker, never unrelated device logs.
                        return {"marker": args.log_marker, "processIdentifier": entry.process,
                                "subsystem": getattr(entry, "subsystem", None)}

            reader = asyncio.create_task(read_marker())
            try:
                pid = await pc.launch(args.bundle_id, kill_existing=True)
                report["launchedProcessIdentifier"] = pid
                save()
                report["log"] = await asyncio.wait_for(reader, 45)
                if pid <= 0 or report["log"]["processIdentifier"] != pid:
                    raise RuntimeError("the startup marker did not come from the launched process")
                await asyncio.sleep(5)
                running = await pc.process_identifier_for_bundle_identifier(args.bundle_id)
                report["runningProcessIdentifier"] = running
                if running != pid:
                    raise RuntimeError("the launched app did not remain running")
            finally:
                reader.cancel()
                await asyncio.gather(reader, return_exceptions=True)
    report["executablesUnchanged"] = executable_hashes(args.app) == before
    if not report["executablesUnchanged"]:
        raise RuntimeError("the source product changed during device validation")
    report["passed"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", required=True, help="iPhone LAN hostname or IPv4 address")
    parser.add_argument("--port", type=int, required=True, help="Advertised RemotePairing TCP port")
    parser.add_argument("--device", required=True, help="Expected iPhone UDID")
    parser.add_argument("--pair-record", type=Path, required=True, help="Private RemotePairing plist; never copied to reports")
    parser.add_argument("--host-identifier", help="Source host UUID for records without host_identifier")
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--bundle-id", required=True)
    parser.add_argument("--log-marker", required=True)
    parser.add_argument("--out", type=Path, required=True, help="Fresh evidence directory")
    args = parser.parse_args()
    args.app = args.app.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    report = {"passed": False, "bundleIdentifier": args.bundle_id,
              "transport": "Direct Linux TCP RemotePairing userspace RSD",
              "macRelay": False, "compilation": False, "signing": False}

    def save():
        (args.out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")

    try:
        asyncio.run(asyncio.wait_for(validate(args, report, save), 240))
    except Exception as exc:
        report["error"] = {"type": type(exc).__name__, "message": str(exc)}
        raise
    finally:
        save()
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
