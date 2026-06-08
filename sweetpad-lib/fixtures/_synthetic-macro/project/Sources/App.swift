// A no-UI entry point so the macOS app target links cleanly. The fixture exists
// to type-check `Probe.swift`'s macro use, not to run.
@main
enum MacroProbeApp {
    static func main() {
        _ = Probe.value
        _ = Probe.source
    }
}
