// More property wrappers, Core Data's and qualified names.

import CoreData
import SwiftUI

final class Model: ObservableObject {}
final class Delegate: NSObject, UIApplicationDelegate, ObservableObject {}
struct V: View { var body: some View { Text("") } }
struct FocusKey: FocusedValueKey { typealias Value = Int }
extension FocusedValues { var focusKey: Int? { self[FocusKey.self] } }

#Preview {
    @FetchRequest(sortDescriptors: []) var a: FetchedResults<NSManagedObject>
    @SectionedFetchRequest(sectionIdentifier: \NSManagedObject.objectID, sortDescriptors: []) var b: SectionedFetchResults<NSManagedObjectID, NSManagedObject>
    @FocusedObject var c: Model?
    @FocusedValue(\.focusKey) var e: Int?
    @UIApplicationDelegateAdaptor var f: Delegate
    @SwiftUI.State var g = 1
    @Environment(Obs.self) var h
    @State
    var i = 1
    V()
}
@Observable final class Obs {}
