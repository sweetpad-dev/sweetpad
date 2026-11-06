// Transformable attributes, and an #Index whose arguments don't type-check.

import Foundation
import SwiftData
import UIKit

@Model
final class Conf: Identifiable, Hashable {
    @Attribute(.transformable(by: UIColorValueTransformer.self)) var color: UIColor = UIColor.red
    @Attribute(.transformable(by: "NSSecureUnarchiveFromData")) var other: [String]? = nil
    var plain: Int = 0
    #Index<Conf>(.binary(\.plain), .rtree(\.plain))
    init() {}
}

final class UIColorValueTransformer: ValueTransformer {}
