import AppKit
import Foundation

let environment = ProcessInfo.processInfo.environment
guard environment["TYPEVOICE_T23_HELPER"] == "1",
      let expected = environment["TYPEVOICE_T23_TEXT"],
      let readyPath = environment["TYPEVOICE_T23_READY_PATH"],
      let successPath = environment["TYPEVOICE_T23_SUCCESS_PATH"] else {
    exit(2)
}

let application = NSApplication.shared
application.setActivationPolicy(.regular)
application.finishLaunching()

let frame = NSRect(x: 0, y: 0, width: 480, height: 120)
let window = NSWindow(
    contentRect: frame,
    styleMask: [.titled],
    backing: .buffered,
    defer: false
)
window.isReleasedWhenClosed = false
let field = NSTextField(frame: frame)
field.stringValue = ""
window.contentView = field
guard window.makeFirstResponder(field) else {
    exit(3)
}
window.makeKeyAndOrderFront(nil)
application.activate(ignoringOtherApps: true)

try Data("ready".utf8).write(to: URL(fileURLWithPath: readyPath))
let deadline = Date().addingTimeInterval(15)
while Date() < deadline {
    if field.stringValue == expected {
        try Data("success".utf8).write(to: URL(fileURLWithPath: successPath))
        exit(0)
    }
    RunLoop.current.run(until: Date().addingTimeInterval(0.05))
}

fputs("AppKit target did not receive the expected Unicode text\n", stderr)
exit(1)
