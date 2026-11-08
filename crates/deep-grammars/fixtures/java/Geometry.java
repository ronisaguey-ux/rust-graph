package com.example.geometry;

public sealed interface Shape permits Circle, Square, Rect {
    double area();

    default boolean largerThan(Shape other) {
        return area() > other.area();
    }

    record Circle(double radius) implements Shape {
        @Override
        public double area() {
            return Math.PI * radius * radius;
        }
    }

    record Square(double side) implements Shape {
        @Override
        public double area() {
            return side * side;
        }
    }

    final class Rect implements Shape {
        private final double width;
        private final double height;

        Rect(double width, double height) {
            this.width = width;
            this.height = height;
        }

        public double area() {
            return width * height;
        }
    }

    static String describe(Shape shape) {
        return switch (shape) {
            case Circle c when c.radius() > 10 -> "large circle";
            case Circle c -> "circle " + c.radius();
            case Square(double side) -> "square " + side;
            case Rect r -> "rect " + r.area();
        };
    }
}
