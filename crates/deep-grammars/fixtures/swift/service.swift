import Foundation

protocol Repository {
    func save(_ item: String) throws
    func load(_ id: Int) -> String?
}

final class MemoryRepository: Repository {
    private var storage: [Int: String] = [:]
    private var nextId = 1

    func save(_ item: String) throws {
        storage[nextId] = item
        nextId += 1
    }

    func load(_ id: Int) -> String? {
        return storage[id]
    }
}

func run() {
    let repository = MemoryRepository()
    do {
        try repository.save("first")
        if let loaded = repository.load(1) {
            print(loaded)
        }
    } catch {
        print("failed")
    }
    let missing = repository.load(99) ?? "none"
    print(missing)
}
