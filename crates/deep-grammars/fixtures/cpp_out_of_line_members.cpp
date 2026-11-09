/* Out-of-line definitions of a constructor, a destructor and a method. */
#include "guard.hpp"

namespace app {

Guard::Guard(mutex &lock) : held(lock), depth(1) {
}

Guard::~Guard() {
}

int Guard::level() const {
    return depth;
}

}
