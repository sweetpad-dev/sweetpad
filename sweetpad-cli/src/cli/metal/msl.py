"""MSL entry point declarations and the subset of the Metal standard library we lower."""

import itertools
import re
from dataclasses import dataclass


SCALARS = {
    "bool": ("i1", 1, "b"),
    "char": ("i8", 1, "c"),
    "uchar": ("i8", 1, "h"),
    "short": ("i16", 2, "s"),
    "ushort": ("i16", 2, "t"),
    "half": ("half", 2, "Dh"),
    "int": ("i32", 4, "i"),
    "uint": ("i32", 4, "j"),
    "float": ("float", 4, "f"),
}
STAGES = {"vertex", "fragment", "kernel", "visible", "stitchable", "intersection", "object", "mesh"}
ATTR = re.compile(r"\[\[(.*?)\]\]", re.S)
NAME = r"[A-Za-z_]\w*"


def split(text, separator=","):
    parts, start, depth = [], 0, 0
    for i, c in enumerate(text):
        if c in "(<[{":
            depth += 1
        elif c in ")>]}":
            depth -= 1
        elif c == separator and depth == 0:
            parts.append(text[start:i].strip())
            start = i + 1
    if text[start:].strip():
        parts.append(text[start:].strip())
    return parts


def primitive(name):
    m = re.fullmatch(r"([a-z]+)([234]?)", name)
    if not m or m[1] not in SCALARS:
        raise ValueError(f"unsupported Metal value type '{name}'")
    ir, size, mangled = SCALARS[m[1]]
    n = int(m[2] or 1)
    return (
        ir if n == 1 else f"<{n} x {ir}>",
        size * (4 if n == 3 else n),
        mangled if n == 1 else f"Dv{n}_{mangled}",
    )


def stdlib(language):
    lines = [
        """
typedef unsigned char uchar;
typedef unsigned short ushort;
typedef unsigned int uint;
typedef unsigned long ulong;
typedef _Float16 half;
#define device __attribute__((address_space(1)))
#define constant __attribute__((address_space(2)))
#define threadgroup __attribute__((address_space(3)))
#define object_data __attribute__((address_space(6)))
#define thread
#define SP_INLINE static inline __attribute__((always_inline))
namespace metal { namespace raytracing {} }
"""
    ]
    vectors = [base for base in SCALARS if base != "bool"]
    for base in vectors:
        for n in (2, 3, 4):
            lines.append(f"typedef {base} {base}{n} __attribute__((ext_vector_type({n})));")
    for base in vectors:
        for n in (2, 3, 4):
            vec = f"{base}{n}"
            fun = f"__sp_make_{vec}"
            lines.append(f"SP_INLINE {vec} {fun}({base} v) {{ return ({vec})v; }}")
            for other in vectors:
                lines.append(
                    f"SP_INLINE {vec} {fun}({other}{n} v) "
                    f"{{ return __builtin_convertvector(v, {vec}); }}"
                )
            for arity in range(2, n + 1):
                for sizes in itertools.product(range(1, n), repeat=arity):
                    if sum(sizes) != n:
                        continue
                    args, values = [], []
                    for i, k in enumerate(sizes):
                        args.append(f"{base}{k if k > 1 else ''} a{i}")
                        values.extend([f"a{i}[{j}]" for j in range(k)] if k > 1 else [f"a{i}"])
                    lines.append(
                        f"SP_INLINE {vec} {fun}({', '.join(args)}) {{ return {{{', '.join(values)}}}; }}"
                    )
            lines.append(f"#define {vec}(...) {fun}(__VA_ARGS__)")
    if language >= (4, 1):
        lines.append(
            "inline void *operator new(__SIZE_TYPE__, void *storage) noexcept { return storage; }"
        )
    return "\n".join(lines) + "\n"


def attributes(text):
    attrs = {}
    for group in ATTR.findall(text):
        for item in split(group):
            m = re.fullmatch(r"(?:(?:metal|clang)::)?(" + NAME + r")(?:\((.*)\))?", item, re.S)
            if not m or m[1] in attrs:
                raise ValueError(f"unsupported or duplicate Metal attribute '{item}'")
            attrs[m[1]] = (m[2] or "").strip()
    return attrs


@dataclass
class Field:
    type: str
    name: str
    attrs: dict

    @classmethod
    def parse(cls, text):
        attrs = attributes(text)
        decl = ATTR.sub("", text).strip()
        m = re.fullmatch(r"(.+?)([A-Za-z_]\w*)", decl, re.S)
        if not m or not m[1].strip() or decl[-1] == "]":
            raise ValueError(f"unsupported Metal declaration '{decl}'")
        return cls(" ".join(m[1].split()), m[2], attrs)


@dataclass
class Entry:
    kind: str
    result: str
    name: str
    params: list
    attrs: dict


def parse(source, language):
    # Inspect only declaration boundaries. Leave expressions and function bodies to Clang.
    structs, aliases, entries, chunks = {}, {}, [], []
    source = re.sub(r"^\s*#pragma[^\n]*", "", source, flags=re.M)
    start, i = 0, 0
    while i < len(source):
        if source[i] == ";":
            decl = source[start : i + 1]
            alias = re.fullmatch(r"\s*using\s+(" + NAME + r")\s*=\s*(.*?)\s*;", decl, re.S)
            if alias:
                aliases[alias[1]] = " ".join(alias[2].split())
            chunks.append(decl)
            start = i + 1
        elif source[i] == "{":
            pattern = re.search(r"[{}]", source[i + 1 :])
            depth, end = 1, i + 1
            while depth and pattern:
                end += pattern.end()
                depth += 1 if source[end - 1] == "{" else -1
                if depth:
                    pattern = re.search(r"[{}]", source[end:])
            if depth:
                raise ValueError("unbalanced Metal source")
            head, body = source[start:i], source[i:end]
            if re.fullmatch(r"\s*typedef\s+struct\s*", head):
                alias = re.match(r"\s*(" + NAME + r")\s*;", source[end:])
                if not alias:
                    raise ValueError("unsupported anonymous struct typedef")
                structs[alias[1]] = body[1:-1]
                chunks.append(f"struct {alias[1]} {body};")
                start = i = end + alias.end()
                continue
            struct = re.fullmatch(r"\s*struct\s+(" + NAME + r")\s*", head)
            entry_words = re.findall(r"\b(" + "|".join(sorted(STAGES)) + r")\b", head)
            if struct:
                structs[struct[1]] = body[1:-1]
            elif entry_words:
                prefix = re.match(r"\s*((?:\[\[.*?\]\]\s*)*)", head, re.S)
                attrs = attributes(prefix[1])
                m = re.fullmatch(
                    r"\s*(?:\[\[.*?\]\]\s*)*(?:(vertex|fragment|kernel)\s+)?"
                    r"(" + NAME + r")\s+(" + NAME + r")\s*\((.*)\)\s*",
                    head,
                    re.S,
                )
                if not m:
                    raise ValueError("unsupported Metal entry declaration: " + head.strip()[:140])
                stage = m[1] or next((k for k in attrs if k in STAGES), None)
                if stage is None:
                    raise ValueError("missing Metal function kind")
                if stage in {"mesh", "object"} and language < (3, 0):
                    raise ValueError(f"{stage} functions require MSL 3.0 or later")
                params = [Field.parse(p) for p in split(m[4]) if p != "void"]
                entries.append(Entry(stage, m[2], m[3], params, attrs))
                head = "SP_INLINE " + m[2] + " __sp_body_" + m[3] + "(" + m[4] + ")"
            chunks.append(head + body)
            start = end
            i = end - 1
        i += 1
    chunks.append(source[start:])
    if not entries:
        raise ValueError("no supported Metal entry point")
    if len({e.name for e in entries}) != len(entries):
        raise ValueError("duplicate or overloaded Metal entry point")
    return structs, aliases, entries, "".join(chunks)
