import Foundation

func summarize(_ numbers: [Int]) -> String {
    guard !numbers.isEmpty else {
        return "none"
    }
    var total = 0
    var index = 0
    repeat {
        total += numbers[index]
        index += 1
    } while index < numbers.count

    switch total {
    case 0:
        return "zero"
    case 1...10:
        return "small: \(total)"
    case let value where value % 2 == 0:
        return "even: \(value)"
    default:
        return "other: \(total)"
    }
}

func firstString(in values: [Any]) -> String? {
    for value in values {
        if case let text as String = value {
            return text
        }
    }
    return nil
}

func classify(_ response: (Int, String)) {
    switch response {
    case (200, _):
        print("ok")
    case (let code, let message) where code >= 500:
        print("server \(code): \(message)")
    default:
        print("unexpected")
    }
}

func loop(with pairs: [(Int, Int)]) -> Int {
    var best = 0
    outer: for (left, right) in pairs {
        for multiplier in 1...3 {
            let product = left * right * multiplier
            if product > 100 {
                best = product
                break outer
            }
            if product < 0 {
                continue outer
            }
        }
    }
    return best
}

func optionalChain(_ feed: Feed?) -> Int {
    let first = feed?.items.first
    guard let item = first else { return 0 }
    let title = feed?.items.first?.identifier ?? "none"
    return title.count + (item.hashValue % 7)
}
