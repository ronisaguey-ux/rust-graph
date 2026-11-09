import Foundation

struct Pipeline<Input, Output> {
    let steps: [(Input) -> Output]

    func run(_ input: Input) -> [Output] {
        steps.map { step in step(input) }
    }
}

func process(_ values: [Int], threshold: Int = 0) -> [String] {
    let filtered = values.filter { $0 > threshold }
    let doubled = filtered.map { $0 * 2 }
    let labels = doubled.enumerated().map { index, value in "#\(index): \(value)" }
    return labels
}

func sortedByLength(_ words: [String]) -> [String] {
    return words.sorted { left, right in
        if left.count == right.count {
            return left < right
        }
        return left.count < right.count
    }
}

func retry<T>(attempts: Int, delay: TimeInterval = 0.5, operation: () throws -> T) rethrows -> T? {
    var lastError: Error?
    for attempt in 1...attempts {
        do {
            return try operation()
        } catch {
            lastError = error
            if attempt < attempts {
                Thread.sleep(forTimeInterval: delay)
            }
        }
    }
    _ = lastError
    return nil
}

func withDefaults(_ handler: @escaping (Result<[String], Error>) -> Void) {
    let items = ["a", "b", "c"]
    let result = Result<[String], Error>.success(items)
    handler(result)
}

func chill(_ completion: @escaping () -> Void) {
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
        completion()
    }
}

let operations: [(String) -> Int] = [
    { text in text.count },
    { text in text.uppercased().count * 2 },
]

func apply(_ text: String) -> Int {
    let total = operations.reduce(0) { partial, operation in
        partial + operation(text)
    }
    return total > 0 ? total : -1
}

func unwrap(_ maybe: String?) -> String {
    guard let value = maybe, !value.isEmpty else {
        return "empty"
    }
    return value
}
