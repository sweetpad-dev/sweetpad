//! Binary property lists, and the `NSKeyedArchiver` archives written in
//! them: every object in one `$objects` array, each referring to the others
//! by their index there, a UID, so an object graph with cycles, like a Core
//! Data entity and its model, fits a tree. The momc stand-in writes its
//! models this way; the ibtool one writes a storyboard's `Info.plist` as a
//! plain binary property list.

use std::collections::HashMap;

/// A property list value. `Uid` is a keyed archive's reference to an
/// object in `$objects`; `Real32` is a single-precision real, which is how
/// a `Float` attribute's default is kept.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Plist {
    Bool(bool),
    Int(i64),
    Real(f64),
    Real32(f32),
    String(String),
    Data(Vec<u8>),
    Uid(u64),
    Array(Vec<Plist>),
    Dict(Vec<(String, Plist)>),
}

/// The reference to `$null`, which stands for nil.
pub(crate) const NULL: Plist = Plist::Uid(0);

/// A class's name and its superclasses', as the archive records it.
pub(crate) type Class = &'static [&'static str];

/// Builds an `NSKeyedArchiver` archive one object at a time.
pub(crate) struct Archiver {
    objects: Vec<Plist>,
    classes: HashMap<&'static str, u64>,
    strings: HashMap<String, u64>,
}

impl Archiver {
    pub fn new() -> Self {
        Archiver {
            objects: vec![Plist::String("$null".to_string())],
            classes: HashMap::new(),
            strings: HashMap::new(),
        }
    }

    /// A place for an object that others refer to before it's written.
    pub fn reserve(&mut self) -> u64 {
        self.objects.push(Plist::Bool(false));
        self.objects.len() as u64 - 1
    }

    /// A reference to the string `text`, written once however often it's
    /// referred to.
    pub fn string(&mut self, text: &str) -> Plist {
        if let Some(&uid) = self.strings.get(text) {
            return Plist::Uid(uid);
        }
        self.objects.push(Plist::String(text.to_string()));
        let uid = self.objects.len() as u64 - 1;
        self.strings.insert(text.to_string(), uid);
        Plist::Uid(uid)
    }

    /// A reference to the string `text`, or nil.
    pub fn optional_string(&mut self, text: Option<&str>) -> Plist {
        text.map_or(NULL, |text| self.string(text))
    }

    /// A reference to `value` as an object of its own, like a number an
    /// attribute has for its default.
    pub fn value(&mut self, value: Plist) -> Plist {
        if let Plist::String(text) = &value {
            return self.string(&text.clone());
        }
        self.objects.push(value);
        Plist::Uid(self.objects.len() as u64 - 1)
    }

    fn class(&mut self, class: Class) -> Plist {
        if let Some(&uid) = self.classes.get(class[0]) {
            return Plist::Uid(uid);
        }
        self.objects.push(Plist::Dict(vec![
            (
                "$classname".to_string(),
                Plist::String(class[0].to_string()),
            ),
            (
                "$classes".to_string(),
                Plist::Array(
                    class
                        .iter()
                        .map(|name| Plist::String((*name).to_string()))
                        .collect(),
                ),
            ),
        ]));
        let uid = self.objects.len() as u64 - 1;
        self.classes.insert(class[0], uid);
        Plist::Uid(uid)
    }

    /// An object of `class` with `fields`, at `at` when it was reserved.
    pub fn object(&mut self, at: Option<u64>, class: Class, fields: Vec<(&str, Plist)>) -> Plist {
        let class = self.class(class);
        let mut dict: Vec<(String, Plist)> = fields
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect();
        dict.push(("$class".to_string(), class));
        let dict = Plist::Dict(dict);
        if let Some(uid) = at {
            self.objects[usize::try_from(uid).expect("an index into $objects")] = dict;
            return Plist::Uid(uid);
        }
        self.objects.push(dict);
        Plist::Uid(self.objects.len() as u64 - 1)
    }

    /// An `NSDictionary`, or its mutable kind, of references.
    pub fn dictionary(&mut self, mutable: bool, entries: Vec<(Plist, Plist)>) -> Plist {
        let class: Class = if mutable {
            &["NSMutableDictionary", "NSDictionary", "NSObject"]
        } else {
            &["NSDictionary", "NSObject"]
        };
        let (keys, objects) = entries.into_iter().unzip();
        self.object(
            None,
            class,
            vec![
                ("NS.keys", Plist::Array(keys)),
                ("NS.objects", Plist::Array(objects)),
            ],
        )
    }

    /// An `NSArray`, `NSMutableArray` or `NSSet` (whichever `class` is) of
    /// references.
    pub fn collection(&mut self, class: Class, items: Vec<Plist>) -> Plist {
        self.object(None, class, vec![("NS.objects", Plist::Array(items))])
    }

    /// The archive, with `root` as its root object.
    pub fn finish(self, root: Plist) -> Vec<u8> {
        write(&Plist::Dict(vec![
            ("$version".to_string(), Plist::Int(100_000)),
            (
                "$archiver".to_string(),
                Plist::String("NSKeyedArchiver".to_string()),
            ),
            (
                "$top".to_string(),
                Plist::Dict(vec![("root".to_string(), root)]),
            ),
            ("$objects".to_string(), Plist::Array(self.objects)),
        ]))
    }
}

pub(crate) const ARRAY: Class = &["NSArray", "NSObject"];
pub(crate) const MUTABLE_ARRAY: Class = &["NSMutableArray", "NSArray", "NSObject"];
pub(crate) const SET: Class = &["NSSet", "NSObject"];

/// One object of a binary property list: a value's bytes, or a container's
/// marker and the objects it holds, by index.
enum Flat {
    Bytes(Vec<u8>),
    Array(Vec<usize>),
    Dict(Vec<usize>, Vec<usize>),
}

/// `value` as a `bplist00` file. Equal values that aren't containers are
/// written once.
pub(crate) fn write(value: &Plist) -> Vec<u8> {
    let mut objects = Vec::new();
    let mut seen = HashMap::new();
    flatten(value, &mut objects, &mut seen);
    let ref_size = size_of_uint(objects.len() as u64);
    let mut out = b"bplist00".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for object in &objects {
        offsets.push(out.len() as u64);
        match object {
            Flat::Bytes(bytes) => out.extend_from_slice(bytes),
            Flat::Array(items) => {
                marker(0xa, items.len(), &mut out);
                for &item in items {
                    uint(item as u64, ref_size, &mut out);
                }
            }
            Flat::Dict(keys, values) => {
                marker(0xd, keys.len(), &mut out);
                for &item in keys.iter().chain(values) {
                    uint(item as u64, ref_size, &mut out);
                }
            }
        }
    }
    let table = out.len() as u64;
    let offset_size = size_of_uint(table);
    for offset in offsets {
        uint(offset, offset_size, &mut out);
    }
    out.extend_from_slice(&[0; 6]);
    out.push(u8::try_from(offset_size).expect("at most 8"));
    out.push(u8::try_from(ref_size).expect("at most 8"));
    out.extend_from_slice(&(objects.len() as u64).to_be_bytes());
    out.extend_from_slice(&0u64.to_be_bytes());
    out.extend_from_slice(&table.to_be_bytes());
    out
}

fn flatten(value: &Plist, objects: &mut Vec<Flat>, seen: &mut HashMap<Vec<u8>, usize>) -> usize {
    match value {
        Plist::Array(items) => {
            let at = objects.len();
            objects.push(Flat::Array(Vec::new()));
            let items = items.iter().map(|i| flatten(i, objects, seen)).collect();
            objects[at] = Flat::Array(items);
            at
        }
        Plist::Dict(entries) => {
            let at = objects.len();
            objects.push(Flat::Dict(Vec::new(), Vec::new()));
            let keys = entries
                .iter()
                .map(|(k, _)| flatten(&Plist::String(k.clone()), objects, seen))
                .collect();
            let values = entries
                .iter()
                .map(|(_, v)| flatten(v, objects, seen))
                .collect();
            objects[at] = Flat::Dict(keys, values);
            at
        }
        scalar => {
            let bytes = encode(scalar);
            if let Some(&at) = seen.get(&bytes) {
                return at;
            }
            objects.push(Flat::Bytes(bytes.clone()));
            seen.insert(bytes, objects.len() - 1);
            objects.len() - 1
        }
    }
}

fn encode(value: &Plist) -> Vec<u8> {
    let mut out = Vec::new();
    match value {
        Plist::Bool(b) => out.push(if *b { 0x09 } else { 0x08 }),
        Plist::Int(n) => match u64::try_from(*n) {
            Ok(n) if n <= 0xffff_ffff => {
                let size = size_of_uint(n);
                out.push(0x10 | u8::try_from(size.trailing_zeros()).expect("at most 3"));
                uint(n, size, &mut out);
            }
            _ => {
                out.push(0x13);
                out.extend_from_slice(&n.to_be_bytes());
            }
        },
        Plist::Real(r) => {
            out.push(0x23);
            out.extend_from_slice(&r.to_be_bytes());
        }
        Plist::Real32(r) => {
            out.push(0x22);
            out.extend_from_slice(&r.to_be_bytes());
        }
        Plist::String(text) if text.is_ascii() => {
            marker(0x5, text.len(), &mut out);
            out.extend_from_slice(text.as_bytes());
        }
        Plist::String(text) => {
            let units: Vec<u16> = text.encode_utf16().collect();
            marker(0x6, units.len(), &mut out);
            for unit in units {
                out.extend_from_slice(&unit.to_be_bytes());
            }
        }
        Plist::Data(bytes) => {
            marker(0x4, bytes.len(), &mut out);
            out.extend_from_slice(bytes);
        }
        Plist::Uid(n) => {
            let size = size_of_uint(*n);
            out.push(0x80 | u8::try_from(size - 1).expect("at most 7"));
            uint(*n, size, &mut out);
        }
        Plist::Array(_) | Plist::Dict(_) => unreachable!("containers are flattened"),
    }
    out
}

/// An object's marker: its kind, and its length in the low four bits, or
/// after it as an integer when it doesn't fit.
fn marker(kind: u8, count: usize, out: &mut Vec<u8>) {
    if count < 15 {
        out.push(kind << 4 | u8::try_from(count).expect("below 15"));
    } else {
        out.push(kind << 4 | 0xf);
        out.extend(encode(&Plist::Int(
            i64::try_from(count).expect("a length fits"),
        )));
    }
}

/// How many bytes, 1, 2, 4 or 8, `n` takes unsigned.
fn size_of_uint(n: u64) -> usize {
    match n {
        0..=0xff => 1,
        0x100..=0xffff => 2,
        0x1_0000..=0xffff_ffff => 4,
        _ => 8,
    }
}

fn uint(n: u64, size: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(&n.to_be_bytes()[8 - size..]);
}

#[cfg(test)]
pub(crate) mod read {
    //! A reader for the binary property lists this module writes, and
    //! Xcode's momc, for the tests.

    use super::Plist;

    pub fn parse(data: &[u8]) -> Plist {
        assert!(data.starts_with(b"bplist00"), "not a binary plist");
        let trailer = &data[data.len() - 32..];
        let offset_size = usize::from(trailer[6]);
        let ref_size = usize::from(trailer[7]);
        let count = be(&trailer[8..16]);
        let top = be(&trailer[16..24]);
        let table = be(&trailer[24..32]);
        let offsets: Vec<usize> = (0..count)
            .map(|i| be(&data[table + i * offset_size..table + (i + 1) * offset_size]))
            .collect();
        object(data, &offsets, ref_size, top)
    }

    fn be(bytes: &[u8]) -> usize {
        bytes.iter().fold(0usize, |n, &b| (n << 8) | usize::from(b))
    }

    fn length(data: &[u8], at: usize) -> (usize, usize) {
        let low = usize::from(data[at] & 0xf);
        if low < 15 {
            return (low, at + 1);
        }
        let size = 1 << (data[at + 1] & 0xf);
        (be(&data[at + 2..at + 2 + size]), at + 2 + size)
    }

    fn object(data: &[u8], offsets: &[usize], ref_size: usize, index: usize) -> Plist {
        let at = offsets[index];
        let marker = data[at];
        let refs = |start: usize, n: usize| -> Vec<usize> {
            (0..n)
                .map(|i| be(&data[start + i * ref_size..start + (i + 1) * ref_size]))
                .collect()
        };
        match marker >> 4 {
            0x0 => Plist::Bool(marker == 0x09),
            0x1 => {
                let size = 1 << (marker & 0xf);
                let bytes = &data[at + 1..at + 1 + size];
                if size == 8 {
                    Plist::Int(i64::from_be_bytes(bytes.try_into().unwrap()))
                } else {
                    Plist::Int(i64::try_from(be(bytes)).unwrap())
                }
            }
            0x2 if marker & 0xf == 2 => {
                Plist::Real32(f32::from_be_bytes(data[at + 1..at + 5].try_into().unwrap()))
            }
            0x2 | 0x3 => Plist::Real(f64::from_be_bytes(data[at + 1..at + 9].try_into().unwrap())),
            0x4 => {
                let (n, start) = length(data, at);
                Plist::Data(data[start..start + n].to_vec())
            }
            0x5 => {
                let (n, start) = length(data, at);
                Plist::String(String::from_utf8(data[start..start + n].to_vec()).unwrap())
            }
            0x6 => {
                let (n, start) = length(data, at);
                let units: Vec<u16> = (0..n)
                    .map(|i| u16::from_be_bytes([data[start + 2 * i], data[start + 2 * i + 1]]))
                    .collect();
                Plist::String(String::from_utf16(&units).unwrap())
            }
            0x8 => Plist::Uid(be(&data[at + 1..at + 2 + usize::from(marker & 0xf)]) as u64),
            0xa | 0xc => {
                let (n, start) = length(data, at);
                Plist::Array(
                    refs(start, n)
                        .into_iter()
                        .map(|i| object(data, offsets, ref_size, i))
                        .collect(),
                )
            }
            0xd => {
                let (n, start) = length(data, at);
                let keys = refs(start, n);
                let values = refs(start + n * ref_size, n);
                Plist::Dict(
                    keys.into_iter()
                        .zip(values)
                        .map(|(k, v)| {
                            let Plist::String(key) = object(data, offsets, ref_size, k) else {
                                panic!("a dictionary key that isn't a string");
                            };
                            (key, object(data, offsets, ref_size, v))
                        })
                        .collect(),
                )
            }
            kind => panic!("unexpected object kind {kind:#x}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plist_reads_back_as_it_was_written() {
        let value = Plist::Dict(vec![
            ("a".to_string(), Plist::Int(3)),
            ("big".to_string(), Plist::Int(9_000_000_000)),
            ("neg".to_string(), Plist::Int(-7)),
            ("f".to_string(), Plist::Real32(0.1)),
            ("d".to_string(), Plist::Real(2.5)),
            ("yes".to_string(), Plist::Bool(true)),
            ("uid".to_string(), Plist::Uid(300)),
            (
                "text".to_string(),
                Plist::String("é and a long enough string".to_string()),
            ),
            ("data".to_string(), Plist::Data(vec![1, 2, 3])),
            (
                "list".to_string(),
                Plist::Array((0..20).map(|_| Plist::Int(3)).collect()),
            ),
        ]);
        assert_eq!(read::parse(&write(&value)), value);
    }

    #[test]
    fn an_archive_puts_its_objects_after_null() {
        let mut archiver = Archiver::new();
        let name = archiver.string("Item");
        assert_eq!(archiver.string("Item"), name);
        let root = archiver.object(None, &["Thing", "NSObject"], vec![("name", name)]);
        let Plist::Dict(top) = read::parse(&archiver.finish(root)) else {
            panic!("not a dictionary");
        };
        let objects = &top.iter().find(|(k, _)| k == "$objects").unwrap().1;
        let Plist::Array(objects) = objects else {
            panic!("no objects");
        };
        assert_eq!(objects[0], Plist::String("$null".to_string()));
        assert_eq!(objects[1], Plist::String("Item".to_string()));
        assert_eq!(objects.len(), 4);
    }
}
