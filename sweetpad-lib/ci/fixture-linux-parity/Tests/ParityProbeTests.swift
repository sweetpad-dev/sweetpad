import XCTest
import UIKit
import ValueKit
@testable import ParityProbe

final class ParityProbeTests: XCTestCase {
    private var expected: [String: String] { ProcessInfo.processInfo.environment }

    @MainActor
    func testCompiledValuesAndOrigin() async {
        XCTAssertEqual(ProbeValues.greeting, expected["EXPECTED_GREETING"] ?? "source-v1|package-v1")
        XCTAssertEqual(ProbeValues.origin, expected["EXPECTED_ORIGIN"] ?? "xcode")
    }

    @MainActor
    func testAppAndPackageResources() async throws {
        XCTAssertEqual(try ProbeValues.resource(), expected["EXPECTED_RESOURCE"] ?? "resource-v1")
        XCTAssertEqual(try ValueKit.resource(), expected["EXPECTED_PACKAGE_RESOURCE"] ?? "package-resource-v1")
        let obsolete = Bundle.main.url(forResource: "obsolete", withExtension: "txt")
        XCTAssertEqual(obsolete != nil, expected["EXPECT_OBSOLETE"] != "false")
        if let oldName = expected["ABSENT_RESOURCE"] {
            XCTAssertNil(Bundle.main.url(forResource: oldName, withExtension: "txt"))
        }
    }

    @MainActor
    func testAssetLoading() async throws {
        if expected["EXPECT_BADGE"] == "false" {
            XCTAssertNil(UIImage(named: "Badge"))
        } else {
            let image = try XCTUnwrap(UIImage(named: "Badge"))
            XCTAssertGreaterThan(image.size.width, 0)
        }
        let color = try XCTUnwrap(UIColor(named: "Accent"))
        var red: CGFloat = 0, green: CGFloat = 0, blue: CGFloat = 0, alpha: CGFloat = 0
        XCTAssertTrue(color.getRed(&red, green: &green, blue: &blue, alpha: &alpha))
        XCTAssertEqual(red, CGFloat(Double(expected["EXPECTED_RED"] ?? "0.25")!), accuracy: 0.005)
        XCTAssertEqual(green, 0.5, accuracy: 0.005)
        XCTAssertEqual(blue, 0.75, accuracy: 0.005)
    }

    @MainActor
    func testStoryboardInitialChild() async throws {
        let storyboard = UIStoryboard(name: "InitialChild", bundle: .main)
        let controller = try XCTUnwrap(storyboard.instantiateInitialViewController())
        XCTAssertTrue(controller is UITableViewController)
        XCTAssertEqual(controller.title, "Table")
        XCTAssertNotNil(controller.view)
    }

    @MainActor
    func testLocalizedStringsAndRegistration() async throws {
        for (language, text) in [("en", "Hello"), ("fr", expected["EXPECTED_FRENCH"] ?? "Bonjour"), ("ar", "مرحبا"), ("ku-TR", "Silav")] {
            let path = try XCTUnwrap(Bundle.main.path(forResource: language, ofType: "lproj"))
            let bundle = try XCTUnwrap(Bundle(path: path))
            XCTAssertEqual(bundle.localizedString(forKey: "hello", value: nil, table: nil), text)
        }
        XCTAssertEqual(Bundle.main.object(forInfoDictionaryKey: "NSAccentColorName") as? String, "Accent")
        XCTAssertEqual(Bundle.main.object(forInfoDictionaryKey: "NSWidgetBackgroundColorName") as? String, "Accent")
        let metadata = try XCTUnwrap(Bundle.main.url(forResource: "extract", withExtension: "actionsdata",
                                                    subdirectory: "Metadata.appintents"))
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: metadata)) as? [String: Any])
        let actions = try XCTUnwrap(json["actions"] as? [String: Any])
        XCTAssertNotNil(actions["ProbeIntent"])
    }
}
