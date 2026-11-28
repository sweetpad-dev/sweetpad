#include <metal_stdlib>
using namespace metal;

#if defined(VERTEX) || defined(MESH)
struct RasterVertex {
    float4 position [[position]];
    float value;
};
#endif

#if defined(OBJECT) || defined(MESH)
struct Payload { float value; };
#endif

#ifdef VERTEX
vertex RasterVertex origin(uint id [[vertex_id]]) {
    const float2 points[] = {float2(-1, -1), float2(3, -1), float2(-1, 3)};
    return {float4(points[id], 0, 1), 0.5f};
}
#endif

#ifdef FRAGMENT
struct RasterInput {
    float4 position [[position]];
    float value;
};
fragment float4 shade(RasterInput in [[stage_in]]) {
    return float4(in.value, 0, 0, 1);
}
#endif

#ifdef COMPUTE
kernel void increment(device uint *numbers [[buffer(0)]], uint i [[thread_position_in_grid]]) {
    numbers[i] += 3u;
}
#endif

#ifdef VISIBLE
[[visible]] uint twice(uint value) { return value * 2u; }
#endif

#ifdef STITCHABLE
[[stitchable]] uint addFive(uint value) { return value + 5u; }
#endif

#ifdef INTERSECTION
#include <metal_raytracing>
using namespace metal::raytracing;
[[intersection(triangle, triangle_data)]]
bool acceptTriangle(float2 barycentric [[barycentric_coord]]) { return barycentric.x < 0.75f; }
#endif

#ifdef OBJECT
[[object, max_total_threads_per_threadgroup(1)]]
void makePayload(object_data Payload &payload [[payload]], mesh_grid_properties grid) {
    payload.value = 0.75f;
    grid.set_threadgroups_per_grid(uint3(1));
}
#endif

#ifdef MESH
using Triangle = metal::mesh<RasterVertex, void, 3, 1, topology::triangle>;
[[mesh, max_total_threads_per_threadgroup(1)]]
void emitTriangle(Triangle output, const object_data Payload &payload [[payload]]) {
    const float2 points[] = {float2(-1, -1), float2(3, -1), float2(-1, 3)};
    for (uint i = 0; i < 3; ++i) {
        output.set_vertex(i, RasterVertex{float4(points[i], 0, 1), payload.value});
        output.set_index(i, i);
    }
    output.set_primitive_count(1);
}
#endif

#ifdef TILE
struct TileValue { half value [[raster_order_group(0)]]; };
kernel void fillTile(imageblock<TileValue, imageblock_layout_explicit> block,
                     ushort2 id [[thread_position_in_threadgroup]]) {
    block.data(id)->value = half(0.25f);
}
#endif
