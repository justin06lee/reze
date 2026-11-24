import Foundation
import Testing

@testable import RezeKit

// The Swift ports held to the desktop's own code: `Fixtures/parity.json` is
// what `src/lib/template.ts` and `src/lib/fuzzy.ts` made of these inputs
// (`Tests/fixtures.ts` writes it), and the ports must make the same of them.

private struct Parity: Decodable {
    struct Expansion: Decodable {
        var text: String
        var missing: [String]
        var cyclic: [String]
    }

    struct Var: Decodable {
        var name: String
        var `default`: String
    }

    struct TemplateCase: Decodable {
        var body: String
        var expanded: Expansion
        var variables: [Var]
        var builtins: [String]
        var rendered: [String]
    }

    struct Hit: Decodable {
        var score: Double
        var indices: [Int]
    }

    struct FuzzyCase: Decodable {
        var query: String
        var target: String
        var match: Hit?
    }

    var macros: [Macro]
    var values: [[String: String]]
    var template: [TemplateCase]
    var fuzzy: [FuzzyCase]

    static func load() throws -> Parity {
        let url = try #require(Bundle.module.url(forResource: "parity", withExtension: "json", subdirectory: "Fixtures"))
        return try JSONDecoder().decode(Parity.self, from: Data(contentsOf: url))
    }
}

@Test func templatesExpandAsTheDesktopExpandsThem() throws {
    let parity = try Parity.load()
    #expect(parity.template.count >= 10)

    for expected in parity.template {
        let expanded = Template.expandIncludes(expected.body, macros: parity.macros)
        #expect(expanded.text == expected.expanded.text, "includes of \(expected.body)")
        #expect(expanded.missing == expected.expanded.missing, "missing in \(expected.body)")
        #expect(expanded.cyclic == expected.expanded.cyclic, "cycles in \(expected.body)")

        let variables = Template.variables(in: expanded.text)
        #expect(variables.map(\.name) == expected.variables.map(\.name), "variables of \(expected.body)")
        #expect(variables.map(\.fallback) == expected.variables.map(\.default), "defaults of \(expected.body)")
        #expect(Template.builtins(in: expanded.text).map(\.rawValue) == expected.builtins, "built-ins of \(expected.body)")

        for (values, rendered) in zip(parity.values, expected.rendered) {
            #expect(Template.render(expanded.text, values: values) == rendered, "\(expected.body) with \(values)")
        }
    }
}

@Test func fuzzyMatchesAsTheDesktopMatches() throws {
    let parity = try Parity.load()
    #expect(parity.fuzzy.count >= 100)

    for expected in parity.fuzzy {
        let match = Fuzzy.match(expected.query, expected.target)
        let label = Comment(rawValue: "\"\(expected.query)\" in \"\(expected.target)\"")
        #expect((match == nil) == (expected.match == nil), label)
        if let match, let want = expected.match {
            #expect(abs(match.score - want.score) < 0.0001, label)
            #expect(match.indices == want.indices, label)
        }
    }
}
