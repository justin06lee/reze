import Foundation

// The macro template language, the same as the desktop app's
// (`src/lib/template.ts`), so that a macro means on the phone what it means on
// the Mac it was written on:
//
//   {{name}}              a variable you get asked for
//   {{name|a default}}    the same, pre-filled
//   {{> other macro}}     another macro's body, inlined by name
//   {{clipboard}}         built-ins, filled in automatically, never asked for

public enum Builtin: String, CaseIterable, Sendable {
    case clipboard, date, time, datetime
}

public struct Variable: Hashable, Sendable {
    public var name: String
    public var fallback: String
}

public struct Expanded: Hashable, Sendable {
    public var text: String
    /// Include targets that are not a macro — surfaced, not silently dropped.
    public var missing: [String]
    /// Include names that form a cycle, or nest too deep to be anything else.
    public var cyclic: [String]
}

public enum Template {
    static let maxIncludeDepth = 10

    private static let token = try! NSRegularExpression(pattern: #"\{\{\s*([^{}]*?)\s*\}\}"#)

    /// Every `{{ … }}` in `text`: the whole of it, and what is inside.
    private static func tokens(in text: String) -> [(whole: Range<String.Index>, inner: String)] {
        let all = NSRange(text.startIndex..., in: text)
        return token.matches(in: text, range: all).compactMap { match in
            guard let whole = Range(match.range, in: text),
                  let inner = Range(match.range(at: 1), in: text)
            else { return nil }
            return (whole, String(text[inner]))
        }
    }

    /// `text` with each token replaced by what `replacement` makes of it —
    /// `nil` leaves the token as it was written.
    private static func replacing(_ text: String, _ replacement: (String) -> String?) -> String {
        var out = ""
        var at = text.startIndex
        for (whole, inner) in tokens(in: text) {
            out += text[at..<whole.lowerBound]
            out += replacement(inner) ?? String(text[whole])
            at = whole.upperBound
        }
        out += text[at...]
        return out
    }

    private static func normalize(_ name: String) -> String {
        name.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }

    /// The name and the default in `name|a default`.
    private static func split(_ raw: String) -> (name: String, fallback: String) {
        guard let pipe = raw.firstIndex(of: "|") else { return (raw, "") }
        return (
            raw[..<pipe].trimmingCharacters(in: .whitespacesAndNewlines),
            raw[raw.index(after: pipe)...].trimmingCharacters(in: .whitespacesAndNewlines)
        )
    }

    /// Splice `{{> name}}` includes into the body, depth-first.
    ///
    /// `trail` carries the chain of includes, so a macro that includes itself
    /// by way of others is reported rather than followed until the stack gives
    /// out.
    public static func expandIncludes(
        _ body: String,
        macros: [Macro],
        trail: [String] = [],
        depth: Int = 0
    ) -> Expanded {
        var missing: [String] = []
        var cyclic: [String] = []

        let text = replacing(body) { inner in
            guard inner.hasPrefix(">") else { return nil }
            let target = inner.dropFirst().trimmingCharacters(in: .whitespacesAndNewlines)
            let key = normalize(target)

            if trail.contains(key) {
                cyclic.append(target)
                return "{{! cycle: \(target) }}"
            }
            if depth >= maxIncludeDepth {
                cyclic.append(target)
                return "{{! too deep: \(target) }}"
            }
            guard let found = macros.first(where: { normalize($0.name) == key }) else {
                missing.append(target)
                return "{{! missing: \(target) }}"
            }

            let nested = expandIncludes(found.body, macros: macros, trail: trail + [key], depth: depth + 1)
            missing += nested.missing
            cyclic += nested.cyclic
            return nested.text
        }

        return Expanded(text: text, missing: missing, cyclic: cyclic)
    }

    /// The variables still to be filled in — built-ins and includes left out.
    public static func variables(in text: String) -> [Variable] {
        var found: [Variable] = []
        for (_, inner) in tokens(in: text) {
            let raw = inner.trimmingCharacters(in: .whitespacesAndNewlines)
            if raw.isEmpty || raw.hasPrefix(">") || raw.hasPrefix("!") { continue }
            let (name, fallback) = split(raw)
            if name.isEmpty || Builtin(rawValue: name) != nil { continue }
            // The first one wins, so the default nearest the top of the macro
            // is the one that is shown.
            if !found.contains(where: { $0.name == name }) {
                found.append(Variable(name: name, fallback: fallback))
            }
        }
        return found
    }

    public static func builtins(in text: String) -> [Builtin] {
        var used: [Builtin] = []
        for (_, inner) in tokens(in: text) {
            let name = inner.trimmingCharacters(in: .whitespacesAndNewlines)
            if let builtin = Builtin(rawValue: name), !used.contains(builtin) {
                used.append(builtin)
            }
        }
        return used
    }

    /// Put values into a body whose includes have already been expanded.
    public static func render(_ text: String, values: [String: String]) -> String {
        replacing(text) { inner in
            let raw = inner.trimmingCharacters(in: .whitespacesAndNewlines)
            if raw.isEmpty || raw.hasPrefix(">") || raw.hasPrefix("!") { return nil }
            let (name, fallback) = split(raw)
            if let supplied = values[name], !supplied.isEmpty { return supplied }
            return fallback
        }
    }

    /// What the built-ins stand for at `now`, in the forms the desktop gives
    /// them: the date and the full timestamp in UTC, the time of day local.
    public static func builtinValues(
        _ names: [Builtin],
        clipboard: String,
        now: Date = Date(),
        zone: TimeZone = .current
    ) -> [String: String] {
        var out: [String: String] = [:]
        for name in names {
            switch name {
            case .clipboard:
                out["clipboard"] = clipboard
            case .date:
                out["date"] = String(stamp(now).prefix(10))
            case .time:
                let local = DateFormatter()
                local.locale = Locale(identifier: "en_US_POSIX")
                local.timeZone = zone
                local.dateFormat = "HH:mm"
                out["time"] = local.string(from: now)
            case .datetime:
                out["datetime"] = stamp(now)
            }
        }
        return out
    }

    /// `2026-11-24T08:15:30.123Z`, as JavaScript's `toISOString` writes it.
    private static func stamp(_ date: Date) -> String {
        let utc = DateFormatter()
        utc.locale = Locale(identifier: "en_US_POSIX")
        utc.timeZone = TimeZone(identifier: "UTC")
        utc.dateFormat = "yyyy-MM-dd'T'HH:mm:ss.SSS'Z'"
        return utc.string(from: date)
    }
}

/// A macro made ready to use: its includes spliced in, and what it still
/// needs to be told before it is text.
public struct Prepared: Hashable, Sendable {
    public var text: String
    public var variables: [Variable]
    public var builtins: [Builtin]
    public var missing: [String]
    public var cyclic: [String]

    public init(_ macro: Macro, in library: Library) {
        let expanded = Template.expandIncludes(macro.body, macros: library.macros)
        text = expanded.text
        variables = Template.variables(in: expanded.text)
        builtins = Template.builtins(in: expanded.text)
        missing = expanded.missing
        cyclic = expanded.cyclic
    }

    /// Whether it reads the clipboard, which on a phone means asking for it.
    public var wantsClipboard: Bool { builtins.contains(.clipboard) }

    /// The text, with `values` for its variables. A variable given nothing
    /// falls back to its default, as on the desktop.
    public func render(values: [String: String] = [:], clipboard: String = "", now: Date = Date()) -> String {
        var all = values
        // Built-ins last: a variable cannot be named after one, so nothing
        // typed in is lost, and nothing typed in can stand in for the clock.
        for (name, value) in Template.builtinValues(builtins, clipboard: clipboard, now: now) {
            all[name] = value
        }
        return Template.render(text, values: all)
    }
}
