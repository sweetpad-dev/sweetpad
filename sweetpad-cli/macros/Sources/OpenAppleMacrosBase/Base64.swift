private let base64Alphabet = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".utf8)

extension Collection where Element == UInt8 {
    public func base64EncodedString() -> String {
        String(decoding: base64Encoded(), as: UTF8.self)
    }

    public func base64Encoded() -> [UInt8] {
        var encoded: [UInt8] = []
        encoded.reserveCapacity((count + 2) / 3 * 4)
        var iterator = makeIterator()
        while let first = iterator.next() {
            let second = iterator.next()
            let third = iterator.next()
            let bits = UInt32(first) << 16
                | UInt32(second ?? 0) << 8
                | UInt32(third ?? 0)
            encoded.append(base64Alphabet[Int(bits >> 18)])
            encoded.append(base64Alphabet[Int(bits >> 12 & 63)])
            encoded.append(second != nil ? base64Alphabet[Int(bits >> 6 & 63)] : 61)
            encoded.append(third != nil ? base64Alphabet[Int(bits & 63)] : 61)
        }
        return encoded
    }
}

extension Array where Element == UInt8 {
    public init?(base64EncodedString string: some StringProtocol) {
        self.init(base64Encoded: string.utf8)
    }

    public init?(base64Encoded bytes: some Collection<UInt8>) {
        let encoded = Array(bytes)
        guard encoded.isEmpty || encoded.count >= 4 else { return nil }
        var decoded: [UInt8] = []
        var sawPadding = false
        let completeCount = encoded.count / 4 * 4
        for start in stride(from: 0, to: completeCount, by: 4) {
            let group = encoded[start..<start + 4]
            // Foundation accepts trailing groups consisting entirely of padding.
            if group.allSatisfy({ $0 == 61 }) {
                if decoded.isEmpty { decoded.append(0) }
                sawPadding = true
                continue
            }
            guard !sawPadding else { return nil }
            var values: [UInt8] = []
            for byte in group {
                switch byte {
                case 65...90: values.append(byte - 65)
                case 97...122: values.append(byte - 97 + 26)
                case 48...57: values.append(byte - 48 + 52)
                case 43: values.append(62)
                case 47: values.append(63)
                case 61: values.append(0)
                default: return nil
                }
            }
            guard encoded[start] != 61, encoded[start + 1] != 61 else { return nil }
            decoded.append(values[0] << 2 | values[1] >> 4)
            if encoded[start + 2] != 61 {
                decoded.append(values[1] << 4 | values[2] >> 2)
                if encoded[start + 3] != 61 {
                    decoded.append(values[2] << 6 | values[3])
                }
            } else if encoded[start + 3] != 61 {
                // A third-position padding character resets the remaining bits.
                decoded.append(values[3])
            }
            sawPadding = group.contains(61)
        }
        guard encoded[completeCount...].allSatisfy({ $0 == 61 }) else { return nil }
        self = decoded
    }
}
