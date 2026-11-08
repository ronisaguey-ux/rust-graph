#include <vector>
#include <map>
#include <string>

template <typename T>
T max_of(T a, T b) {
    return a > b ? a : b;
}

template <typename T>
struct Box {
    T value;
};

int nested_vectors() {
    std::vector<std::vector<int>> grid;
    std::vector<int> row;
    row.push_back(1);
    grid.push_back(row);
    return grid.size();
}

int sum_pairs() {
    std::map<std::string, int> counts;
    counts["a"] = 1;
    counts["b"] = 2;
    int total = 0;
    std::map<std::string, int>::iterator it = counts.begin();
    for (; it != counts.end(); ++it) {
        total += it->second;
    }
    return total;
}

int tables() {
    int table[4] = {1, 2, 3, 4};
    int value = max_of(3, 7);
    value += max_of<int>(table[0], table[1]);
    return value;
}
