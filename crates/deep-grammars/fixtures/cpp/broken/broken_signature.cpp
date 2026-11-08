class Counter {
public:
    Counter() : value_(0) {}

    int 1bump() {
        return value_++;
    }

private:
    int value_;
};
