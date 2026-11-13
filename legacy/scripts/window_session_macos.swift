import AppKit
import ApplicationServices
import CoreGraphics
import Darwin
import Foundation
import QuartzCore

let processPathCapacity = 4 * Int(MAXPATHLEN)

enum DriverError: Error, CustomStringConvertible {
    case invalid(String)
    case notReady(String)
    var description: String {
        switch self { case .invalid(let message), .notReady(let message): return message }
    }
}

func require(_ condition: Bool, _ message: String) throws {
    if !condition { throw DriverError.invalid(message) }
}

func unixNanoseconds() -> UInt64 {
    var value = timespec()
    clock_gettime(CLOCK_REALTIME, &value)
    return UInt64(value.tv_sec) * 1_000_000_000 + UInt64(value.tv_nsec)
}

func same(_ first: [String: Any], _ second: [String: Any]) -> Bool {
    NSDictionary(dictionary: first).isEqual(to: second)
}

func unsignedInteger(_ value: Any?, minimum: UInt64 = 0, maximum: UInt64 = UInt64.max) -> UInt64? {
    guard let number = value as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID(),
          ["c", "s", "i", "l", "q", "C", "S", "I", "L", "Q"].contains(String(cString: number.objCType)),
          let integer = UInt64(number.stringValue), integer >= minimum, integer <= maximum else { return nil }
    return integer
}

func isBoolean(_ value: Any?) -> Bool {
    guard let number = value as? NSNumber else { return false }
    return CFGetTypeID(number) == CFBooleanGetTypeID()
}

func finiteNumber(_ value: Any?) -> Bool {
    guard let number = value as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID() else { return false }
    return number.doubleValue.isFinite
}

func processInfo(_ pid: pid_t) throws -> proc_bsdinfo {
    try require(pid > 0, "Invalid owned PID")
    var info = proc_bsdinfo()
    let size = MemoryLayout<proc_bsdinfo>.size
    let count = withUnsafeMutablePointer(to: &info) {
        proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, $0, Int32(size))
    }
    try require(count == Int32(size), "Incomplete or failed kernel process identity read")
    try require(info.pbi_pid == UInt32(pid) && info.pbi_start_tvsec > 0
                && info.pbi_start_tvusec < 1_000_000, "Invalid kernel process lifetime")
    try require((info.pbi_status == UInt32(SRUN) || info.pbi_status == UInt32(SSLEEP))
                && info.pbi_flags & UInt32(PROC_FLAG_INEXIT) == 0, "Owned kernel process is not live and runnable")
    return info
}

func kernelIdentity(_ pid: pid_t, _ executable: String) throws -> [String: Any] {
    let before = try processInfo(pid)
    var buffer = [CChar](repeating: -1, count: processPathCapacity)
    let capacity = buffer.count
    let count = buffer.withUnsafeMutableBufferPointer {
        proc_pidpath(pid, $0.baseAddress, UInt32(capacity))
    }
    guard count > 0, count < Int32(capacity), let end = buffer.firstIndex(of: 0), end > 0,
          Int(count) == end || Int(count) == end + 1,
          let path = String(bytes: buffer.prefix(end).map({ UInt8(bitPattern: $0) }), encoding: .utf8),
          path.hasPrefix("/"), path == executable else {
        throw DriverError.invalid("Incomplete, invalid or wrong kernel executable path")
    }
    let after = try processInfo(pid)
    try require(before.pbi_start_tvsec == after.pbi_start_tvsec
                && before.pbi_start_tvusec == after.pbi_start_tvusec,
                "Owned kernel process lifetime changed during read")
    return ["source": "proc_pidinfo(PROC_PIDTBSDINFO)+proc_pidpath", "pid": Int(pid),
            "start_tvsec": before.pbi_start_tvsec, "start_tvusec": before.pbi_start_tvusec,
            "executable": path]
}

@MainActor
func launchServices(_ pid: pid_t, _ executable: String) throws -> [String: Any] {
    guard let application = NSRunningApplication(processIdentifier: pid) else { return ["registered": false] }
    try require(application.processIdentifier == pid && !application.isTerminated, "Owned application lifetime ended")
    var diagnostics: [String: Any] = ["registered": true]
    if let path = application.executableURL?.standardizedFileURL.path {
        try require(path == executable, "LaunchServices executable disagrees with kernel identity")
        diagnostics["executable"] = path
    }
    if let launched = application.launchDate {
        diagnostics["launch_date"] = ISO8601DateFormatter().string(from: launched)
    }
    return diagnostics
}

func validateExpected(_ provided: [String: Any], _ current: [String: Any]) throws {
    guard Set(provided.keys) == Set(current.keys),
          provided["schema"] as? String == "sims-window-binding-v2",
          unsignedInteger(provided["pid"], minimum: 1, maximum: UInt64(Int32.max)) != nil,
          unsignedInteger(provided["window_id"], minimum: 1, maximum: UInt64(UInt32.max)) != nil,
          isBoolean(provided["frontmost"]), isBoolean(provided["on_screen"]),
          let bounds = provided["bounds"] as? [Any], bounds.count == 4,
          bounds.allSatisfy({ finiteNumber($0) }),
          let providedDisplays = provided["displays"] as? [[String: Any]],
          !providedDisplays.isEmpty,
          let identity = provided["kernel_identity"] as? [String: Any],
          Set(identity.keys) == ["source", "pid", "start_tvsec", "start_tvusec", "executable"],
          unsignedInteger(identity["pid"], minimum: 1, maximum: UInt64(Int32.max)) != nil,
          unsignedInteger(identity["start_tvsec"], minimum: 1) != nil,
          unsignedInteger(identity["start_tvusec"], maximum: 999_999) != nil else {
        throw DriverError.invalid("Malformed or legacy kernel window binding")
    }
    for display in providedDisplays {
        try require(Set(display.keys) == ["display_id", "mode_id", "width", "height", "pixel_width",
                                         "pixel_height", "nominal_refresh_hz", "maximum_frames_per_second",
                                         "builtin", "active", "asleep"], "Malformed display binding")
        try require(unsignedInteger(display["display_id"], minimum: 1, maximum: UInt64(UInt32.max)) != nil
                    && unsignedInteger(display["mode_id"], maximum: UInt64(UInt32.max)) != nil,
                    "Malformed display identity")
        for name in ["width", "height", "pixel_width", "pixel_height", "maximum_frames_per_second"] {
            try require(unsignedInteger(display[name], maximum: UInt64(Int.max)) != nil,
                        "Malformed display integer")
        }
        for name in ["builtin", "active", "asleep"] {
            try require(isBoolean(display[name]), "Malformed display Boolean")
        }
        try require(finiteNumber(display["nominal_refresh_hz"]), "Malformed display refresh rate")
    }
    try require(same(provided, current), "Stale helper binding")
}

// Data validation precedes every process/window lookup. This expected lifetime
// is supplied by a future retained-execution adapter, never discovered here.
func validateTarget(_ value: Any?, _ pid: pid_t, _ executable: String) throws -> (CGWindowID, [String: Any]) {
    guard let target = value as? [String: Any],
          Set(target.keys) == ["schema", "window_id", "kernel_identity"],
          target["schema"] as? String == "sims-window-target-v1",
          let window = unsignedInteger(target["window_id"], minimum: 1, maximum: UInt64(UInt32.max)),
          let identity = target["kernel_identity"] as? [String: Any],
          Set(identity.keys) == ["source", "pid", "start_tvsec", "start_tvusec", "executable"],
          identity["source"] as? String == "proc_pidinfo(PROC_PIDTBSDINFO)+proc_pidpath",
          unsignedInteger(identity["pid"], minimum: 1, maximum: UInt64(Int32.max)) == UInt64(pid),
          unsignedInteger(identity["start_tvsec"], minimum: 1) != nil,
          unsignedInteger(identity["start_tvusec"], maximum: 999_999) != nil,
          identity["executable"] as? String == executable else {
        throw DriverError.invalid("Missing or malformed exact window and expected kernel lifetime")
    }
    return (CGWindowID(window), identity)
}

@MainActor
func displays(_ bounds: CGRect) throws -> [[String: Any]] {
    var identifiers = [CGDirectDisplayID](repeating: 0, count: 8)
    var count: UInt32 = 0
    let result = identifiers.withUnsafeMutableBufferPointer {
        CGGetDisplaysWithRect(bounds, 8, $0.baseAddress, &count)
    }
    try require(result == .success && count > 0 && count <= 8, "Cannot identify owned-window displays")
    return try identifiers.prefix(Int(count)).sorted().map { identifier in
        guard let mode = CGDisplayCopyDisplayMode(identifier) else {
            throw DriverError.invalid("Cannot read owned-window display mode")
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

@MainActor
func windowBinding(_ pid: pid_t, _ executable: String, _ identity: [String: Any], _ targetWindow: CGWindowID) throws -> [String: Any]? {
    try require(targetWindow != 0, "Missing exact owned window ID")
    guard let records = CGWindowListCopyWindowInfo(.optionIncludingWindow, targetWindow)
        as? [[String: Any]] else { throw DriverError.invalid("Cannot inspect exact owned window") }
    if records.isEmpty { return nil }
    try require(records.count == 1, "Exact window query returned unexpected records")
    let record = records[0]
    guard unsignedInteger(record[kCGWindowOwnerPID as String], minimum: 1, maximum: UInt64(Int32.max)) == UInt64(pid),
          unsignedInteger(record[kCGWindowLayer as String]) == 0,
          let identifier = unsignedInteger(record[kCGWindowNumber as String], minimum: 1, maximum: UInt64(UInt32.max)),
          identifier == UInt64(targetWindow),
          let values = record[kCGWindowBounds as String] as? [String: Double],
          let bounds = CGRect(dictionaryRepresentation: values as CFDictionary),
          [bounds.minX, bounds.minY, bounds.width, bounds.height].allSatisfy({ $0.isFinite }),
          bounds.width > 100, bounds.height > 100,
          isBoolean(record[kCGWindowIsOnscreen as String]),
          (record[kCGWindowIsOnscreen as String] as? Bool) == true,
          finiteNumber(record[kCGWindowAlpha as String]),
          let alpha = record[kCGWindowAlpha as String] as? Double, alpha > 0, alpha <= 1 else {
        throw DriverError.invalid("Invalid owned window geometry or visibility")
    }
    // CGEvent posts target a PID, not a window. Require the one PID-scoped AX
    // window to be focused and to match this exact CG target's geometry. AX
    // reads also avoid NSRunningApplication's run-loop-cached activity state.
    _ = try focusedOwnedWindow(pid, [Double(bounds.minX), Double(bounds.minY), Double(bounds.width), Double(bounds.height)])
    return ["schema": "sims-window-binding-v2", "pid": Int(pid), "window_id": identifier,
            "executable": executable, "kernel_identity": identity,
            "frontmost": true, "on_screen": true,
            "display_scope": "Current CoreGraphics mode and NSScreen maximum; not instantaneous refresh",
            "displays": try displays(bounds),
            "bounds": [bounds.minX, bounds.minY, bounds.width, bounds.height]]
}

@MainActor
func binding(_ pid: pid_t, _ executable: String, _ targetWindow: CGWindowID, _ targetIdentity: [String: Any]) throws -> [String: Any]? {
    let identity = try kernelIdentity(pid, executable)
    try require(same(identity, targetIdentity), "Owned process differs from expected retained lifetime")
    _ = try launchServices(pid, executable)
    guard let first = try windowBinding(pid, executable, identity, targetWindow) else { return nil }
    let confirmedIdentity = try kernelIdentity(pid, executable)
    try require(same(identity, confirmedIdentity) && same(confirmedIdentity, targetIdentity), "Owned kernel process identity changed")
    _ = try launchServices(pid, executable)
    guard let second = try windowBinding(pid, executable, confirmedIdentity, targetWindow), same(first, second) else {
        throw DriverError.invalid("Focus, geometry, display or window changed during binding")
    }
    return first
}

func attribute(_ element: AXUIElement, _ name: String) throws -> CFTypeRef {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success,
          let value else { throw DriverError.invalid("Missing owned Accessibility attribute: \(name)") }
    return value
}

@MainActor
func focusedOwnedWindow(_ pid: pid_t, _ bounds: [Double]) throws -> AXUIElement {
    try require(AXIsProcessTrusted(), "Existing Accessibility access required; no prompt is requested")
    let application = AXUIElementCreateApplication(pid)
    guard let windows = try attribute(application, kAXWindowsAttribute) as? [AXUIElement],
          windows.count == 1 else {
        throw DriverError.invalid("Ambiguous owned Accessibility window")
    }
    let target = windows[0]
    let focused = try attribute(application, kAXFocusedWindowAttribute)
    try require(CFGetTypeID(focused) == AXUIElementGetTypeID() && CFEqual(focused, target),
                "Exact owned window is not the unambiguous focused Accessibility window")
    var owner: pid_t = 0
    try require(AXUIElementGetPid(target, &owner) == .success && owner == pid, "Wrong focused-window owner")
    let positionValue = try attribute(target, kAXPositionAttribute)
    let sizeValue = try attribute(target, kAXSizeAttribute)
    try require(CFGetTypeID(positionValue) == AXValueGetTypeID()
                && CFGetTypeID(sizeValue) == AXValueGetTypeID(), "Invalid Accessibility geometry types")
    var position = CGPoint.zero
    var size = CGSize.zero
    try require(AXValueGetValue(positionValue as! AXValue, .cgPoint, &position)
                && AXValueGetValue(sizeValue as! AXValue, .cgSize, &size)
                && [Double(position.x), Double(position.y), Double(size.width), Double(size.height)] == bounds,
                "Accessibility and on-screen window geometry disagree")
    let frontmost = try attribute(application, kAXFrontmostAttribute)
    guard isBoolean(frontmost), let active = frontmost as? Bool else {
        throw DriverError.invalid("Malformed owned Accessibility frontmost state")
    }
    if !active { throw DriverError.notReady("foreground_application_unavailable") }
    return target
}

@MainActor
func closeOwnedWindow(_ pid: pid_t, _ executable: String, _ expected: [String: Any], _ targetWindow: CGWindowID, _ targetIdentity: [String: Any]) throws {
    guard let bounds = expected["bounds"] as? [Double] else {
        throw DriverError.invalid("Missing exact owned window geometry")
    }
    let target = try focusedOwnedWindow(pid, bounds)
    var owner: pid_t = 0
    try require(AXUIElementGetPid(target, &owner) == .success && owner == pid, "Wrong close-window owner")
    let closeValue = try attribute(target, kAXCloseButtonAttribute)
    try require(CFGetTypeID(closeValue) == AXUIElementGetTypeID(), "Invalid owned close button")
    guard let current = try binding(pid, executable, targetWindow, targetIdentity), same(current, expected) else {
        throw DriverError.invalid("Window identity changed before close")
    }
    try require(AXUIElementPerformAction(closeValue as! AXUIElement, kAXPressAction as CFString) == .success,
                "Owned close action failed")
}

@MainActor
func run() -> Int32 {
    var posts = [[String: Any]]()
    var heldRelease: CGEvent?
    var expected: [String: Any]?
    var pid: pid_t = 0
    var executable = ""
    var targetWindow: CGWindowID = 0
    var targetIdentity = [String: Any]()
    var response = [String: Any]()
    var result: Int32 = 0
    func post(_ event: CGEvent, down: Bool) throws {
        guard let expected, let current = try binding(pid, executable, targetWindow, targetIdentity), same(current, expected) else {
            throw DriverError.invalid("Focus, PID lifetime, geometry or window identity changed before post")
        }
        let unixBefore = unixNanoseconds()
        let hostBefore = DispatchTime.now().uptimeNanoseconds
        let quartzBefore = CACurrentMediaTime()
        event.postToPid(pid)
        let quartzAfter = CACurrentMediaTime()
        let hostAfter = DispatchTime.now().uptimeNanoseconds
        let unixAfter = unixNanoseconds()
        posts.append(["down": down, "unix_before_ns": unixBefore, "unix_after_ns": unixAfter,
                      "host_before_ns": hostBefore, "host_after_ns": hostAfter,
                      "clock_api": "QuartzCore.CACurrentMediaTime",
                      "quartz_before_seconds": quartzBefore, "quartz_after_seconds": quartzAfter])
    }
    do {
        let data = FileHandle.standardInput.readData(ofLength: 16385)
        try require(data.count <= 16384, "Oversized helper request")
        guard let request = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let operation = request["operation"] as? String,
              let rawPID = unsignedInteger(request["pid"], minimum: 1, maximum: UInt64(Int32.max)),
              let path = request["executable"] as? String, path.hasPrefix("/"), !path.contains("\0") else {
            throw DriverError.invalid("Invalid helper request")
        }
        pid = pid_t(rawPID)
        executable = path
        try require(["observe", "key", "release", "close"].contains(operation), "Unknown helper operation")
        let keys: Set<String> = operation == "key"
            ? ["operation", "pid", "executable", "target", "binding", "code", "press_ms"]
            : (operation == "release" && request["code"] != nil
               ? ["operation", "pid", "executable", "target", "binding", "code"]
            : (request["binding"] == nil ? ["operation", "pid", "executable", "target"]
               : ["operation", "pid", "executable", "target", "binding"]))
        try require(Set(request.keys) == keys, "Unknown helper request fields")
        (targetWindow, targetIdentity) = try validateTarget(request["target"], pid, executable)
        let identity = try kernelIdentity(pid, executable)
        try require(same(identity, targetIdentity), "Owned process differs from expected retained lifetime")
        let observed: [String: Any]?
        do {
            observed = try binding(pid, executable, targetWindow, targetIdentity)
        } catch DriverError.notReady(let reason) {
            try require(operation == "observe" && request["binding"] == nil,
                        "Owned foreground registration disappeared after binding")
            try require(same(identity, try kernelIdentity(pid, executable)), "Owned kernel process identity changed")
            let output = try JSONSerialization.data(withJSONObject: ["status": "not_ready", "reason": reason,
                                                                     "kernel_identity": identity])
            print(String(decoding: output, as: UTF8.self))
            return 0
        }
        guard let current = observed else {
            try require(operation == "observe" && request["binding"] == nil, "Owned window disappeared")
            try require(same(identity, try kernelIdentity(pid, executable)), "Owned kernel process identity changed")
            let output = try JSONSerialization.data(withJSONObject: ["status": "not_ready",
                "reason": "owned_window_unavailable", "kernel_identity": identity])
            print(String(decoding: output, as: UTF8.self))
            return 0
        }
        try require(same(identity, current["kernel_identity"] as? [String: Any] ?? [:]),
                    "Owned kernel process identity changed before binding")
        if let value = request["binding"] {
            guard let provided = value as? [String: Any] else { throw DriverError.invalid("Malformed helper binding") }
            try validateExpected(provided, current)
        } else {
            try require(operation == "observe", "Input requires an existing owned window binding")
        }
        expected = current
        response = ["operation": operation, "binding": current, "launch_services": try launchServices(pid, executable)]
        if operation == "key" || operation == "release" {
            let requestedCode = request["code"] ?? (operation == "release" ? 124 : -1)
            guard let code = unsignedInteger(requestedCode), code == 124 || code == 36 else {
                throw DriverError.invalid("Only explicit ArrowRight or Enter release is supported")
            }
            try require(CGPreflightPostEventAccess(), "Existing event-posting access required; no prompt is requested")
            guard let release = CGEvent(keyboardEventSource: nil, virtualKey: CGKeyCode(code), keyDown: false) else {
                throw DriverError.invalid("Cannot construct owned key release")
            }
            if operation == "key" {
                try require((unsignedInteger(request["code"]) == 124 || unsignedInteger(request["code"]) == 36)
                            && unsignedInteger(request["press_ms"]) == 20,
                            "Only bounded explicit ArrowRight or Enter/20ms input is supported")
                guard let press = CGEvent(keyboardEventSource: nil, virtualKey: CGKeyCode(code), keyDown: true) else {
                    throw DriverError.invalid("Cannot construct owned key press")
                }
                heldRelease = release
                try post(press, down: true)
                usleep(20_000)
                response["code"] = code
                response["press_ms"] = 20
            }
            try post(release, down: false)
            heldRelease = nil
            response["released"] = true
            response["code"] = code
        } else if operation == "close" {
            try closeOwnedWindow(pid, executable, current, targetWindow, targetIdentity)
        } else {
            try require(operation == "observe", "Unknown helper operation")
        }
    } catch {
        response["error"] = String(describing: error)
        FileHandle.standardError.write(Data("\(error)\n".utf8))
        result = 1
    }
    if let heldRelease {
        do {
            try post(heldRelease, down: false)
            response["released"] = true
        } catch {
            response["release_error"] = String(describing: error)
            result = 1
        }
    }
    response["posts"] = posts
    if let output = try? JSONSerialization.data(withJSONObject: response, options: [.sortedKeys]) {
        print(String(decoding: output, as: UTF8.self))
    } else {
        result = 1
    }
    return result
}

Task { @MainActor in exit(run()) }
dispatchMain()
