import SwiftyJSON

struct PodProbe {
    static func go() {
        let j = JSON(["k": "v"])
        _ = j["k"].stringValue
    }
}
