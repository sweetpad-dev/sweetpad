# Experimental Linux Metal compiler

SweetPad can compile Metal source into AIR and link a Metal library on Linux. The CLI embeds the Python frontend and linker. Stock Clang and LLVM 14 compile the shader bodies and assemble typed AIR bitcode. An Apple SDK and GPU are used by the reference tests. The Linux shader compiler uses its own Metal declarations.

Install Python 3, `clang-14`, and `llvm-14` on the Linux build host. Enable the compiler for a SweetPad build with `SWEETPAD_NATIVE_METAL=1`. The switch selects a separate Swift Build tools directory so an incremental build picks up the change.

```sh
sudo apt-get install python3 clang-14 llvm-14
SWEETPAD_NATIVE_METAL=1 sweetpad build --project Example.xcodeproj --scheme Example
```

The packaged `sweetpad` executable also accepts the tool names `metal` and `metallib` through symlinks. To compile a standalone Metal source in MSL 4.1, use the following commands on Linux.

```sh
mkdir -p tools
ln -s "$(command -v sweetpad)" tools/metal
ln -s "$(command -v sweetpad)" tools/metallib
tools/metal -c -target air64-apple-macosx27.0 -std=metal4.1 Shader.metal -o Shader.air
tools/metallib Shader.air -o Shader.metallib
```

## What the GPU tests cover

The probes use the same MSL source for both compilers. A macOS process loads the resulting libraries on one GPU. It tests each candidate stage with Xcode-built companions and checks the result against both Xcode and an independent expected value.

| Kind | Observable result |
| --- | --- |
| Vertex | Vertex IDs generate a triangle covering all 256 pixels of a render target. |
| Fragment | An interpolated input controls the red channel of all 256 pixels. |
| Compute | Four distinct buffer elements increase by three. |
| Visible | A visible-function table applies the candidate function to four inputs. |
| Stitchable | A Metal stitching graph composes the candidate twice, then a compute shader calls it through a function table. |
| Intersection | A triangle intersection callback accepts one ray and rejects a second ray that hits the same triangle. |
| Object | An object shader launches a mesh threadgroup and supplies the value observed in the framebuffer. |
| Mesh | A mesh shader emits vertex positions, a varying, indices and a primitive count. |
| Tile | A second tile shader reads back all 256 imageblock values produced by the candidate. |

The suite selects MSL 2.4, 3.0, 3.1, 3.2, 4.0 and 4.1. Object and mesh functions require MSL 3.0. A separate language probe checks the version macro, a C++17 `if constexpr` specialization in 4.0, and placement construction of a thread-local object in 4.1. The implemented placement form discards the returned pointer. [Metal Shading Language Specification](https://developer.apple.com/metal/Metal-Shading-Language-Specification.pdf)

## Khronos coverage

The corpus contains 451 vertex, fragment and compute reference files from the pinned SPIRV-Cross revision, with 443 distinct source texts. The inventory checksum covers the source hashes and their paths. For each mode the suite checks which files Xcode accepts, which files the Linux compiler accepts, exported names and types, the AIR and MSL version fields, and function and vertex-attribute reflection after Metal loads each jointly built library. Three unmodified Khronos programs also run on the GPU in every mode. They cover a two-attribute vertex interface, a fragment loop, and vector shifts in compute. The six advanced function kinds use the dedicated probes above. [Khronos source corpus](https://github.com/KhronosGroup/SPIRV-Cross/tree/78b75e2d99ccf038fd525ba1effad3ebe7b6aeb4/reference/shaders-msl)

`coverage.json` records the individual sources already verified and Xcode's rejection set per mode. The Linux job fails if a previously supported source stops building. The hardware comparison fails for changed public interfaces, unexpected acceptance, mismatched behavior, and failed stage probes. It lists every Xcode-accepted source the candidate still rejects. Pass `--require-full-corpus` to make those omissions fail the comparison too.

Measured on an Apple M3 Pro with Xcode 27.2 build 27B5019j and a native Ubuntu 24.04 build. The denominator for each column is 443 distinct Khronos sources.

| MSL | Xcode compiled and linked | Linux compiled, linked and matched public GPU reflection |
| --- | ---: | ---: |
| 2.4 | 432 | 53 |
| 3.0 | 433 | 53 |
| 3.1 | 440 | 53 |
| 3.2 | 443 | 53 |
| 4.0 | 443 | 53 |
| 4.1 | 442 | 53 |

## Reproduce the comparison

Use a Linux host and a Mac with Xcode 27.2, the Metal toolchain component and a GPU supporting ray tracing, mesh shaders, stitching and tile shaders. Run the commands from the repository root. Both hosts need the pinned source archive. Transfer the Linux output directory to the Mac, preserving its layout.

```sh
rev=78b75e2d99ccf038fd525ba1effad3ebe7b6aeb4
curl -fL "https://codeload.github.com/KhronosGroup/SPIRV-Cross/tar.gz/$rev" -o corpus.tar.gz
```

On Linux, the default command runs the compiler source directly. Pass `--tools /absolute/path/to/tools` to test the packaged multicall binary instead.

```sh
python3 sweetpad-cli/fixtures/metal/test_driver.py
python3 sweetpad-cli/fixtures/metal/verify.py build --compiler linux --corpus corpus.tar.gz --output linux
```

On the Mac, select the intended Xcode with `DEVELOPER_DIR` or `xcode-select`.

```sh
python3 sweetpad-cli/fixtures/metal/verify.py build --compiler xcode --corpus corpus.tar.gz --output xcode
python3 sweetpad-cli/fixtures/metal/verify.py compare --linux linux --xcode xcode --output report
```

The report includes per-version counts, GPU observations, public reflection and paths for remaining gaps. GitHub Actions runs the Linux regressions on changes to the compiler. The workflow's manual hardware job needs a self-hosted Apple Silicon runner labeled `metal-gpu`.

For the CoMaps debug-rectangle regression, compile its original `libs/shaders/Metal/debug_rect.metal` on each host with `-target air64-apple-macosx27.0 -std=macos-metal2.4` and link each AIR object. Run `report/Runtime comaps-rect FILE.metallib . 2.4 -` for each library. The expected RGBA is `[191, 64, 0, 255]` across all 256 pixels. At CoMaps revision `27ea5a9`, this is the first of 12 shader translation units that builds on Linux. Six more encounter a `float4x4` uniform first.

## Current boundaries

The frontend covers scalar and vector arithmetic, ordinary control flow, selected buffer layouts, vertex attributes and varyings, triangle meshes and intersections, scalar stitching and explicit tile imageblocks. Textures and samplers, matrix and packed-vector operations, general argument buffers, cross-file shader helper resolution, source-level shader debugging and Metal archives remain open. MSL 4.1 integer interleave is also open. Xcode accepted the source and the M3 Pro refused its pipeline. Device, simulator and macOS targets have distinct library identities. The nine-stage GPU comparison targets macOS 27.
