"""Network discovery failures must not remove healthy phones from the list."""
from contextlib import asynccontextmanager, redirect_stderr
import asyncio
import importlib.util
import io
from pathlib import Path
import unittest
from unittest.mock import patch

SPEC=importlib.util.spec_from_file_location("device_network",Path(__file__).parents[1]/"src/cli/device_network.py")
network=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(network)


class DiscoveryTests(unittest.IsolatedAsyncioTestCase):
    async def test_bad_phone_does_not_hide_healthy_phone(self):
        @asynccontextmanager
        async def connection(profile):
            if profile["udid"] == "bad": raise ValueError("not trusted")
            yield profile
        stderr=io.StringIO()
        with patch.object(network,"connection",connection), patch.object(network,"device_values",lambda profile,transport: {"UDID":profile["udid"]}), redirect_stderr(stderr):
            values=await network.discover_devices([{"udid":"bad"},{"udid":"good"}],1)
        self.assertEqual(values,[{"UDID":"good"}])
        self.assertIn("bad: ValueError: not trusted",stderr.getvalue())

    async def test_all_failed_devices_explain_the_failure(self):
        @asynccontextmanager
        async def connection(profile):
            raise OSError("unreachable")
            yield profile
        with patch.object(network,"connection",connection):
            with self.assertRaisesRegex(RuntimeError,"bad: OSError: unreachable"):
                await network.discover_devices([{"udid":"bad"}],1)

    async def test_empty_discovery_remains_a_successful_empty_list(self):
        self.assertEqual(await network.discover_devices([],1),[])

    def test_action_timeouts_are_configurable_and_streams_remain_unbounded(self):
        self.assertEqual(network.action_timeout({"action":"install"},{}),180)
        self.assertEqual(network.action_timeout({"action":"install"},{"SWEETPAD_DEVICE_TIMEOUT":"600"}),600)
        self.assertIsNone(network.action_timeout({"action":"logs"},{}))
        for bad in ["0","-1","nan","inf","3601"]:
            with self.assertRaises(ValueError): network.configured_timeout(bad,"timeout")


class RequestTimeoutTests(unittest.TestCase):
    def test_timed_out_action_names_its_device_and_duration(self):
        async def stalled(request):
            await asyncio.sleep(30)
        stderr=io.StringIO()
        with patch.object(network,"main",stalled), redirect_stderr(stderr), self.assertRaises(SystemExit) as error:
            network.run_request({"action":"install","device":"fixture-phone","timeoutSeconds":0.001})
        self.assertEqual(error.exception.code,1)
        self.assertIn("install for fixture-phone timed out after 0.001s",stderr.getvalue())

    def test_stream_timeout_is_reported_without_a_global_deadline(self):
        async def failed(request):
            raise asyncio.TimeoutError()
        stderr=io.StringIO()
        with patch.object(network,"main",failed), redirect_stderr(stderr), self.assertRaises(SystemExit) as error:
            network.run_request({"action":"logs","device":"fixture-phone"})
        self.assertEqual(error.exception.code,1)
        self.assertIn("logs for fixture-phone timed out",stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
