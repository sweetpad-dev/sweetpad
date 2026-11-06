import PhotosUI
import SwiftUI

struct ContentView: View {
    @State private var photo: PhotosPickerItem?

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "globe")
                .imageScale(.large)
                .foregroundStyle(.tint)
            Text(Greeting.message)
            // PhotosPicker is in the overlay PhotosUI has for SwiftUI, which
            // the open-source compiler loads only when asked to.
            PhotosPicker("Pick a photo", selection: $photo)
        }
        .padding()
    }
}
