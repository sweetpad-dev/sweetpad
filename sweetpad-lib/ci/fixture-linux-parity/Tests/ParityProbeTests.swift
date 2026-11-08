import XCTest
import UIKit
import ValueKit
import SwiftUI
import SQLite3
import Darwin
@testable import ParityProbe

final class ParityProbeTests: XCTestCase {
    private var expected: [String: String] { ProcessInfo.processInfo.environment }

    func testLocalizedShortcutModelsArePackaged() throws {
        for locale in ["en", "fr", "ar", "ku-TR"] {
            let folder = Bundle.main.bundleURL.appendingPathComponent("\(locale).lproj/nlu.appintents")
            XCTAssertFalse(try Data(contentsOf: folder.appendingPathComponent("nlu.lzfse")).isEmpty)
            let markers = try FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)
                .filter { $0.pathExtension == "version" }
            XCTAssertEqual(markers.count, 1)
            XCTAssertEqual(try Data(contentsOf: XCTUnwrap(markers.first)), Data([0]))
        }
    }

    @MainActor
    func testSystemFrameworkBindingsAndSQLite() throws {
        let host = UIHostingController(rootView: Text("Linking probe").fixedSize())
        XCTAssertNotNil(host.view)
        let location = try XCTUnwrap(dlopen("/System/Library/Frameworks/CoreLocation.framework/CoreLocation", RTLD_LAZY | RTLD_LOCAL))
        defer { dlclose(location) }
        let accuracy = try XCTUnwrap(dlsym(location, "kCLLocationAccuracyBest"))
        XCTAssertEqual(accuracy.assumingMemoryBound(to: Double.self).pointee, -1)
        XCTAssertEqual(Darwin.noErr, 0)
        var database: OpaquePointer?
        XCTAssertEqual(sqlite3_open(":memory:", &database), SQLITE_OK)
        defer { sqlite3_close(database) }
        var statement: OpaquePointer?
        XCTAssertEqual(sqlite3_prepare_v2(database, "SELECT 6 * 7", -1, &statement, nil), SQLITE_OK)
        defer { sqlite3_finalize(statement) }
        XCTAssertEqual(sqlite3_step(statement), SQLITE_ROW)
        XCTAssertEqual(sqlite3_column_int64(statement, 0), 42)
    }

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
