#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <TargetConditionals.h>

// Each invocation tests one stage. Its companion stages and observers come from Xcode.
static id<MTLDevice> gpu;
static id<MTLCommandQueue> queue;
static id<MTLLibrary> candidate, consumer;
static NSString *controlDir, *version, *kind;

static void require(BOOL ok, NSString *message) {
    if (!ok) @throw [NSException exceptionWithName:@"MetalProbe" reason:message userInfo:nil];
}
static id<MTLLibrary> load(NSString *path) {
    NSError *err = nil;
    id<MTLLibrary> lib = [gpu newLibraryWithURL:[NSURL fileURLWithPath:path] error:&err];
    require(lib != nil, [NSString stringWithFormat:@"load %@: %@", path, err]);
    return lib;
}
static id<MTLFunction> function(id<MTLLibrary> lib, NSString *name) {
    id<MTLFunction> fn = [lib newFunctionWithName:name];
    require(fn != nil, [@"missing function " stringByAppendingString:name]);
    return fn;
}
static id<MTLFunction> stage(NSString *which, NSString *name) {
    if ([kind isEqual:which]) return function(candidate, name);
    NSString *file = [NSString stringWithFormat:@"%@-%@.metallib", version, which];
    return function(load([controlDir stringByAppendingPathComponent:file]), name);
}
static id<MTLComputePipelineState> computePipeline(id<MTLFunction> main, id<MTLFunction> callback) {
    MTLComputePipelineDescriptor *desc = [MTLComputePipelineDescriptor new];
    desc.computeFunction = main;
    if (callback) {
        MTLLinkedFunctions *linked = [MTLLinkedFunctions new];
        linked.functions = @[callback];
        desc.linkedFunctions = linked;
    }
    NSError *error = nil;
    id<MTLComputePipelineState> pipeline = [gpu newComputePipelineStateWithDescriptor:desc
        options:MTLPipelineOptionNone reflection:nil error:&error];
    require(pipeline != nil, error.description);
    return pipeline;
}
static void finish(id<MTLCommandBuffer> cmd) {
    [cmd commit];
    [cmd waitUntilCompleted];
    require(cmd.status == MTLCommandBufferStatusCompleted, cmd.error.description);
}
static id<MTLBuffer> words(const uint32_t *data, NSUInteger count) {
    return [gpu newBufferWithBytes:data length:count * sizeof(uint32_t) options:MTLResourceStorageModeShared];
}
static NSArray *readWords(id<MTLBuffer> buf, NSUInteger count) {
    NSMutableArray *values = [NSMutableArray new];
    for (NSUInteger i = 0; i < count; i++) [values addObject:@(((uint32_t *)buf.contents)[i])];
    return values;
}
static id<MTLFunction> stitched(void) {
    id<MTLFunction> add = function(candidate, @"addFive");
    MTLFunctionStitchingInputNode *input = [[MTLFunctionStitchingInputNode alloc] initWithArgumentIndex:0];
    MTLFunctionStitchingFunctionNode *one = [[MTLFunctionStitchingFunctionNode alloc]
        initWithName:add.name arguments:@[input] controlDependencies:@[]];
    MTLFunctionStitchingFunctionNode *two = [[MTLFunctionStitchingFunctionNode alloc]
        initWithName:add.name arguments:@[one] controlDependencies:@[]];
    MTLFunctionStitchingGraph *graph = [[MTLFunctionStitchingGraph alloc]
        initWithFunctionName:@"twiceComposed" nodes:@[one, two] outputNode:two attributes:@[]];
    MTLStitchedLibraryDescriptor *desc = [MTLStitchedLibraryDescriptor new];
    desc.functions = @[add];
    desc.functionGraphs = @[graph];
    NSError *error = nil;
    id<MTLLibrary> lib = [gpu newLibraryWithStitchedDescriptor:desc error:&error];
    require(lib != nil, error.description);
    return function(lib, @"twiceComposed");
}
static NSArray *arithmetic(void) {
    uint32_t input[] = {0, 1, 7, 19};
    id<MTLBuffer> data = words(input, 4);
    BOOL language = [kind isEqual:@"language"];
    BOOL direct = language || [kind isEqual:@"compute"];
    id<MTLFunction> callback = direct ? nil :
        ([kind isEqual:@"visible"] ? function(candidate, @"twice") : stitched());
    id<MTLComputePipelineState> pipe = computePipeline(
        direct ? function(candidate, language ? @"language" : @"increment") : function(consumer, @"apply"), callback);
    id<MTLCommandBuffer> cmd = [queue commandBuffer];
    id<MTLComputeCommandEncoder> enc = [cmd computeCommandEncoder];
    [enc setComputePipelineState:pipe];
    if (callback) {
        MTLVisibleFunctionTableDescriptor *desc = [MTLVisibleFunctionTableDescriptor new];
        desc.functionCount = 1;
        id<MTLVisibleFunctionTable> table = [pipe newVisibleFunctionTableWithDescriptor:desc];
        [table setFunction:[pipe functionHandleWithFunction:callback] atIndex:0];
        [enc setVisibleFunctionTable:table atBufferIndex:0];
    }
    [enc setBuffer:data offset:0 atIndex:direct ? 0 : 1];
    [enc dispatchThreads:MTLSizeMake(4, 1, 1) threadsPerThreadgroup:MTLSizeMake(4, 1, 1)];
    [enc endEncoding];
    finish(cmd);
    return readWords(data, 4);
}
static NSArray *khronosCompute(void) {
    uint32_t input[] = {3, 8, 15, 24, 0, 1, 2, 3};
    uint32_t initial[] = {99,99,99,99,99,99,99,99};
    id<MTLBuffer> in = words(input, 8), out = words(initial, 8);
    id<MTLComputePipelineState> pipe = computePipeline(function(candidate, @"main0"), nil);
    id<MTLCommandBuffer> cmd = [queue commandBuffer];
    id<MTLComputeCommandEncoder> enc = [cmd computeCommandEncoder];
    [enc setComputePipelineState:pipe];
    [enc setBuffer:in offset:0 atIndex:0];
    [enc setBuffer:out offset:0 atIndex:1];
    [enc dispatchThreads:MTLSizeMake(1, 1, 1) threadsPerThreadgroup:MTLSizeMake(1, 1, 1)];
    [enc endEncoding];
    finish(cmd);
    return readWords(out, 8);
}
static NSArray *traceRays(void) {
    const float triangle[] = {0,0,0, 1,0,0, 0,1,0};
    id<MTLBuffer> vertices = [gpu newBufferWithBytes:triangle length:sizeof(triangle) options:MTLResourceStorageModeShared];
    MTLAccelerationStructureTriangleGeometryDescriptor *geom = [MTLAccelerationStructureTriangleGeometryDescriptor descriptor];
    geom.vertexBuffer = vertices;
    geom.vertexFormat = MTLAttributeFormatFloat3;
    geom.vertexStride = 12;
    geom.triangleCount = 1;
    geom.opaque = NO;
    MTLPrimitiveAccelerationStructureDescriptor *desc = [MTLPrimitiveAccelerationStructureDescriptor descriptor];
    desc.geometryDescriptors = @[geom];
    MTLAccelerationStructureSizes sizes = [gpu accelerationStructureSizesWithDescriptor:desc];
    id<MTLAccelerationStructure> scene = [gpu newAccelerationStructureWithSize:sizes.accelerationStructureSize];
    id<MTLBuffer> scratch = [gpu newBufferWithLength:sizes.buildScratchBufferSize options:MTLResourceStorageModePrivate];
    id<MTLCommandBuffer> build = [queue commandBuffer];
    id<MTLAccelerationStructureCommandEncoder> builder = [build accelerationStructureCommandEncoder];
    [builder buildAccelerationStructure:scene descriptor:desc scratchBuffer:scratch scratchBufferOffset:0];
    [builder endEncoding];
    finish(build);
    id<MTLFunction> callback = function(candidate, @"acceptTriangle");
    id<MTLComputePipelineState> pipe = computePipeline(function(consumer, @"trace"), callback);
    MTLIntersectionFunctionTableDescriptor *td = [MTLIntersectionFunctionTableDescriptor new];
    td.functionCount = 1;
    id<MTLIntersectionFunctionTable> table = [pipe newIntersectionFunctionTableWithDescriptor:td];
    [table setFunction:[pipe functionHandleWithFunction:callback] atIndex:0];
    uint32_t initial[] = {99, 99};
    id<MTLBuffer> hits = words(initial, 2);
    id<MTLCommandBuffer> cmd = [queue commandBuffer];
    id<MTLComputeCommandEncoder> enc = [cmd computeCommandEncoder];
    [enc setComputePipelineState:pipe];
    [enc setAccelerationStructure:scene atBufferIndex:0];
    [enc setIntersectionFunctionTable:table atBufferIndex:1];
    [enc setBuffer:hits offset:0 atIndex:2];
    [enc dispatchThreads:MTLSizeMake(2, 1, 1) threadsPerThreadgroup:MTLSizeMake(2, 1, 1)];
    [enc endEncoding];
    finish(cmd);
    return readWords(hits, 2);
}
static id<MTLRenderPipelineState> rasterPipeline(BOOL mesh) {
    NSError *error = nil;
    id<MTLRenderPipelineState> pipe;
    if (mesh) {
        MTLMeshRenderPipelineDescriptor *d = [MTLMeshRenderPipelineDescriptor new];
        d.colorAttachments[0].pixelFormat = MTLPixelFormatRGBA8Unorm;
        d.objectFunction = stage(@"object", @"makePayload");
        d.meshFunction = stage(@"mesh", @"emitTriangle");
        d.fragmentFunction = stage(@"fragment", @"shade");
        pipe = [gpu newRenderPipelineStateWithMeshDescriptor:d options:0 reflection:nil error:&error];
    } else {
        MTLRenderPipelineDescriptor *d = [MTLRenderPipelineDescriptor new];
        d.colorAttachments[0].pixelFormat = MTLPixelFormatRGBA8Unorm;
        if ([kind isEqual:@"comaps-rect"]) {
            d.vertexFunction = function(candidate, @"vsDebugRect");
            d.fragmentFunction = function(candidate, @"fsDebugRect");
            MTLVertexDescriptor *vd = [MTLVertexDescriptor vertexDescriptor];
            vd.attributes[0].format = MTLVertexFormatFloat2;
            vd.attributes[0].bufferIndex = 0;
            vd.layouts[0].stride = 8;
            d.vertexDescriptor = vd;
        } else if ([kind isEqual:@"khronos-vertex"]) {
            d.vertexFunction = function(candidate, @"main0");
            d.fragmentFunction = function(consumer, @"copiedColor");
            MTLVertexDescriptor *vd = [MTLVertexDescriptor vertexDescriptor];
            for (unsigned i = 0; i < 2; i++) {
                vd.attributes[i].format = MTLVertexFormatFloat4;
                vd.attributes[i].offset = 16 * i;
                vd.attributes[i].bufferIndex = 0;
            }
            vd.layouts[0].stride = 32;
            d.vertexDescriptor = vd;
        } else if ([kind isEqual:@"khronos-fragment"]) {
            d.vertexFunction = function(consumer, @"khronosVertices");
            d.fragmentFunction = function(candidate, @"main0");
        } else {
            d.vertexFunction = stage(@"vertex", @"origin");
            d.fragmentFunction = stage(@"fragment", @"shade");
        }
        pipe = [gpu newRenderPipelineStateWithDescriptor:d error:&error];
    }
    require(pipe != nil, error.description);
    return pipe;
}
static id<MTLRenderPipelineState> tilePipeline(id<MTLFunction> fn) {
    MTLTileRenderPipelineDescriptor *d = [MTLTileRenderPipelineDescriptor new];
    d.colorAttachments[0].pixelFormat = MTLPixelFormatRGBA8Unorm;
    d.tileFunction = fn;
    d.rasterSampleCount = 1;
    d.threadgroupSizeMatchesTileSize = YES;
    NSError *error = nil;
    id<MTLRenderPipelineState> p = [gpu newRenderPipelineStateWithTileDescriptor:d options:0 reflection:nil error:&error];
    require(p != nil, error.description);
    return p;
}
static NSArray *draw(void) {
    BOOL tile = [kind isEqual:@"tile"];
    BOOL mesh = [kind isEqual:@"mesh"] || [kind isEqual:@"object"];
    MTLTextureDescriptor *td = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA8Unorm
        width:16 height:16 mipmapped:NO];
    td.usage = MTLTextureUsageRenderTarget;
    td.storageMode = MTLStorageModeShared;
    id<MTLTexture> target = [gpu newTextureWithDescriptor:td];
    MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
    pass.colorAttachments[0].texture = target;
    pass.colorAttachments[0].loadAction = MTLLoadActionClear;
    pass.colorAttachments[0].storeAction = MTLStoreActionStore;
    pass.colorAttachments[0].clearColor = MTLClearColorMake(0, 1, 0, 1);
    id<MTLRenderPipelineState> pipe = tile ? tilePipeline(function(candidate, @"fillTile")) : rasterPipeline(mesh);
    id<MTLRenderPipelineState> reader = nil;
    id<MTLBuffer> buffer = nil;
    if (tile) {
        reader = tilePipeline(function(consumer, @"readTile"));
        pass.imageblockSampleLength = MAX(pipe.imageblockSampleLength, reader.imageblockSampleLength);
        pass.tileWidth = pass.tileHeight = 16;
        buffer = [gpu newBufferWithLength:256*sizeof(float) options:MTLResourceStorageModeShared];
        memset(buffer.contents, 0xff, buffer.length);
    }
    id<MTLCommandBuffer> cmd = [queue commandBuffer];
    id<MTLRenderCommandEncoder> enc = [cmd renderCommandEncoderWithDescriptor:pass];
    [enc setRenderPipelineState:pipe];
    if (tile) {
        [enc dispatchThreadsPerTile:MTLSizeMake(16, 16, 1)];
#if TARGET_OS_OSX
        [enc memoryBarrierWithScope:MTLBarrierScopeRenderTargets afterStages:MTLRenderStageTile beforeStages:MTLRenderStageTile];
#endif
        [enc setRenderPipelineState:reader];
        [enc setTileBuffer:buffer offset:0 atIndex:0];
        [enc dispatchThreadsPerTile:MTLSizeMake(16, 16, 1)];
    } else if (mesh) {
        [enc drawMeshThreadgroups:MTLSizeMake(1, 1, 1) threadsPerObjectThreadgroup:MTLSizeMake(1, 1, 1)
            threadsPerMeshThreadgroup:MTLSizeMake(1, 1, 1)];
    } else if ([kind isEqual:@"comaps-rect"]) {
        const float xy[][2] = {{-1,-1}, {3,-1}, {-1,3}};
        const float color[] = {0.75f,0.25f,0,1};
        [enc setVertexBytes:xy length:sizeof(xy) atIndex:0];
        [enc setFragmentBytes:color length:sizeof(color) atIndex:0];
        [enc drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:3];
    } else if ([kind isEqual:@"khronos-vertex"]) {
        const float data[][8] = {{-1,-1,0,1, 0,1,1,1}, {3,-1,0,1, 0,1,1,1}, {-1,3,0,1, 0,1,1,1}};
        [enc setVertexBytes:data length:sizeof(data) atIndex:0];
        [enc drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:3];
    } else {
        [enc drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:3];
    }
    [enc endEncoding];
    finish(cmd);
    if (tile) {
        const float *p = buffer.contents;
        for (unsigned i = 0; i < 256; i++) require(isfinite(p[i]) && p[i] == p[0], @"inconsistent tile");
        return @[@(p[0]), @256];
    }
    uint8_t pixels[256][4];
    [target getBytes:pixels bytesPerRow:64 fromRegion:MTLRegionMake2D(0,0,16,16) mipmapLevel:0];
    for (unsigned i = 1; i < 256; i++) require(memcmp(pixels[0], pixels[i], 4) == 0, @"nonuniform framebuffer");
    return @[@(pixels[0][0]), @(pixels[0][1]), @(pixels[0][2]), @(pixels[0][3]), @256];
}

static NSDictionary *reflect(int count, const char **paths) {
    NSMutableDictionary *cases = [NSMutableDictionary new];
    for (int i = 0; i < count; i++) { @autoreleasepool {
        NSString *path = @(paths[i]);
        NSString *key = path.lastPathComponent.stringByDeletingPathExtension;
        @try {
            id<MTLLibrary> lib = load(path);
            NSMutableDictionary *functions = [NSMutableDictionary new];
            for (NSString *name in lib.functionNames) {
                id<MTLFunction> fn = function(lib, name);
                NSMutableDictionary *attributes = [NSMutableDictionary new];
                if (fn.functionType == MTLFunctionTypeVertex) {
                    for (MTLVertexAttribute *attr in fn.vertexAttributes) {
                        attributes[attr.name] = @[@(attr.attributeIndex), @(attr.attributeType), @(attr.active)];
                    }
                }
                functions[name] = @{@"type":@(fn.functionType), @"attributes":attributes};
            }
            cases[key] = functions;
        } @catch (NSException *e) {
            cases[key] = @{@"error":e.reason};
        }
    } }
    return cases;
}

static void printJSON(NSDictionary *result) {
    NSData *json = [NSJSONSerialization dataWithJSONObject:result options:NSJSONWritingSortedKeys error:nil];
    fwrite(json.bytes, 1, json.length, stdout);
    puts("");
}

int main(int argc, const char **argv) {
    @autoreleasepool { @try {
        gpu = MTLCreateSystemDefaultDevice();
        require(gpu != nil, @"no Metal GPU available");
        if (argc >= 2 && strcmp(argv[1], "reflect") == 0) {
            printJSON(reflect(argc - 2, argv + 2));
            return 0;
        }
        require(argc == 6, @"usage: Runtime kind candidate.metallib reference-dir version Consumer.metallib");
        queue = [gpu newCommandQueue];
        kind = @(argv[1]);
        controlDir = @(argv[3]);
        version = @(argv[4]);
        candidate = load(@(argv[2]));
        if (![@[@"compute", @"language", @"comaps-rect"] containsObject:kind]) consumer = load(@(argv[5]));
        NSArray *values;
        if ([@[@"compute", @"language", @"visible", @"stitchable"] containsObject:kind]) values = arithmetic();
        else if ([kind isEqual:@"khronos-compute"]) values = khronosCompute();
        else if ([kind isEqual:@"intersection"]) values = traceRays();
        else values = draw();
        NSDictionary *result = @{@"gpu":gpu.name, @"kind":kind, @"values":values};
        printJSON(result);
        return 0;
    } @catch (NSException *e) {
        fprintf(stderr, "%s\n", e.reason.UTF8String);
        return 1;
    } }
}
