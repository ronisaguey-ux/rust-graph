import Foundation

func format(_ user: User, posts: [Post]) -> String {
    let name = user.name
    let count = posts.count
    let first = posts.first?.title ?? "none"
    return "\(name) has \(count) posts, first: \(first)"
}

func render(_ items: [FeedItem], limit: Int) -> String {
    var lines: [String] = []
    for item in items.prefix(limit) {
        let label: String
        switch item {
        case .post(let post):
            label = "\(post.title) (\(post.score ?? 0))"
        case .advertisement(let url, _):
            label = "sponsored: \(url.host ?? "unknown")"
        case .separator(let text):
            label = "-- \(text.uppercased()) --"
        }
        lines.append(label)
    }
    return lines.joined(separator: "\n")
}

func describe(_ value: Double) -> String {
    let formatted = String(format: "%.2f", value)
    return "value: \(formatted), raw: \(value), pinned: \(String(describing: value))"
}

let banner = """
    Welcome, \(User.self)
    Interpolation inside a multi-line literal: \(format(User(id: UUID(), name: "ada", email: nil), posts: []))
    """

let raw = #"literal \(notCode) and "quotes""#

let escaped = "tab:\t newline:\n quote:\" done"

let nested = "outer \(items.map { item in "\(item)" }.joined()) end"
