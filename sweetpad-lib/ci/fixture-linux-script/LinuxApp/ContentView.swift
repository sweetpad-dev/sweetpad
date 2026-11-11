import Darwin
import PhotosUI
import SwiftUI

extension EnvironmentValues {
    @Entry var optionalBuildMarker: String?
    @Entry var typedBuildMarker: Optional<Int> = nil
    @Entry var explicitBuildMarker: Int = 7
}

func verifyEntryDefaults() {
    var values = EnvironmentValues()
    precondition(values.optionalBuildMarker == nil)
    precondition(values.typedBuildMarker == nil)
    precondition(values.explicitBuildMarker == 7)
    values.optionalBuildMarker = "assigned"
    values.typedBuildMarker = 42
    precondition(values.optionalBuildMarker == "assigned")
    precondition(values.typedBuildMarker == 42)
    print("ENTRYCHECK defaults and assignments passed")
    fflush(nil)
}

struct ContentView: View {
    @State private var photo: PhotosPickerItem?

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "globe")
                .imageScale(.large)
                .foregroundStyle(.tint)
            Text(Greeting.message + schemeGeneratedValue)
            // PhotosPicker is in the overlay PhotosUI has for SwiftUI, which
            // the open-source compiler loads only when asked to.
            PhotosPicker("Pick a photo", selection: $photo)
        }
        .padding()
    }
}
