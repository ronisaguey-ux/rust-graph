import Foundation

struct User: Codable, Identifiable, Hashable {
    let id: UUID
    let name: String
    let email: String?
    var roles: [String] = []
    var isActive: Bool { roles.contains("active") }
}

struct Post: Codable {
    let id: Int
    let title: String
    let body: String
    let author: User
    let tags: [String]
    let score: Double?

    var summary: String {
        let trimmed = body.prefix(80)
        return "\(title) by \(author.name): \(trimmed)…"
    }
}

enum FeedItem: Codable {
    case post(Post)
    case advertisement(URL, id: String)
    case separator(String)

    var identifier: String {
        switch self {
        case .post(let post):
            return "post-\(post.id)"
        case .advertisement(_, let id):
            return "ad-\(id)"
        case .separator(let title):
            return "sep-\(title.lowercased())"
        }
    }
}

struct Feed: Codable {
    var items: [FeedItem] = []
    var cursor: String?

    mutating func append(_ newItems: [FeedItem], cursor next: String?) {
        items.append(contentsOf: newItems)
        cursor = next
    }

    func filtered(by term: String) -> [FeedItem] {
        guard !term.isEmpty else { return items }
        return items.filter { item in
            switch item {
            case .post(let post):
                return post.title.localizedCaseInsensitiveContains(term)
            case .advertisement, .separator:
                return false
            }
        }
    }
}

extension Feed {
    static func decode(_ data: Data) throws -> Feed {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(Feed.self, from: data)
    }
}
