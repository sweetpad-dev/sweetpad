"""Lower Metal stage interfaces to AIR. Clang compiles the shader bodies."""

import re

from msl import Field, primitive, split


LAYOUT = (
    "e-p:64:64:64-i1:8:8-i8:8:8-i16:16:16-i32:32:32-i64:64:64-f32:32:32-f64:64:64-"
    "v16:16:16-v24:32:32-v32:32:32-v48:64:64-v64:64:64-v96:128:128-v128:128:128-"
    "v192:256:256-v256:256:256-v512:512:512-v1024:1024:1024-n8:16:32"
)
GPU_TYPES = r"""
struct _mesh_grid_properties_t;
struct _mesh_t;
using sp_grid = _mesh_grid_properties_t __attribute__((address_space(3))) *;
using sp_mesh = _mesh_t __attribute__((address_space(7))) *;
extern "C" void sp_set_grid(sp_grid, uint3) asm("air.set_threadgroups_per_grid_mesh_properties");
extern "C" void sp_set_position(sp_mesh, uint, float4) asm("air.set_position_mesh");
extern "C" void sp_set_index(sp_mesh, uint, uchar) asm("air.set_index_mesh");
extern "C" void sp_set_count(sp_mesh, uint) asm("air.set_primitive_count_mesh");
using sp_pixel = uchar __attribute__((address_space(4))) *;
extern "C" sp_pixel sp_pixel_data(ushort2, uint, ushort) asm("air.imageblock_data");
namespace metal {
struct mesh_grid_properties {
    sp_grid handle;
    void set_threadgroups_per_grid(uint3 n) const { sp_set_grid(handle, n); }
};
enum class topology { triangle };
template<class V, class P, uint NV, uint NP, topology T> struct mesh {
    sp_mesh handle;
    void set_vertex(uint i, V v) const { sp_write_vertex(handle, i, v); }
    void set_index(uint i, uint value) const { sp_set_index(handle, i, (uchar)value); }
    void set_primitive_count(uint n) const { sp_set_count(handle, n); }
};
enum imageblock_layout { imageblock_layout_explicit };
template<class T, imageblock_layout L> struct imageblock {
    T __attribute__((address_space(4))) *data(ushort2 xy) const {
        return reinterpret_cast<T __attribute__((address_space(4))) *>(sp_pixel_data(xy, 0, 0));
    }
};
}
"""


class Metadata:
    def __init__(self):
        self.nodes, self.groups = [], {}

    def node(self, *values):
        ref = f"!{100000 + len(self.nodes)}"
        self.nodes.append(f"{ref} = !{{{', '.join(values)}}}")
        return ref

    def add(self, group, *values):
        self.groups.setdefault(group, []).append(self.node(*values))

    def emit(self):
        return "\n".join(
            [f"!air.{k} = !{{{', '.join(v)}}}" for k, v in self.groups.items()] + self.nodes
        )


def string(value):
    return '!"' + value + '"'


def integer(value):
    return f"i32 {int(value)}"


def label(type_name, field=None):
    parts = [string("air.arg_type_name"), string(type_name)]
    if field:
        parts.extend([string("air.arg_name"), string(field)])
    return parts


def exact(attrs, allowed):
    extra = attrs.keys() - allowed
    if extra:
        raise ValueError("unsupported Metal attribute " + ", ".join(sorted(extra)))


def varying(field):
    annotation = field.attrs.get("user")
    return string(
        "user(" + annotation + ")"
        if annotation
        else f"generated({len(field.name)}{field.name}{primitive(field.type)[2]})"
    )


BUILTINS = {
    "vertex_id",
    "instance_id",
    "base_vertex",
    "base_instance",
    "primitive_id",
    "thread_position_in_grid",
    "thread_position_in_threadgroup",
    "thread_index_in_threadgroup",
    "threadgroup_position_in_grid",
    "threads_per_threadgroup",
    "threadgroups_per_grid",
    "barycentric_coord",
}
VERTEX_IDS = {"vertex_id", "instance_id", "base_vertex", "base_instance"}
THREAD_IDS = {tag for tag in BUILTINS if tag.startswith("thread")}


class Stage:
    def __init__(self, entry, structs, aliases, md):
        self.e, self.structs, self.aliases, self.md = entry, structs, aliases, md
        self.inputs, self.outputs, self.extra = [], [], []
        self.parameters, self.cpp_params, self.call, self.body_params = [], [], [], []
        self.output_types, self.values = [], []
        self.special, self.helpers, self.forwards = [], [], []
        exact(entry.attrs, {entry.kind, "max_total_threads_per_threadgroup"})
        for p in entry.params:
            self.body_params.append(self.parameter(p))
        self.result()
        if entry.kind == "intersection":
            tags = split(entry.attrs["intersection"])
            if tags != ["triangle", "triangle_data"]:
                raise ValueError("only triangle intersections with triangle_data are implemented")
            self.extra.extend(string("air." + t) for t in tags)
        if "max_total_threads_per_threadgroup" in entry.attrs:
            if entry.kind not in {"object", "mesh", "kernel"}:
                raise ValueError("max_total_threads_per_threadgroup requires a threadgroup stage")
            if int(entry.attrs["max_total_threads_per_threadgroup"]) <= 0:
                raise ValueError("max_total_threads_per_threadgroup must be positive")
            self.extra.append(
                md.node(
                    string("air.max_work_group_size"),
                    integer(entry.attrs["max_total_threads_per_threadgroup"]),
                )
            )
        if entry.kind == "stitchable":
            self.stitch()

    def fields(self, name):
        if name not in self.structs:
            raise ValueError(f"unknown Metal structure '{name}'")
        return [Field.parse(s) for s in split(self.structs[name], ";")]

    def layout(self, name):
        if name not in self.structs:
            size = primitive(name)[1]
            return size, size, []
        offset, alignment, info = 0, 1, []
        for field in self.fields(name):
            exact(field.attrs, {"raster_order_group"})
            if field.attrs.get("raster_order_group", "0") != "0":
                raise ValueError("only raster order group zero is implemented")
            size = primitive(field.type)[1]
            offset = (offset + size - 1) // size * size
            info.extend(
                [integer(offset), integer(size), integer(0), string(field.type), string(field.name)]
            )
            offset += size
            alignment = max(alignment, size)
        return (offset + alignment - 1) // alignment * alignment, alignment, info

    def input(self, ir, cpp, tags):
        i = len(self.inputs)
        self.inputs.append(self.md.node(integer(i), *tags))
        name = f"a{i}"
        self.parameters.append((ir, name))
        if cpp:
            self.cpp_params.append(f"{cpp} {name}")
            self.call.append(f"{ir} %{name}")
        return name

    def scalar_field(self, f):
        ir = primitive(f.type)[0]
        if self.e.kind == "fragment":
            exact(f.attrs, {"position", "user", "flat", "center_perspective", "center_no_perspective"})
            if "position" in f.attrs:
                tags = [string("air.position"), string("air.center"), string("air.no_perspective")]
            else:
                tags = [string("air.fragment_input"), varying(f)]
                if "flat" in f.attrs:
                    tags.append(string("air.flat"))
                else:
                    tags.extend(
                        [
                            string("air.center"),
                            string(
                                "air.no_perspective"
                                if "center_no_perspective" in f.attrs
                                else "air.perspective"
                            ),
                        ]
                    )
        elif self.e.kind == "vertex" and "attribute" in f.attrs:
            exact(f.attrs, {"attribute"})
            slot = int(f.attrs["attribute"])
            if not 0 <= slot < 31:
                raise ValueError("vertex attribute index must be between 0 and 30")
            tags = [string("air.vertex_input"), string("air.location_index"), integer(slot), integer(1)]
        else:
            raise ValueError(f"unsupported {self.e.kind} stage input '{f.name}'")
        return self.input(ir, f.type, tags + label(f.type, f.name))

    def parameter(self, p):
        typ = self.aliases.get(p.type, p.type)
        if "stage_in" in p.attrs:
            exact(p.attrs, {"stage_in"})
            value_type = re.sub(r"\bconst\b", "", typ).strip()
            return (
                value_type + "{" + ", ".join(self.scalar_field(f) for f in self.fields(value_type)) + "}"
            )
        if "buffer" in p.attrs or "payload" in p.attrs:
            payload = "payload" in p.attrs
            exact(p.attrs, {"payload"} if payload else {"buffer"})
            m = re.fullmatch(r"((?:(?:const|device|object_data|constant)\s+)+)(\w+)\s*([*&])", typ)
            if not m:
                raise ValueError(f"unsupported Metal address-space parameter '{typ}'")
            quals, name, ptr = m.groups()
            space = 6 if payload else 2 if "constant" in quals else 1 if "device" in quals else 0
            if not space or (payload and "object_data" not in quals):
                raise ValueError("invalid buffer or payload address space")
            size, align, members = self.layout(name)
            ir = "%struct." + name if name in self.structs else primitive(name)[0]
            tags = (
                [string("air.payload")]
                if payload
                else [
                    string("air.buffer"),
                    string("air.location_index"),
                    integer(p.attrs["buffer"]),
                    integer(1),
                    string("air.read" if "const" in quals or space == 2 else "air.read_write"),
                    string("air.address_space"),
                    integer(space),
                ]
            )
            if members:
                tags.extend([string("air.struct_type_info"), self.md.node(*members)])
            tags += [
                string("air.arg_type_size"),
                integer(size),
                string("air.arg_type_align_size"),
                integer(align),
            ]
            arg = self.input(f"{ir} addrspace({space})*", f"{quals}{name} *", tags + label(name, p.name))
            return ("*" if ptr == "&" else "") + arg
        if typ == "mesh_grid_properties" and self.e.kind == "object":
            exact(p.attrs, set())
            arg = self.input(
                "%struct._mesh_grid_properties_t addrspace(3)*",
                "sp_grid",
                [string("air.mesh_grid_properties")] + label(typ, p.name),
            )
            return typ + "{" + arg + "}"
        if "mesh<" in typ and self.e.kind == "mesh":
            return self.mesh(p, typ)
        if typ.startswith("imageblock<"):
            exact(p.attrs, set())
            sub = split(typ[typ.index("<") + 1 : -1])
            if len(sub) != 2 or sub[1] != "imageblock_layout_explicit":
                raise ValueError("only explicit imageblocks are implemented")
            size, align, members = self.layout(sub[0])
            self.input(
                '%"struct.metal::_imageblock_base"',
                None,
                [
                    string("air.imageblock"),
                    string("explicit"),
                    string("air.imageblock_data_size"),
                    integer(size),
                    string("air.struct_type_info"),
                    self.md.node(*members),
                    string("air.arg_type_align_size"),
                    integer(align),
                ]
                + label(f"imageblock<{sub[0]}, layout_explicit>", p.name),
            )
            return typ + "{}"
        semantics = p.attrs.keys() & BUILTINS
        if len(semantics) == 1:
            exact(p.attrs, semantics)
            tag = next(iter(semantics))
            allowed = {
                "vertex": VERTEX_IDS,
                "fragment": {"primitive_id"},
                "intersection": {"barycentric_coord", "primitive_id"},
                "object": THREAD_IDS,
                "mesh": THREAD_IDS,
                "kernel": THREAD_IDS,
            }
            if tag not in allowed.get(self.e.kind, set()):
                raise ValueError(f"unsupported {self.e.kind} input attribute '{tag}'")
        elif self.e.kind in {"visible", "stitchable"} and not p.attrs:
            tag = "visible_input"
        else:
            raise ValueError(f"unsupported {self.e.kind} input '{p.name}' ({', '.join(p.attrs)})")
        return self.input(primitive(typ)[0], typ, [string("air." + tag)] + label(typ, p.name))

    def mesh(self, p, typ):
        exact(p.attrs, set())
        sub = split(typ[typ.index("<") + 1 : -1])
        if len(sub) != 5 or sub[1] != "void" or sub[4] != "topology::triangle" or int(sub[2]) > 256:
            raise ValueError("unsupported mesh topology, primitive attributes, or index width")
        vertex = sub[0]
        fields, writes, slot = [], [], 0
        for f in self.fields(vertex):
            exact(f.attrs, {"position", "user"})
            if "position" in f.attrs:
                if f.type != "float4":
                    raise ValueError("mesh position requires float4")
                tag = [string("air.position")]
                writes.append(f"sp_set_position(h, i, v.{f.name});")
            else:
                if f.type != "float":
                    raise ValueError("only scalar float mesh varyings are implemented")
                tag = [string("air.mesh_vertex_data"), integer(slot), varying(f)]
                writes.append(f"sp_vertex_f32(h, {slot}, i, v.{f.name});")
                slot += 1
            fields.append(self.md.node(*(tag + label(f.type, f.name))))
        signature = f"inline void sp_write_vertex(sp_mesh h, uint i, const {vertex}& v)"
        self.forwards += [f"struct {vertex};", signature + ";"]
        self.helpers += [
            'extern "C" void sp_vertex_f32(sp_mesh, uint, uint, float) asm("air.set_vertex_data_mesh.f32");',
            signature + " { " + " ".join(writes) + " }",
        ]
        descriptor = self.md.node(
            string("air.mesh_type_info"),
            self.md.node(*fields),
            self.md.node(),
            integer(sub[2]),
            integer(sub[3]),
            string("air.triangle"),
        )
        arg = self.input(
            "%struct._mesh_t addrspace(7)*",
            "sp_mesh",
            [string("air.mesh"), descriptor]
            + label("mesh<" + ", ".join(sub[:4] + ["triangle"]) + ">", p.name),
        )
        return p.type + "{" + arg + "}"

    def result(self):
        e = self.e
        if e.result == "void":
            return
        ret = self.fields(e.result) if e.result in self.structs else [Field(e.result, "", {})]
        for f in ret:
            ir = primitive(f.type)[0]
            if e.kind == "vertex":
                exact(f.attrs, {"position", "user"})
                tag = (
                    [string("air.position")]
                    if "position" in f.attrs or not f.name
                    else [string("air.vertex_output"), varying(f)]
                )
            elif e.kind == "fragment":
                exact(f.attrs, {"color"})
                tag = [string("air.render_target"), integer(f.attrs.get("color", 0)), integer(0)]
            elif e.kind == "intersection" and f.type == "bool":
                tag = [string("air.accept_intersection")]
            elif e.kind in {"visible", "stitchable"}:
                tag = [string("air.visible_output")]
            else:
                raise ValueError(f"unsupported {e.kind} result '{e.result}'")
            self.outputs.append(self.md.node(*(tag + label(f.type, f.name))))
            n = len(self.values)
            mem = "i8" if ir == "i1" else ir
            self.cpp_params.append(f"{f.type} *o{n}")
            self.call.append(f"{mem}* %o{n}")
            self.output_types.append(ir)
            self.values.append(f"*o{n} = result" + ("." + f.name if f.name else "") + ";")

    def stitch(self):
        e = self.e
        if e.result not in {"uint", "int", "float"} or any(p.type != e.result for p in e.params):
            raise ValueError("stitching currently supports homogeneous 32-bit scalars")
        ir, size, _ = primitive(e.result)
        # AIR stores the conversion functions by pointer. Their names are private to the module.
        name = "__sp_stitch_" + e.name
        definitions = [
            (
                ir,
                "load",
                "i8* %p",
                f"%v = bitcast i8* %p to {ir}*\n%x = load {ir}, {ir}* %v\nret {ir} %x",
            ),
            (
                "void",
                "store",
                f"i8* %p, {ir} %x",
                f"%v = bitcast i8* %p to {ir}*\nstore {ir} %x, {ir}* %v\nret void",
            ),
            ("void", "destroy", "i8* %p", "ret void"),
            (
                "void",
                "copy",
                "i8* %p, i8 addrspace(2)* %c",
                f"%v = bitcast i8* %p to {ir}*\n%k = bitcast i8 addrspace(2)* %c to {ir} addrspace(2)*\n"
                f"%x = load {ir}, {ir} addrspace(2)* %k\nstore {ir} %x, {ir}* %v\nret void",
            ),
        ]
        fnrefs = []
        for ret, op, args, body in definitions:
            symbol = f"@{name}_{op}"
            # LLVM's global DCE does not treat named metadata as a live reference.
            # Keep these entry-specific definitions available to Metal's stitching graph.
            self.special.append(f"define hidden {ret} {symbol}({args}) alwaysinline {{\n{body}\n}}")
            argtypes = re.sub(r" %\w+", "", args)
            fnrefs.append(f"{ret} ({argtypes})* {symbol}")
        dtype = self.md.node(string("air." + e.result + "_type"), integer(size), integer(size), "null")
        traits = self.md.node(string("air.stitching_type"), dtype, *fnrefs)
        args = [self.md.node(string("air.stitching_argument"), traits, string(p.name)) for p in e.params]
        self.extra.append(self.md.node(string("air.stitching_info"), traits, *args))

    def cpp(self):
        e = self.e
        body = f"__sp_body_{e.name}({', '.join(self.body_params)});"
        if self.values:
            body = "auto result = " + body + "\n" + "\n".join(self.values)
        return (
            'extern "C" void __sp_impl_'
            + e.name
            + "("
            + ", ".join(self.cpp_params)
            + ") {\n"
            + body
            + "\n}"
        )

    def llvm(self):
        types = self.output_types
        aggregate = self.e.result in self.structs
        ret = "<{ " + ", ".join(types) + " }>" if aggregate else next(iter(types), "void")
        args = ", ".join(f"{t} %{p}" for t, p in self.parameters)
        body = [f"define {ret} @{self.e.name}({args}) {{"]
        mem = ["i8" if t == "i1" else t for t in types]
        body += [f"%o{i} = alloca {m}" for i, m in enumerate(mem)]
        body.append(f"call void @__sp_impl_{self.e.name}({', '.join(self.call)})")
        value = ""
        for i, (t, m) in enumerate(zip(types, mem)):
            body.append(f"%v{i} = load {m}, {m}* %o{i}")
            v = f"%v{i}"
            if m != t:
                body.append(f"%b{i} = trunc {m} {v} to {t}")
                v = f"%b{i}"
            if aggregate:
                body.append(f"%s{i} = insertvalue {ret} {value or 'undef'}, {t} {v}, {i}")
                v = f"%s{i}"
            value = v
        body += [f"ret {ret} {value}", "}"]
        function = f"{ret} ({', '.join(t for t, p in self.parameters)})* @{self.e.name}"
        kind = "visible" if self.e.kind == "stitchable" else self.e.kind
        self.md.add(kind, function, self.md.node(*self.outputs), self.md.node(*self.inputs), *self.extra)
        return "\n".join(body + self.special)
