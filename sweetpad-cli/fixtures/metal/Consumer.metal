#include <metal_stdlib>
#include <metal_raytracing>
using namespace metal;
using namespace metal::raytracing;

struct KhronosVertex {
    float4 position [[position]];
    float4 accum [[user(locn0)]];
};
vertex KhronosVertex khronosVertices(uint i [[vertex_id]]) {
    const float2 p[] = {float2(-1, -1), float2(3, -1), float2(-1, 3)};
    return {float4(p[i], 0, 1), float4(0.25, 0.5, 0.75, 1)};
}
struct CopiedVertex { float4 color [[user(locn1)]]; };
fragment float4 copiedColor(CopiedVertex in [[stage_in]]) { return in.color; }

// Callbacks are supplied by the library under test. Only these consumers use Xcode.
using Transform = uint(uint);
kernel void apply(visible_function_table<Transform> table [[buffer(0)]],
                  device uint *values [[buffer(1)]], uint i [[thread_position_in_grid]]) {
    values[i] = table[0](values[i]);
}

kernel void trace(primitive_acceleration_structure scene [[buffer(0)]],
                  intersection_function_table<triangle_data> callbacks [[buffer(1)]],
                  device uint *hits [[buffer(2)]], uint i [[thread_position_in_grid]]) {
    ray r;
    r.origin = float3(i ? 0.8f : 0.2f, 0.1f, 1.0f);
    r.direction = float3(0, 0, -1);
    r.min_distance = 0;
    r.max_distance = 2;
    intersector<triangle_data> query;
    auto hit = query.intersect(r, scene, callbacks);
    hits[i] = hit.type == intersection_type::triangle;
}

struct TileValue { half value [[raster_order_group(0)]]; };
kernel void readTile(imageblock<TileValue, imageblock_layout_explicit> tile,
                     device float *values [[buffer(0)]], ushort2 p [[thread_position_in_threadgroup]]) {
    values[p.y * 16 + p.x] = tile.data(p)->value;
}
