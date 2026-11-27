import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import sdk_coverage as sdk
from verify_registration import equal, normalize_registration


def symbol(name, kind, uid=None):
    path = name.split(".")
    return {"identifier": {"precise": uid or name}, "pathComponents": path,
            "kind": {"identifier": "swift." + kind},
            "declarationFragments": [{"kind": "identifier", "spelling": path[-1]}]}


class SDKCoverageTests(unittest.TestCase):
    def test_new_parameter_result_and_summary_are_unclassified(self):
        symbols = [symbol("IntentResult", "protocol"), symbol("ParameterSummary", "protocol"),
                   symbol("KnownResult", "protocol"), symbol("FutureResult", "protocol"),
                   symbol("NewSummary", "struct"), symbol("IntentParameter.init(title:future:)", "init")]
        edges = [("KnownResult", "IntentResult"), ("FutureResult", "KnownResult"), ("NewSummary", "ParameterSummary")]
        keys, declarations = sdk.inventory([{"symbols": symbols, "relationships": [
            {"kind": "conformsTo", "source": src, "target": dst} for src, dst in edges]}])
        unknown = {"parameter.future", "result.FutureResult", "summary.NewSummary"}
        self.assertTrue(unknown <= keys)
        with patch.object(sdk, "checklist", return_value=dict.fromkeys(keys - unknown, "Match")):
            with self.assertRaises(ValueError) as failed:
                sdk.check(keys, declarations, sdk.BASELINE)
        for name in unknown:
            self.assertIn("Unclassified SDK declaration: " + name, str(failed.exception))

    def test_new_overload_with_previously_classified_labels_fails(self):
        name = "IntentParameter.init(title:)"
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(sdk, "checklist", return_value={"parameter.title": "Match"}):
                keys = {"parameter.title"}
                sdk.check(keys, {name: {"one"}}, "26.5", update=True, directory=Path(tmp))
                with self.assertRaisesRegex(ValueError, "SDK overloads changed"):
                    sdk.check(keys, {name: {"one", "two"}}, "26.6", directory=Path(tmp))

    def test_complete_registration_detects_flags_and_other_schema_changes(self):
        native = json.loads((sdk.HERE / "ResultProtocols.actionsdata").read_text())
        actual = json.loads(json.dumps(native))
        actual["generator"] = {"name": "sweetpad", "version": "test"}
        equal("generator identity", normalize_registration(native), normalize_registration(actual))
        actual["actions"]["InteractiveResultProbe"]["outputFlags"] = 0
        actual["actions"]["ConfirmationSnippet"]["generator"] = {"unexpected": True}
        actual["newRegistrationField"] = []
        with self.assertRaises(ValueError) as failed:
            equal("registration", normalize_registration(native), normalize_registration(actual))
        for path in ["/InteractiveResultProbe/outputFlags", "/ConfirmationSnippet/generator", "/newRegistrationField"]:
            self.assertIn(path, str(failed.exception))


if __name__ == "__main__":
    unittest.main()
