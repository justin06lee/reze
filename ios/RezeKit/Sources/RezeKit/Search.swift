import Foundation

// Finding a macro: by typing some of its name (the palette's fuzzy match,
// `src/lib/fuzzy.ts`) and by having just typed all of it (expand in place,
// `src-tauri/src/expand.rs`). Both are ports, so the phone ranks and matches
// the way the desktop does.

public struct Match: Hashable, Sendable {
    public var score: Double
    /// Which characters of the target matched, for highlighting.
    public var indices: [Int]
}

public enum Fuzzy {
    private static let separators: Set<Character> = [" ", "-", "_", "/", ".", ":"]

    /// Subsequence match with bonuses for where the characters fall, tuned for
    /// short trigger names: "fa" lands on "full analysis" ahead of "refactor
    /// plan".
    public static func match(_ query: String, _ target: String) -> Match? {
        if query.isEmpty { return Match(score: 0, indices: []) }

        let q = Array(query.lowercased())
        let t = Array(target.lowercased())
        var indices: [Int] = []
        var score = 0.0
        var ti = 0
        var last = -2

        for ch in q {
            // A space in the query separates, it is not a character to find:
            // "fu an" matches "full analysis" without typing the exact gap.
            if ch == " " { continue }
            guard ti <= t.count, let found = t[ti...].firstIndex(of: ch) else { return nil }

            if found == 0 {
                score += 20
            } else if separators.contains(t[found - 1]) {
                score += 14
            }
            if found == last + 1 { score += 10 }
            // A mild penalty for gaps, so tight matches float up.
            score -= Double(min(found - ti, 6))

            indices.append(found)
            last = found
            ti = found + 1
        }

        // A clean prefix or an exact hit, above anything the bonuses give.
        let compact = String(q.filter { !$0.isWhitespace })
        let whole = String(t)
        if whole == compact {
            score += 60
        } else if whole.hasPrefix(compact) {
            score += 30
        } else if whole.contains(compact) {
            score += 15
        }

        // Shorter targets are a better fit for the same match.
        score -= Double(t.count) * 0.1
        return Match(score: score, indices: indices)
    }

    /// The macros `query` finds, best first. With nothing typed, all of them,
    /// the most used first — the order the palette opens in.
    public static func search(_ query: String, in macros: [Macro]) -> [Macro] {
        let query = query.trimmingCharacters(in: .whitespaces)
        if query.isEmpty {
            return macros.sorted {
                $0.usageCount != $1.usageCount
                    ? $0.usageCount > $1.usageCount
                    : $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
            }
        }
        return macros
            .compactMap { macro -> (Macro, Double)? in
                if let name = match(query, macro.name) { return (macro, name.score + 25) }
                // Failing the name, its tags and description: a macro can be
                // found by what it is for, not only by what it was called.
                let haystack = (macro.tags + [macro.description]).joined(separator: " ")
                return match(query, haystack).map { (macro, $0.score - 20) }
            }
            .sorted { $0.1 != $1.1 ? $0.1 > $1.1 : $0.0.usageCount > $1.0.usageCount }
            .map(\.0)
    }
}

/// A macro whose name was just typed, and how much of the text is that name.
public struct Trigger: Hashable, Sendable {
    public var macro: Macro
    /// How many characters before the caret to take away: the name, and any
    /// spaces typed after it.
    public var delete: Int
    /// Those spaces, to put back after the expansion.
    public var trailing: String
}

public enum Triggers {
    /// Never look further back than this many words, however long a macro's
    /// name is.
    static let wordCeiling = 8

    /// The longest macro name that ends `text`, whole words only.
    ///
    /// Longest wins: with both `analysis` and `full analysis` defined, having
    /// typed "full analysis" means the second.
    public static func match(_ text: String, in macros: [Macro]) -> Trigger? {
        let chars = Array(text)
        var words: [(start: Int, word: String)] = []
        var start: Int?
        for (i, c) in chars.enumerated() {
            if c.isWhitespace {
                if let s = start {
                    words.append((s, String(chars[s..<i])))
                    start = nil
                }
            } else if start == nil {
                start = i
            }
        }
        if let s = start { words.append((s, String(chars[s...]))) }
        guard let lastWord = words.last else { return nil }
        let end = lastWord.start + lastWord.word.count

        for span in stride(from: min(words.count, wordCeiling), through: 1, by: -1) {
            let first = words.count - span
            let candidate = words[first...].map(\.word).joined(separator: " ").lowercased()
            for macro in macros {
                let name = macro.name
                    .split(whereSeparator: \.isWhitespace)
                    .joined(separator: " ")
                    .lowercased()
                if !name.isEmpty, name == candidate {
                    return Trigger(
                        macro: macro,
                        delete: chars.count - words[first].start,
                        trailing: String(chars[end...])
                    )
                }
            }
        }
        return nil
    }
}
