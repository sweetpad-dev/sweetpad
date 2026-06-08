import CoreData
import Foundation

// This file references symbols that exist ONLY in Core Data's generated
// NSManagedObject subclasses (DerivedSources/CoreDataGenerated/Model). None of
// Note, Tag, their @NSManaged properties, addToTags(_:), or fetchRequest() are
// declared anywhere in the committed sources; xcodebuild synthesizes them.
enum CoreDataProbe {
    static func makeNote(in ctx: NSManagedObjectContext) -> Note {
        let note = Note(context: ctx)
        note.title = "First note"
        note.body = "Body text"
        note.createdAt = Date()

        let tag = Tag(context: ctx)
        tag.name = "swift"
        note.addToTags(tag)

        return note
    }

    static func fetchRequests() -> (NSFetchRequest<Note>, NSFetchRequest<Tag>) {
        let notes = Note.fetchRequest()
        let tags = Tag.fetchRequest()
        return (notes, tags)
    }
}
