import Foundation

func sharedWork() {
    #if os(iOS)
    print(PhoneDatabase.shared.name)
    #endif
}
