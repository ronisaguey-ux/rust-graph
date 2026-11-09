import Foundation

protocol Repository {
    associatedtype Model
    func all() throws -> [Model]
    func save(_ model: Model) throws
}

struct MemoryStore<Model: Identifiable>: Repository where Model.ID: Hashable {
    private var storage: [Model.ID: Model] = [:]

    func all() throws -> [Model] {
        Array(storage.values)
    }

    mutating func save(_ model: Model) throws {
        storage[model.id] = model
    }
}

protocol Identifiable {
    associatedtype ID: Hashable
    var id: ID { get }
}

struct Book: Identifiable, Equatable {
    let id: String
    let title: String
    let pages: Int
}

struct Paged<Element> {
    let items: [Element]
    let page: Int
    let hasMore: Bool

    func map<Result>(_ transform: (Element) -> Result) -> Paged<Result> {
        Paged<Result>(items: items.map(transform), page: page, hasMore: hasMore)
    }
}

func largest<T: Comparable>(_ values: [T]) -> T? {
    guard var best = values.first else { return nil }
    for value in values.dropFirst() where value > best {
        best = value
    }
    return best
}

func pairUp<A, B>(_ left: [A], _ right: [B]) -> [(A, B)] {
    var pairs: [(A, B)] = []
    for index in 0..<min(left.count, right.count) {
        pairs.append((left[index], right[index]))
    }
    return pairs
}

func dictionary<K: Hashable, V>(from entries: [(K, V)]) -> [K: V] {
    var result: [K: V] = [:]
    for (key, value) in entries {
        result[key] = value
    }
    return result
}
