import Foundation

protocol Shape {
    var name: String { get }
    func area() -> Double
}

struct Point {
    let x: Double
    let y: Double

    func distance(to other: Point) -> Double {
        let dx = x - other.x
        let dy = y - other.y
        return (dx * dx + dy * dy).squareRoot()
    }
}

class Circle: Shape {
    private let radius: Double
    var name: String

    init(name: String, radius: Double) {
        self.name = name
        self.radius = radius
    }

    func area() -> Double {
        return Double.pi * radius * radius
    }
}

enum Direction {
    case north
    case south
    case east
    case west
}
