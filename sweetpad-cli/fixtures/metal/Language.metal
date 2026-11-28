#include <metal_stdlib>
using namespace metal;

#if __METAL_VERSION__ >= 400
template<bool Add>
static uint adjust(uint x) {
    if constexpr (Add) return x + 40u;
    else return x - 40u;
}
#endif

#if __METAL_VERSION__ >= 410 || defined(REQUIRE_PLACEMENT)
struct Number {
    uint value;
    explicit Number(uint x) : value(x ^ 65u) {}
};
#endif

kernel void language(device uint *words [[buffer(0)]], uint i [[thread_position_in_grid]]) {
    uint x = words[i];
#if __METAL_VERSION__ >= 410 || defined(REQUIRE_PLACEMENT)
    Number object(1u);
    object.~Number();
    new (&object) Number(x);
    words[i] = object.value;
#elif __METAL_VERSION__ >= 400
    words[i] = adjust<true>(x);
#else
    words[i] = x + __METAL_VERSION__;
#endif
}
