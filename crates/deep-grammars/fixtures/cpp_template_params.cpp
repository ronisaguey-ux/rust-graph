/* Template parameters that are not types, and a base class with arguments. */
namespace app {

typedef void (*cleanup)(int);

template<cleanup Function>
class Holder {
public:
    Holder() : value(0) {
    }
    int value;
};

template <class T>
struct Wrapper : public std::allocator<T> {
    Wrapper(const Wrapper &other) : std::allocator<T>(other) {
    }
};

}
