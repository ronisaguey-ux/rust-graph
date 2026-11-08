#include <string>
#include <vector>
#include <iostream>

namespace app {

class Shape {
public:
    Shape(const std::string &name) : name_(name), sides_(0) {}

    ~Shape() {}

    int sides() const {
        return sides_;
    }

    std::string label() const {
        return name_;
    }

protected:
    std::string name_;
    int sides_;
};

class Square : public Shape {
public:
    Square(int side) : Shape("square"), side_(side) {}

    int area() const {
        return side_ * side_;
    }

private:
    int side_;
};

}  // namespace app

int main() {
    app::Square box = app::Square(4);
    std::cout << box.label() << " " << box.area() << std::endl;
    return 0;
}
