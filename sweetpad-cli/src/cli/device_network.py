"""Embedded Linux Wi-Fi and USB transport. Requires pymobiledevice3 11.50.0.

Requests and results use JSON. Private pairing records stay on disk and never
enter the Rust process, arguments, or output. All device actions authenticate
the configured UDID before opening an app service.
"""
import asyncio
from contextlib import asynccontextmanager, AsyncExitStack
from importlib.metadata import version
import json
import math
import os
from pathlib import Path, PurePosixPath
import plistlib
import socket
import struct
import sys
import uuid


def profiles(path):
    values = json.loads(Path(path).read_text())["devices"]
    identifiers = [p["udid"] for p in values]
    if len(identifiers) != len(set(identifiers)):
        raise ValueError("duplicate UDIDs in network device configuration")
    for p in values:
        if not p["udid"] or not Path(p["pairRecord"]).is_absolute():
            raise ValueError("network devices require a UDID and an absolute pairRecord path")
        if p.get("port") is not None and not 0 < p["port"] < 65536:
            raise ValueError("RemotePairing port must be between 1 and 65535")
    return values


async def dns_records(server, name, kind):
    """Query the system DNS proxy. Validate the peer, transaction, and QR bit."""
    from pymobiledevice3.bonjour import build_query, parse_mdns_message
    transaction = int.from_bytes(os.urandom(2), "big")
    query = build_query(name, kind)
    query = struct.pack("!HH", transaction, 0x0100) + query[4:]
    sock = socket.socket(socket.AF_INET6 if ":" in server else socket.AF_INET, socket.SOCK_DGRAM)
    sock.setblocking(False)
    try:
        loop = asyncio.get_running_loop()
        await loop.sock_connect(sock, (server, 53))
        await loop.sock_sendall(sock, query)
        packet = await asyncio.wait_for(loop.sock_recv(sock, 65535), 4)
        txid, flags = struct.unpack("!HH", packet[:4])
        if flags & 0x0200:
            reader, writer = await asyncio.wait_for(asyncio.open_connection(server, 53), 4)
            try:
                writer.write(struct.pack("!H", len(query)) + query)
                await writer.drain()
                size = struct.unpack("!H", await asyncio.wait_for(reader.readexactly(2), 4))[0]
                packet = await asyncio.wait_for(reader.readexactly(size), 4)
                txid, flags = struct.unpack("!HH", packet[:4])
            finally:
                writer.close()
                await writer.wait_closed()
        if txid != transaction or not flags & 0x8000 or flags & 0x020F:
            return []
        return [r for r in parse_mdns_message(packet)
                if r["type"] == kind and r["name"].lower() == name.lower() and r["ttl"] > 0]
    except (OSError, ValueError, TimeoutError, EOFError, struct.error):
        return []
    finally:
        sock.close()


async def dns_sd_fallback():
    """Some VM resolvers bridge DNS-SD while their network drops multicast."""
    from pymobiledevice3.bonjour import ServiceInstance
    servers = [l.split()[1] for l in Path("/etc/resolv.conf").read_text().splitlines()
               if l.startswith("nameserver ")][:3]
    result = []
    for server in servers:
        pointers = await dns_records(server, "_remotepairing._tcp.local.", 12)
        names = list(dict.fromkeys(p["ptrdname"] for p in pointers))[:64]
        async def resolve(name):
            srv, txt = await asyncio.gather(dns_records(server, name, 33), dns_records(server, name, 16))
            if not srv or not txt:
                return None
            return ServiceInstance(instance=name, host=srv[0]["target"], port=srv[0]["port"],
                                   addresses=[], properties=txt[0]["txt"])
        result.extend(a for a in await asyncio.gather(*(resolve(n) for n in names)) if a is not None)
        if result:
            break
    return result


async def endpoints(profile):
    from pymobiledevice3.bonjour import browse_remotepairing
    from pymobiledevice3.remote.tunnel_service import _match_remote_pair_record, PEER_ALT_IRK_KEY
    if profile.get("host") and profile.get("port"):
        return [(profile["host"], profile["port"])]
    record = plistlib.loads(Path(profile["pairRecord"]).read_bytes())
    irk = record.get(PEER_ALT_IRK_KEY)
    if irk is None:
        raise ValueError("automatic discovery requires a RemotePairing record with peer_alt_irk; bootstrap a current record over trusted USB, or configure host and port")
    for attempt in range(3):
        # Keep the same authenticated advertisement contract on both paths.
        # DNS-SD is a discovery aid, never an authentication authority.
        for discovery in ("multicast", "system-dns-sd"):
            answers = (await browse_remotepairing(timeout=2) if discovery == "multicast"
                       else await dns_sd_fallback())
            result = []
            for answer in answers:
                if _match_remote_pair_record(answer, {profile["udid"]: irk}) != profile["udid"]:
                    continue
                # Resolve advertised hostnames locally. Foreign interface scopes
                # must never enter a local socket address.
                if profile.get("host") and (answer.host or "").rstrip(".").lower() != profile["host"].rstrip(".").lower():
                    continue
                if answer.host:
                    result.append((answer.host, answer.port))
                result.extend((a.full_ip, answer.port) for a in answer.addresses)
            if result:
                profile["discoveryMethod"] = discovery
                return list(dict.fromkeys(result))
        if attempt < 2:
            await asyncio.sleep(0.2)
    raise ValueError("no authenticated RemotePairing advertisement found; use host and port when multicast cannot reach this host")


@asynccontextmanager
async def connection(profile):
    from pymobiledevice3.remote import tunnel_service
    from pymobiledevice3.remote.remote_service_discovery import RemoteServiceDiscoveryService
    from pymobiledevice3.remote.userspace_tunnel import UserspaceDialPlane

    class Provider(tunnel_service.RemotePairingTunnelService):
        @property
        def pair_record_path(self):
            return Path(profile["pairRecord"])

    async with AsyncExitStack() as stack:
        if profile.get("usb"):
            from pymobiledevice3.lockdown import create_using_usbmux
            client = await create_using_usbmux(serial=profile["udid"], connection_type="USB", autopair=False)
            stack.push_async_callback(client.close)
            provider = await tunnel_service.CoreDeviceTunnelProxy.create(client)
        else:
            provider = None
            failures = []
            for host, port in await endpoints(profile):
                candidate = Provider(profile["udid"], host, port)
                if profile.get("hostIdentifier"):
                    candidate.identifier = profile["hostIdentifier"]
                try:
                    await asyncio.wait_for(candidate.connect(autopair=False), 10)
                except Exception as exc:
                    await candidate.close()
                    failures.append(f"{host}:{port}: {type(exc).__name__}")
                    continue
                provider = candidate
                break
            if provider is None:
                raise ConnectionError("RemotePairing authentication failed (" + "; ".join(failures) + ")")
        stack.push_async_callback(provider.close)
        tunnel_service.USE_USERSPACE_TUNNEL = True
        tunnel = await stack.enter_async_context(provider.start_tcp_tunnel())
        tun = tunnel.client.tun
        tun.set_peer(tunnel.address)
        plane = await stack.enter_async_context(UserspaceDialPlane(tun, tunnel.address))
        rsd = RemoteServiceDiscoveryService((tunnel.address, tunnel.port),
            open_connection=plane.dial, auxiliary_metadata=tunnel.auxiliary_metadata)
        stack.push_async_callback(rsd.close)
        await rsd.connect()
        if rsd.udid != profile["udid"]:
            raise ValueError("authenticated device UDID differs from the configured device; no app action was performed")
        yield rsd


def device_values(rsd, connection_type="Network"):
    values = {k: rsd.all_values.get(k, "") for k in
              ("DeviceName", "ProductType", "ProductVersion", "DeviceClass")}
    return dict(values, UniqueDeviceID=rsd.udid, ConnectionType=connection_type, PairingState="paired")


def process_matches(executable, app_dir):
    return app_dir in PurePosixPath(executable).parts


def write_private_json(path, value):
    with open(path, "x", opener=lambda name, flags: os.open(name, flags, 0o600)) as out:
        json.dump(value, out, indent=2)


def isolated_host_identifier():
    # Standard pymobiledevice3 records use a hostname-derived UUID3. A separate
    # UUID4 identity also preserves their device-side trust, not only their files.
    return str(uuid.uuid4()).upper()


async def pair_network(request):
    """Create an isolated record over already trusted USB, never overwrite one."""
    from pymobiledevice3.lockdown import create_using_usbmux
    from pymobiledevice3.remote.tunnel_service import RemotePairingLockdownService
    from pymobiledevice3.exceptions import RemotePairingCompletedError
    directory = Path(request["out"]).resolve()
    directory.mkdir(mode=0o700, parents=True, exist_ok=False)
    record_path = directory / "remote-record.plist"

    class PrivateRecord(RemotePairingLockdownService):
        @property
        def pair_record_path(self):
            return record_path

    client = await create_using_usbmux(serial=request["device"], connection_type="USB", autopair=False)
    try:
        await client.set_value(True, domain="com.apple.mobile.wireless_lockdown", key="EnableWifiConnections")
        service = await PrivateRecord.create(client)
        service.identifier = isolated_host_identifier()
        try:
            try:
                await service.connect(autopair=True)
            except RemotePairingCompletedError:
                pass
            if not record_path.is_file():
                raise RuntimeError("the device did not create a RemotePairing record")
            record_path.chmod(0o600)
            record = plistlib.loads(record_path.read_bytes())
            record["host_identifier"] = service.identifier
            record_path.write_bytes(plistlib.dumps(record))
        finally:
            await service.close()
        profile = {"udid": client.udid, "pairRecord": str(record_path)}
        config = directory / "devices.json"
        write_private_json(config, {"devices": [profile]})
        return {"paired": True, "device": client.udid, "configuration": str(config),
                "transport": "trusted USB bootstrap; subsequent app traffic uses direct Wi-Fi"}
    finally:
        await client.close()


async def perform(rsd, request):
    from pymobiledevice3.services.installation_proxy import InstallationProxyService
    from pymobiledevice3.services.dvt.instruments.dvt_provider import DvtProvider
    from pymobiledevice3.services.dvt.instruments.process_control import ProcessControl
    from pymobiledevice3.services.dvt.instruments.device_info import DeviceInfo
    action = request["action"]
    if action == "device":
        return device_values(rsd)
    if action in ("install", "uninstall"):
        async with InstallationProxyService(rsd) as service:
            if action == "install":
                await service.install_from_local(request["app"], developer=True)
            else:
                await service.uninstall(request["bundle"])
        return {"completed": True}
    async with DvtProvider(rsd) as dvt, ProcessControl(dvt) as pc:
        if action in ("launch", "console"):
            pid = await pc.launch(request["bundle"], arguments=request.get("args", []),
                environment=request.get("env", {}), start_suspended=request.get("suspended", False),
                kill_existing=True)
            if pid <= 0:
                raise RuntimeError("the device did not return a launched process identifier")
            if action == "console":
                async for output in pc:
                    if output.pid == pid:
                        print(output.message, end="", flush=True)
            return {"pid": pid}
        if action in ("processes", "terminate"):
            async with DeviceInfo(dvt) as info:
                result = []
                for item in await info.proclist():
                    pid = item["pid"]
                    executable = item.get("realAppName") or await info.execname_for_pid(pid)
                    if not process_matches(executable, request["appDir"]):
                        continue
                    app_path = str(PurePosixPath(executable))
                    app_path = app_path[:app_path.index(request["appDir"]) + len(request["appDir"])]
                    result.append({"pid": pid, "executable": "file://" + executable,
                                   "app_path": app_path, "main": PurePosixPath(executable).parent.name == request["appDir"]})
                    if action == "terminate":
                        await pc.kill(pid)
                return result
        if action == "logs":
            from pymobiledevice3.services.os_trace import OsTraceService, OsActivityStreamFlag
            async with OsTraceService(rsd) as service:
                flags = OsActivityStreamFlag.DEBUG | OsActivityStreamFlag.INFO | OsActivityStreamFlag.NO_SENSITIVE
                ready = False
                async for entry in service.syslog(stream_flags=int(flags)):
                    if not ready:
                        if request.get("readyEvent", True):
                            print(json.dumps({"event": "device_logs_ready", "udid": rsd.udid}), flush=True)
                        ready = True
                    if PurePosixPath(entry.image_name).name not in (request["executable"], request["executable"] + ".debug.dylib"):
                        continue
                    if entry.pid != await pc.process_identifier_for_bundle_identifier(request["bundle"]):
                        continue
                    subsystem = entry.label.subsystem if entry.label else ""
                    category = entry.label.category if entry.label else entry.image_name
                    if request.get("subsystem") and request["subsystem"] != subsystem:
                        continue
                    if request.get("category") and request["category"] != category:
                        continue
                    level = entry.level.name
                    if request.get("level") != "debug" and level == "DEBUG":
                        continue
                    if request.get("level") == "default" and level == "INFO":
                        continue
                    print(json.dumps({"timestamp": entry.timestamp.isoformat(sep=" "),
                        "eventMessage": entry.message, "messageType": "Notice" if level == "USER_ACTION" else level.title(),
                        "subsystem": subsystem, "category": category, "processID": entry.pid,
                        "senderImagePath": entry.image_name}), flush=True)
        else:
            raise ValueError("unknown network device action")


def configured_timeout(value, name):
    timeout = float(value)
    if not math.isfinite(timeout) or not 0 < timeout <= 3600:
        raise ValueError(f"{name} must be greater than zero and at most 3600 seconds")
    return timeout


def action_timeout(request, environment):
    if request["action"] in ("console", "logs"):
        return None
    return configured_timeout(request.get("timeoutSeconds", environment.get("SWEETPAD_DEVICE_TIMEOUT", "180")), "SWEETPAD_DEVICE_TIMEOUT")


async def discover_devices(configured, timeout):
    async def one(profile):
        async def read():
            async with connection(profile) as rsd:
                return device_values(rsd, "USB" if profile.get("usb") else "Network")
        try:
            return await asyncio.wait_for(read(), timeout), None
        except Exception as error:
            reason = f"discovery timed out after {timeout:g}s" if isinstance(error, asyncio.TimeoutError) else f"{type(error).__name__}: {error}"
            return None, f"{profile['udid']}: {reason}"

    rows = await asyncio.gather(*(one(profile) for profile in configured))
    result = [value for value, _ in rows if value is not None]
    errors = [error for _, error in rows if error is not None]
    if errors and not result:
        raise RuntimeError("No configured device could be reached: " + "; ".join(errors))
    for error in errors:
        print("warning: skipping device " + error, file=sys.stderr)
    return result


async def main(request):
    if version("pymobiledevice3") != "11.50.0":
        raise ValueError("device transport requires pymobiledevice3==11.50.0 in SWEETPAD_DEVICE_PYTHON")
    if request["action"] == "pair-network":
        return await pair_network(request)
    if "SWEETPAD_DEVICE_NETWORK_CONFIG" in os.environ:
        configured = profiles(os.environ["SWEETPAD_DEVICE_NETWORK_CONFIG"])
    else:
        from pymobiledevice3.usbmux import list_devices
        configured = [{"udid": d.serial, "usb": True} for d in await list_devices()
                      if d.connection_type == "USB"]
    if request["action"] == "list":
        timeout = configured_timeout(os.environ.get("SWEETPAD_DEVICE_DISCOVERY_TIMEOUT", "30"), "SWEETPAD_DEVICE_DISCOVERY_TIMEOUT")
        return await discover_devices(configured, timeout)
    profile = next((p for p in configured if p["udid"] == request["device"]), None)
    if profile is None:
        raise ValueError("device is absent from SWEETPAD_DEVICE_NETWORK_CONFIG")
    async with connection(profile) as rsd:
        return await perform(rsd, request)


def run_request(request):
    try:
        timeout = action_timeout(request, os.environ)
        result = asyncio.run(asyncio.wait_for(main(request), timeout))
        if result is not None:
            print(json.dumps(result), flush=True)
    except (KeyboardInterrupt, BrokenPipeError):
        pass
    except asyncio.TimeoutError:
        device = f" for {request['device']}" if request.get("device") else ""
        duration = f" after {timeout:g}s" if timeout is not None else ""
        hint = "; check connectivity or increase SWEETPAD_DEVICE_TIMEOUT" if timeout is not None else "; check connectivity"
        print(f"Linux device {request['action']}{device} timed out{duration}{hint}", file=sys.stderr)
        sys.exit(1)
    except Exception as exc:
        print(f"Linux device {request['action']}: {type(exc).__name__}: {exc}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    run_request(json.loads(sys.argv[1]))
