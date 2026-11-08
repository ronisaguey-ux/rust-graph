#include <map>
#include <utility>
#include <vector>

namespace store {

template <typename Key, typename Value>
class Table {
public:
    Table() : size_(0) {}

    void insert(const Key &key, const Value &value) {
        keys_.push_back(key);
        values_.push_back(value);
        size_ += 1;
    }

    Value get(int index) const {
        return values_[index];
    }

private:
    std::vector<Key> keys_;
    std::vector<Value> values_;
    int size_;
};

}  // namespace store

int nested_collections() {
    std::vector<std::vector<std::pair<int, int>>> matrix;
    std::vector<std::pair<int, int>> row;
    row.push_back(std::pair<int, int>(1, 2));
    matrix.push_back(row);
    std::map<int, std::vector<int>> buckets;
    buckets[0] = std::vector<int>();
    return matrix.size() + buckets.size();
}

int use_table() {
    store::Table<int, int> table;
    table.insert(1, 10);
    table.insert(2, 20);
    return table.get(0) + table.get(1);
}
