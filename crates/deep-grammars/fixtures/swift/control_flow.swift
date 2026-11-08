func classify(_ score: Int) -> String {
    if score >= 90 {
        return "A"
    } else if score >= 80 {
        return "B"
    } else {
        return "F"
    }
}

func requireName(_ name: String?) -> String {
    guard let value = name else {
        return "anonymous"
    }
    return value
}

var counter = 0
while counter < 3 {
    counter += 1
}

repeat {
    counter -= 1
} while counter > 0

for index in 0..<3 {
    if index == 1 {
        continue
    }
    print(index)
}

switch counter {
case 0:
    print("zero")
case 1...3:
    print("small")
default:
    print("large")
}

defer {
    print("done")
}
