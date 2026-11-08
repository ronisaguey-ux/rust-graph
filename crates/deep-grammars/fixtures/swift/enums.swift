enum Result {
    case success(Int)
    case failure(String)
}

func describe(_ result: Result) -> String {
    switch result {
    case .success(let value):
        return "ok: \(value)"
    case .failure(let message):
        return "error: \(message)"
    }
}
