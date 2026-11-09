import Foundation

enum ValidationError: LocalizedError {
    case missing(String)
    case tooShort(field: String, minimum: Int)
    case invalid(String, reason: String?)

    var errorDescription: String? {
        switch self {
        case .missing(let field):
            return "missing \(field)"
        case .tooShort(let field, let minimum):
            return "\(field) needs at least \(minimum) characters"
        case .invalid(let value, let reason):
            return "invalid \(value): \(reason ?? "unknown")"
        }
    }
}

func validate(name: String, age: Int) throws -> User {
    guard !name.isEmpty else {
        throw ValidationError.missing("name")
    }
    guard name.count >= 2 else {
        throw ValidationError.tooShort(field: "name", minimum: 2)
    }
    guard age >= 0 && age < 130 else {
        let reason: String? = age < 0 ? "negative" : "too large"
        throw ValidationError.invalid("age", reason: reason)
    }
    return User(id: UUID(), name: name, email: nil)
}

func load(_ path: String) -> Result<Data, Error> {
    do {
        let data = try Data(contentsOf: URL(fileURLWithPath: path))
        return .success(data)
    } catch let error as CocoaError {
        return .failure(error)
    } catch {
        return .failure(ValidationError.missing(path))
    }
}

func attempt(_ path: String) -> Data? {
    defer { print("done with \(path)") }
    let result = load(path)
    switch result {
    case .success(let data):
        return data
    case .failure(let error):
        print("error: \(error)")
        return nil
    }
}
