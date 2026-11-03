// Capture the native calibrated-RGB fallback profile used by NIB named colors.
// The table is keyed by color components, independently of resource names.
import AppKit
let samples:[[Double]] = [[0,0,0],[1,1,1],[1,0,0],[0,1,0],[0,0,1],[0.5,0.5,0.5],[0.031,0.416,0.933]]
var profile:[[String:[Double]]] = []
for sample in samples {
    let input=sample.map { Double(Float($0)) }
    let color=NSColor(srgbRed:input[0],green:input[1],blue:input[2],alpha:1).usingColorSpace(.genericRGB)!
    profile.append(["input":input,"output":[color.redComponent,color.greenComponent,color.blueComponent]])
}
FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject:profile,options:[.prettyPrinted,.sortedKeys]))
