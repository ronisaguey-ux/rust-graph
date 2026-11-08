#include <string>

namespace geometry {

class Shape {
public:
    ~Shape() {}
    double area() const {
        return 0.0;
    }
    const char *name() const {
        return "shape";
    }
};

class Circle : public Shape {
public:
    Circle(double radius) : radius_(radius) {}

    double area() const override {
        return 3.14159 * radius_ * radius_;
    }

    const char *name() const override {
        return "circle";
    }

private:
    double radius_;
};

class Rect : public Shape {
public:
    Rect(double width, double height) : width_(width), height_(height) {}

    double area() const override {
        return width_ * height_;
    }

    const char *name() const override {
        return "rect";
    }

private:
    double width_;
    double height_;
};

double total(const Shape &first, const Shape &second) {
    return first.area() + second.area();
}

}  // namespace geometry
