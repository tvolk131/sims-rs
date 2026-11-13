import AppKit
import ApplicationServices
import CoreGraphics
import Foundation
import ScreenCaptureKit

enum DriverError: Error, CustomStringConvertible {
    case invalid(String)
    var description: String {
        switch self { case .invalid(let message): return message }
    }
}

func argument(_ name: String) throws -> String {
    guard let index = CommandLine.arguments.firstIndex(of: name),
          index + 1 < CommandLine.arguments.count else {
        throw DriverError.invalid("Missing argument: \(name)")
    }
    return CommandLine.arguments[index + 1]
}

func number(_ name: String) throws -> Double {
    guard let value = Double(try argument(name)), value.isFinite else {
        throw DriverError.invalid("Invalid numeric argument: \(name)")
    }
    return value
}

func emit(_ value: [String: Any]) throws {
    let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    print(String(decoding: data, as: UTF8.self))
}

func window(_ pid: pid_t) throws -> (CGWindowID, CGRect) {
    let records = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
        as? [[String: Any]] ?? []
    let owned = records.filter {
        ($0[kCGWindowOwnerPID as String] as? Int) == Int(pid)
            && ($0[kCGWindowLayer as String] as? Int) == 0
    }
    let candidates: [(CGWindowID, CGRect)] = owned.compactMap { record in
        if let identifier = record[kCGWindowNumber as String] as? UInt32,
           let values = record[kCGWindowBounds as String] as? [String: Double],
           let bounds = CGRect(dictionaryRepresentation: values as CFDictionary), bounds.width > 100, bounds.height > 100 {
            return (identifier, bounds)
        }
        return nil
    }
    guard candidates.count == 1 else {
        throw DriverError.invalid("Expected one visible application window for PID \(pid); found \(candidates.count)")
    }
    return candidates[0]
}

func focus(_ pid: pid_t) throws {
    guard let application = NSRunningApplication(processIdentifier: pid) else {
        throw DriverError.invalid("Target application is not running")
    }
    _ = application.activate(options: [])
    if NSWorkspace.shared.frontmostApplication?.processIdentifier != pid && AXIsProcessTrusted() {
        let accessible = AXUIElementCreateApplication(pid)
        var settable = DarwinBoolean(false)
        if AXUIElementIsAttributeSettable(accessible, kAXFrontmostAttribute as CFString, &settable) == .success,
           settable.boolValue {
            _ = AXUIElementSetAttributeValue(accessible, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
        }
    }
    let deadline = Date().addingTimeInterval(2)
    while NSWorkspace.shared.frontmostApplication?.processIdentifier != pid && Date() < deadline {
        RunLoop.current.run(until: Date().addingTimeInterval(0.01))
    }
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else {
        throw DriverError.invalid("Refusing input: target application is not frontmost")
    }
}

func displayConfiguration(_ bounds: CGRect) throws -> [[String: Any]] {
    var identifiers = [CGDirectDisplayID](repeating: 0, count: 8)
    var count: UInt32 = 0
    let result = identifiers.withUnsafeMutableBufferPointer {
        CGGetDisplaysWithRect(bounds, 8, $0.baseAddress, &count)
    }
    guard result == .success, count > 0, count <= 8 else {
        throw DriverError.invalid("Cannot identify the target window's display configuration")
    }
    return try identifiers.prefix(Int(count)).map { identifier in
        guard let mode = CGDisplayCopyDisplayMode(identifier) else {
            throw DriverError.invalid("Cannot read the target window's current display mode")
        }
        let screen = NSScreen.screens.first {
            ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value == identifier
        }
        return ["display_id": identifier, "mode_id": mode.ioDisplayModeID,
                "width": mode.width, "height": mode.height,
                "pixel_width": mode.pixelWidth, "pixel_height": mode.pixelHeight,
                "nominal_refresh_hz": mode.refreshRate,
                "maximum_frames_per_second": screen?.maximumFramesPerSecond ?? 0,
                "builtin": CGDisplayIsBuiltin(identifier) != 0,
                "active": CGDisplayIsActive(identifier) != 0,
                "asleep": CGDisplayIsAsleep(identifier) != 0]
    }
}

func postPointer(_ kind: CGEventType, at point: CGPoint, window identifier: CGWindowID, pid: pid_t) throws {
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else {
        throw DriverError.invalid("Refusing pointer input: target application lost focus")
    }
    let (currentIdentifier, bounds) = try window(pid)
    guard currentIdentifier == identifier, bounds.contains(point) else {
        throw DriverError.invalid("Refusing pointer input: target window moved or changed")
    }
    var hit: AXUIElement?
    var hitPid: pid_t = 0
    guard AXUIElementCopyElementAtPosition(AXUIElementCreateSystemWide(), Float(point.x), Float(point.y), &hit) == .success,
          let hit, AXUIElementGetPid(hit, &hitPid) == .success, hitPid == pid else {
        throw DriverError.invalid("Refusing pointer input: accessibility hit testing does not identify the target application")
    }
    guard let event = CGEvent(mouseEventSource: nil, mouseType: kind,
                              mouseCursorPosition: point, mouseButton: .left) else {
        throw DriverError.invalid("Cannot construct native pointer event")
    }
    event.setIntegerValueField(.mouseEventClickState, value: kind == .mouseMoved ? 0 : 1)
    event.post(tap: .cgSessionEventTap)
}

func postInputRelease(_ release: CGEvent, pid: pid_t) {
    if release.type == .leftMouseUp {
        release.post(tap: .cgSessionEventTap)
    } else if release.type == .keyUp {
        release.postToPid(pid)
    }
}

@available(macOS 14.0, *)
func captureWindow(_ identifier: CGWindowID, destination: String) async throws {
    let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
    guard let target = content.windows.first(where: { $0.windowID == identifier }) else {
        throw DriverError.invalid("Target window is no longer shareable")
    }
    let filter = SCContentFilter(desktopIndependentWindow: target)
    let configuration = SCStreamConfiguration()
    configuration.width = Int((filter.contentRect.width * CGFloat(filter.pointPixelScale)).rounded())
    configuration.height = Int((filter.contentRect.height * CGFloat(filter.pointPixelScale)).rounded())
    configuration.showsCursor = false
    configuration.ignoreShadowsSingleWindow = true
    let image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: configuration)
    guard let data = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
        throw DriverError.invalid("Cannot encode window capture")
    }
    try data.write(to: URL(fileURLWithPath: destination), options: .withoutOverwriting)
}

@MainActor
func main() async throws {
    guard CommandLine.arguments.count > 1 else { throw DriverError.invalid("Missing operation") }
    let operation = CommandLine.arguments[1]
    if operation == "permissions" {
        if CommandLine.arguments.contains("--request") {
            _ = CGRequestPostEventAccess()
            _ = AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue(): true] as CFDictionary)
        }
        try emit(["event_post_allowed": CGPreflightPostEventAccess(),
                  "screen_capture_allowed": CGPreflightScreenCaptureAccess(),
                  "accessibility_trusted": AXIsProcessTrusted()])
        return
    }
    guard ["window", "capture", "key", "click", "move", "close", "release"].contains(operation) else {
        throw DriverError.invalid("Unknown operation: \(operation)")
    }
    guard let pid = Int32(try argument("--pid")), pid > 0 else {
        throw DriverError.invalid("Invalid target PID")
    }
    if operation == "release" {
        guard CGPreflightPostEventAccess() else {
            throw DriverError.invalid("Native input cleanup requires event-posting permission")
        }
        let release: CGEvent?
        if CommandLine.arguments.contains("--code") {
            guard let code = UInt16(try argument("--code")) else {
                throw DriverError.invalid("Invalid cleanup key code")
            }
            release = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false)
        } else {
            release = CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp,
                              mouseCursorPosition: CGEvent(source: nil)?.location ?? .zero,
                              mouseButton: .left)
        }
        guard let release else { throw DriverError.invalid("Cannot construct native input cleanup") }
        postInputRelease(release, pid: pid)
        try emit(["operation": operation, "pid": pid, "release_attempted": true])
        return
    }
    if operation == "window" { try focus(pid) }
    let (identifier, bounds) = try window(pid)
    if operation == "window" {
        try emit(["window_id": identifier, "pid": pid,
                  "bounds": [bounds.minX, bounds.minY, bounds.width, bounds.height],
                  "displays": try displayConfiguration(bounds),
                  "display_scope": "Current CoreGraphics mode and NSScreen maximum; not instantaneous refresh"])
        return
    }
    if operation == "capture" {
        guard CGPreflightScreenCaptureAccess() else {
            throw DriverError.invalid("macOS Screen Recording permission is required")
        }
        let destination = try argument("--path")
        guard !FileManager.default.fileExists(atPath: destination) else {
            throw DriverError.invalid("Refusing to overwrite a capture")
        }
        guard #available(macOS 14.0, *) else {
            throw DriverError.invalid("Native window capture requires macOS 14 or newer")
        }
        try await captureWindow(identifier, destination: destination)
        try emit(["window_id": identifier, "path": destination])
        return
    }
    guard CGPreflightPostEventAccess() else {
        throw DriverError.invalid("macOS Accessibility/event-posting permission is required; run permissions --request and approve the requesting app in System Settings")
    }
    let pressMilliseconds = CommandLine.arguments.contains("--press-ms")
        ? try number("--press-ms") : (operation == "key" ? 20.0 : 80.0)
    guard pressMilliseconds >= 1, pressMilliseconds <= 1000 else {
        throw DriverError.invalid("Press duration must be between 1 and 1000 milliseconds")
    }
    try focus(pid)
    let (focusedIdentifier, focusedBounds) = try window(pid)
    guard focusedIdentifier == identifier, focusedBounds == bounds else {
        throw DriverError.invalid("Target window moved or changed while focusing; refusing stale input")
    }
    var heldRelease: CGEvent?
    defer {
        if let heldRelease { postInputRelease(heldRelease, pid: pid) }
    }
    if operation == "key" {
        guard let code = UInt16(try argument("--code")) else { throw DriverError.invalid("Invalid key code") }
        guard let release = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false) else {
            throw DriverError.invalid("Cannot construct keyboard release")
        }
        for down in [true, false] {
            guard let event = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down) else {
                throw DriverError.invalid("Cannot construct keyboard event")
            }
            if down { heldRelease = release }
            event.postToPid(pid)
            if !down { heldRelease = nil }
            if down { usleep(useconds_t(pressMilliseconds * 1000)) }
        }
    } else if operation == "click" || operation == "move" || operation == "close" {
        if operation == "close" {
            let point = CGPoint(x: bounds.minX + 14, y: bounds.minY + 16)
            guard let release = CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp,
                                       mouseCursorPosition: point, mouseButton: .left) else {
                throw DriverError.invalid("Cannot construct close-button release")
            }
            for kind in [CGEventType.leftMouseDown, .leftMouseUp] {
                if kind == .leftMouseDown { heldRelease = release }
                try postPointer(kind, at: point, window: identifier, pid: pid)
                if kind == .leftMouseUp { heldRelease = nil }
                if kind == .leftMouseDown { usleep(useconds_t(pressMilliseconds * 1000)) }
            }
            try emit(["operation": operation, "pid": pid, "window_id": identifier,
                      "press_ms": pressMilliseconds])
            return
        }
        let contentWidth = try number("--content-width")
        let contentHeight = try number("--content-height")
        let position = try CGPoint(x: number("--x"), y: number("--y"))
        guard contentWidth > 0, abs(contentWidth - bounds.width) < 1,
              contentHeight > 0, contentHeight <= bounds.height,
              bounds.height - contentHeight <= 80,
              position.x >= 0, position.x < contentWidth,
              position.y >= 0, position.y < contentHeight else {
            throw DriverError.invalid("Input point lies outside the target window content")
        }
        let point = CGPoint(x: bounds.minX + position.x,
                            y: bounds.minY + bounds.height - contentHeight + position.y)
        guard let release = CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp,
                                   mouseCursorPosition: point, mouseButton: .left) else {
            throw DriverError.invalid("Cannot construct pointer release")
        }
        let events: [CGEventType] = operation == "click" ? [.mouseMoved, .leftMouseDown, .leftMouseUp] : [.mouseMoved]
        for kind in events {
            if kind == .leftMouseDown { heldRelease = release }
            try postPointer(kind, at: point, window: identifier, pid: pid)
            if kind == .leftMouseUp { heldRelease = nil }
            if kind == .leftMouseDown { usleep(useconds_t(pressMilliseconds * 1000)) }
        }
    } else {
        throw DriverError.invalid("Unknown operation: \(operation)")
    }
    try emit(["operation": operation, "pid": pid, "window_id": identifier,
              "press_ms": pressMilliseconds])
}

Task { @MainActor in
    do { try await main(); exit(0) } catch {
        FileHandle.standardError.write(Data("\(error)\n".utf8))
        exit(1)
    }
}
dispatchMain()
