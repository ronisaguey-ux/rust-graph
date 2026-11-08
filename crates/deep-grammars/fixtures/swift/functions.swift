import Foundation

func greet(person name: String, greeting: String = "Hello") -> String {
    return "\(greeting), \(name)!"
}

func total(_ values: Int...) -> Int {
    var sum = 0
    for value in values {
        sum += value
    }
    return sum
}

func firstPositive(_ values: [Int]) throws -> Int {
    for value in values {
        if value > 0 {
            return value
        }
    }
    throw LookupError.empty
}

let numbers = [3, -1, 4]
let doubled = numbers.map { value in value * 2 }
let kept = numbers.filter { value in value > 0 }
let tried = try firstPositive(numbers)
print(greet(person: "ada"))
print(total(1, 2, 3))
print(doubled)
print(kept)
print(tried)
