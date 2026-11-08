import importlib.util
import json
from pathlib import Path
import tempfile
import stat
from types import SimpleNamespace
import unittest
import uuid

SOURCE = Path(__file__).resolve().parents[2] / "sweetpad-cli/src/cli/device_network.py"
spec = importlib.util.spec_from_file_location("device_network", SOURCE)
network = importlib.util.module_from_spec(spec)
spec.loader.exec_module(network)


class NetworkDeviceContracts(unittest.TestCase):
    def test_isolated_identity_cannot_reuse_default_hostname_uuid3(self):
        self.assertEqual(uuid.UUID(network.isolated_host_identifier()).version, 4)

    def test_private_configuration_is_exclusive_and_owner_only(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "devices.json"
            network.write_private_json(path, {"devices": []})
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            with self.assertRaises(FileExistsError):
                network.write_private_json(path, {"changed": True})
            self.assertEqual(json.loads(path.read_text()), {"devices": []})

    def test_device_results_exclude_private_lockdown_values(self):
        device = SimpleNamespace(udid="EXPECTED", all_values={
            "DeviceName": "Phone", "ProductVersion": "27.2",
            "EscrowBag": b"private bytes", "WiFiAddress": "personal metadata"})
        value = network.device_values(device)
        self.assertEqual(value["UniqueDeviceID"], "EXPECTED")
        self.assertEqual(value["ConnectionType"], "Network")
        self.assertNotIn("EscrowBag", value)
        self.assertNotIn("WiFiAddress", value)
        json.dumps(value)

    def test_process_selection_uses_whole_bundle_directory(self):
        self.assertTrue(network.process_matches("/Applications/Probe.app/Probe", "Probe.app"))
        self.assertFalse(network.process_matches("/Applications/MyProbe.app/Probe", "Probe.app"))
        self.assertFalse(network.process_matches("/Applications/Probe.app.backup/Probe", "Probe.app"))

    def test_ambiguous_ids_and_invalid_record_paths_are_rejected(self):
        profile = {"udid": "EXPECTED", "pairRecord": "/private/record.plist"}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "devices.json"
            for devices in ([profile, profile], [dict(profile, pairRecord="relative.plist")],
                            [dict(profile, port=0)], [dict(profile, port=65536)]):
                path.write_text(json.dumps({"devices": devices}))
                with self.assertRaises(ValueError):
                    network.profiles(path)
            path.write_text(json.dumps({"devices": [profile]}))
            self.assertEqual(network.profiles(path), [profile])


if __name__ == "__main__":
    unittest.main()
