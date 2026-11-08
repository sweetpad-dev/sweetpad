import AppIntents
import UniformTypeIdentifiers
import SwiftUI

struct FileInputIntent: AppIntent {
    static let title: LocalizedStringResource = "Upload images"

    @Parameter(title: "Images", description: "Images to upload",
               supportedContentTypes: [.image, .jpeg, .png, .gif, .heic],
               inputConnectionBehavior: .connectToPreviousIntentResult)
    var images: [IntentFile]?

    @Parameter(title: "Caption", requestValueDialog: IntentDialog("What caption?"))
    var caption: String?

    func perform() async throws -> some IntentResult & ProvidesDialog & ShowsSnippetView {
        .result(dialog: "Uploaded", view: Text("Done"))
    }
}
