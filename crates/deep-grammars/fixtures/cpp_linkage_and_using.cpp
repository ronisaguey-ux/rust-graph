/* A C++ header: a linkage block, a scoped using-declaration and a
   function-pointer typedef that takes a qualified pointer parameter. */
#include <memory>

#ifdef __cplusplus
extern "C" {
#endif

int c_api_entry(int argc);

#ifdef __cplusplus
}
#endif

namespace app {

using helpers::LockGuard;
using std::mutex;

typedef void (*destructor)(mutex *const);

class Guard {
public:
    explicit Guard(mutex &lock);
    ~Guard();
};

}
