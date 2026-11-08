struct Stack<Element> {
    private var items: [Element] = []

    mutating func push(_ item: Element) {
        items.append(item)
    }

    mutating func pop() -> Element? {
        return items.popLast()
    }

    var isEmpty: Bool {
        return items.isEmpty
    }
}

extension Stack {
    func peek() -> Element? {
        return items.last
    }
}

func identity<T>(_ value: T) -> T {
    return value
}
