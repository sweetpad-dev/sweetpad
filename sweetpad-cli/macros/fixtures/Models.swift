// The SwiftData shapes the corpus app uses (sweetpad-lib/corpus/_swiftdata-app).

import Foundation
import SwiftData

enum Genre: String, Codable, CaseIterable {
    case fiction, science
}

struct Info: Codable, Hashable {
    var isbn: String
    var edition: Int
}

@Model
final class Author {
    @Attribute(.unique) var name: String
    var born: Date?
    @Relationship(deleteRule: .cascade, inverse: \Book.author) var books: [Book] = []

    init(name: String, born: Date? = nil) {
        self.name = name
        self.born = born
    }
}

@Model
final class Book {
    #Unique<Book>([\.title, \.pages])
    #Index<Book>([\.title], [\.pages, \.rating])

    var title: String
    var pages: Int = 0
    var rating: Double?
    var tags: [String] = []
    var genre: Genre = Genre.fiction
    var info: Info?
    var author: Author?
    @Transient var cached: String = ""
    @Attribute(.externalStorage) var cover: Data?
    @Attribute(originalName: "subtitle") var tagline: String?
    var added = Date(timeIntervalSinceReferenceDate: 0)

    var summary: String { "\(title) (\(pages))" }

    init(title: String, pages: Int, author: Author? = nil) {
        self.title = title
        self.pages = pages
        self.author = author
    }
}

@Model
class Shelf {
    var label: String
    @Relationship(minimumModelCount: 0, maximumModelCount: 10) var books: [Book]?
    init(label: String) { self.label = label }
}

@ModelActor
actor Importer {
    func count() throws -> Int {
        try modelContext.fetchCount(FetchDescriptor<Book>())
    }

    func add(_ title: String) throws {
        modelContext.insert(Book(title: title, pages: 1))
        try modelContext.save()
    }
}
