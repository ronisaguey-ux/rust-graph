#include <memory>
#include <vector>

struct Node {
    int value;
    Node *next;
};

int allocations() {
    auto node = new Node(7);
    auto list = new std::vector<int>();
    list->push_back(node->value);
    int first = (*list)[0];
    int buffer[8];
    int *raw = new int[8];
    delete node;
    delete raw;
    delete list;
    return first + buffer[0];
}

double casts(int a, double b) {
    double widened = static_cast<double>(a);
    int narrowed = (int)b;
    unsigned long bits = reinterpret_cast<unsigned long>(&a);
    const int *locked = const_cast<const int *>(&a);
    return widened + narrowed + (double)bits + *locked;
}

int lambdas() {
    auto square = [](int v) {
        return v * v;
    };
    auto add = [](int a, int b) {
        return a + b;
    };
    int total = add(square(2), square(3));
    int captured = 0;
    auto bump = [captured](int step) {
        return captured + step;
    };
    return total + bump(1);
}
