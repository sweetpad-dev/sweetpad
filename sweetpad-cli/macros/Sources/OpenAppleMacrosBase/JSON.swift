@_spi(PluginMessage) private import SwiftCompilerPluginMessageHandling

public enum JSON {
    /// Encode Swift value to an UInt8 array.
    public static func encode<T: Encodable>(_ value: T) throws -> [UInt8] {
        try SwiftCompilerPluginMessageHandling.JSON.encode(value)
    }

    /// Decode a JSON data to a Swift value.
    public static func decode<T: Decodable>(_ type: T.Type, from json: UnsafeBufferPointer<UInt8>) throws -> T {
        try SwiftCompilerPluginMessageHandling.JSON.decode(type, from: json)
    }
}
