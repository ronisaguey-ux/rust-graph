import Foundation

protocol HTTPClient: AnyObject {
    func get(_ path: String, headers: [String: String]) async throws -> (Data, HTTPURLResponse)
}

enum NetworkError: Error {
    case invalidURL(String)
    case status(Int)
    case decoding(String, underlying: Error?)
}

struct Endpoint<Response> {
    let path: String
    let method: String
    let query: [String: String]
    let decode: (Data) throws -> Response

    init(path: String, method: String = "GET", query: [String: String] = [:], decode: @escaping (Data) throws -> Response) {
        self.path = path
        self.method = method
        self.query = query
        self.decode = decode
    }
}

final class APIClient: HTTPClient {
    private let session: URLSession
    private let base: URL
    private var cache: [String: Data] = [:]

    init(base: URL, session: URLSession = .shared) {
        self.base = base
        self.session = session
    }

    func get(_ path: String, headers: [String: String] = [:]) async throws -> (Data, HTTPURLResponse) {
        guard let url = URL(string: path, relativeTo: base) else {
            throw NetworkError.invalidURL(path)
        }
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        for (key, value) in headers {
            request.setValue(value, forHTTPHeaderField: key)
        }
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw NetworkError.status(-1)
        }
        guard (200..<300).contains(http.statusCode) else {
            throw NetworkError.status(http.statusCode)
        }
        cache[path] = data
        return (data, http)
    }

    func fetch<Response>(_ endpoint: Endpoint<Response>) async throws -> Response {
        let (data, _) = try await get(endpoint.path, headers: ["Accept": "application/json"])
        do {
            return try endpoint.decode(data)
        } catch {
            throw NetworkError.decoding(endpoint.path, underlying: error)
        }
    }
}

extension APIClient {
    func fetchAll<Item: Decodable>(_ paths: [String]) async throws -> [Item] {
        var results: [Item] = []
        for path in paths {
            let endpoint = Endpoint<Item>(path: path) { data in
                try JSONDecoder().decode(Item.self, from: data)
            }
            results.append(try await fetch(endpoint))
        }
        return results
    }
}
