import CoreData
import SwiftUI

final class PersistenceController {
    static let shared = PersistenceController()

    let container: NSPersistentContainer

    init() {
        container = NSPersistentContainer(name: "Model")
        container.loadPersistentStores { _, error in
            if let error = error {
                fatalError("Failed to load Core Data stores: \(error)")
            }
        }
    }
}

@main
struct CoreDataGenApp: App {
    let persistence = PersistenceController.shared

    var body: some Scene {
        WindowGroup {
            Text("CoreDataGen")
                .environment(\.managedObjectContext, persistence.container.viewContext)
        }
    }
}
