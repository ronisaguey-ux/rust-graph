import Foundation

actor Counter {
    private var value: Int = 0

    func increment() -> Int {
        value += 1
        return value
    }

    func reset() {
        value = 0
    }
}

struct Download {
    let url: URL
    let bytes: Int
}

func downloadAll(_ urls: [URL]) async throws -> [Download] {
    var downloads: [Download] = []
    try await withThrowingTaskGroup(of: Download.self) { group in
        for url in urls {
            group.addTask {
                let (data, _) = try await URLSession.shared.data(from: url)
                return Download(url: url, bytes: data.count)
            }
        }
        for try await download in group {
            downloads.append(download)
        }
    }
    return downloads.sorted { left, right in left.bytes > right.bytes }
}

func measure<T>(_ label: String, operation: () async throws -> T) async rethrows -> T {
    let started = Date()
    defer {
        let elapsed = Date().timeIntervalSince(started)
        print("\(label) took \(String(format: "%.3f", elapsed))s")
    }
    return try await operation()
}

func firstReady(_ urls: [URL]) async -> Download? {
    await withTaskGroup(of: Download?.self) { group in
        for url in urls {
            group.addTask {
                guard let (data, response) = try? await URLSession.shared.data(from: url) else {
                    return nil
                }
                let code = (response as? HTTPURLResponse)?.statusCode ?? 0
                guard code == 200 else { return nil }
                return Download(url: url, bytes: data.count)
            }
        }
        for await result in group {
            if let download = result {
                group.cancelAll()
                return download
            }
        }
        return nil
    }
}

@MainActor
final class DownloadsViewModel {
    private(set) var downloads: [Download] = []
    private(set) var error: String?

    func reload(_ urls: [URL]) async {
        error = nil
        do {
            downloads = try await downloadAll(urls)
        } catch {
            self.error = "failed: \(error.localizedDescription)"
        }
    }
}
