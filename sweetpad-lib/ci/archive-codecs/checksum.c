/* AEA uses MurmurHash64A with this fixed seed. Byte loads avoid alignment UB. */
#include <stddef.h>
#include <stdint.h>

uint64_t sweetpad_murmur64(const uint8_t *data, size_t size) {
    const uint64_t multiplier = UINT64_C(0xc6a4a7935bd1e995);
    uint64_t hash = UINT64_C(0xe2236fdc26a5f6d2) ^ ((uint64_t)size * multiplier);
    size_t index = 0;
    for (; size - index >= 8; index += 8) {
        uint64_t word = 0;
        for (unsigned byte = 0; byte < 8; byte++)
            word |= (uint64_t)data[index + byte] << (8 * byte);
        word *= multiplier; word ^= word >> 47; word *= multiplier;
        hash ^= word; hash *= multiplier;
    }
    uint64_t tail = 0;
    for (unsigned byte = 0; index + byte < size; byte++)
        tail |= (uint64_t)data[index + byte] << (8 * byte);
    if (index < size) { hash ^= tail; hash *= multiplier; }
    hash ^= hash >> 47; hash *= multiplier; hash ^= hash >> 47;
    return hash;
}
