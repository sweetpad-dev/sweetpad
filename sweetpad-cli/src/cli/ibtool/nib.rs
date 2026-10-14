//! NIBArchive, the format of a compiled iOS nib, which UIKit's
//! `UINibDecoder` reads: a header, then four tables of objects, keys,
//! values and class names. An object is a class and a run of values; a
//! value is a key and a typed scalar, data, nil or a reference to another
//! object.
//!
//! Xcode's ibtool archives real UIKit objects, so a nib holds what each
//! class's `encodeWithCoder:` writes. [`Graph`] collects those objects;
//! [`Graph::write`] lays them out in the order UIKit's encoder does, the
//! order a depth-first walk from the root first reaches each.

use std::collections::HashMap;

/// An object in a [`Graph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct Id(usize);

/// A value an object archives under a key.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Value {
    Int(i64),
    Bool(bool),
    Float(f32),
    Double(f64),
    Data(Vec<u8>),
    Nil,
    Object(Id),
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Bool(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Int(value)
    }
}

impl From<f32> for Value {
    fn from(value: f32) -> Self {
        Value::Float(value)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Value::Double(value)
    }
}

impl From<Id> for Value {
    fn from(value: Id) -> Self {
        Value::Object(value)
    }
}

impl From<Option<Id>> for Value {
    fn from(value: Option<Id>) -> Self {
        value.map_or(Value::Nil, Value::Object)
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Object {
    class: &'static str,
    values: Vec<(&'static str, Value)>,
}

/// Where the archive lets a decoder that lacks a class use another: a
/// UIKit class's AppKit counterpart.
const FALLBACKS: &[(&str, &str)] = &[("UIColor", "NSColor"), ("UIFont", "NSFont")];

/// The key an array's or a dictionary's elements are archived under.
pub(super) const ELEMENT: &str = "UINibEncoderEmptyKey";

/// The objects of a nib.
///
/// UIKit's encoder writes an object once however often it's referred to,
/// and the values Foundation and UIKit share, like equal strings, colors
/// and fonts, are one object to it. [`Graph::shared`] keeps one of each of
/// those; every other object is its own.
#[derive(Default)]
pub(super) struct Graph {
    objects: Vec<Object>,
    shared: HashMap<String, Id>,
}

impl Graph {
    /// A new object of `class`, with no values yet.
    pub fn object(&mut self, class: &'static str) -> Id {
        self.objects.push(Object {
            class,
            values: Vec::new(),
        });
        Id(self.objects.len() - 1)
    }

    /// Archives `value` under `key` in `id`, after the values it has.
    pub fn set(&mut self, id: Id, key: &'static str, value: impl Into<Value>) {
        self.objects[id.0].values.push((key, value.into()));
    }

    /// An object of `class` with `values`, the same one for every call
    /// with the same class and values.
    pub fn shared(&mut self, class: &'static str, values: Vec<(&'static str, Value)>) -> Id {
        let key = format!("{class}{values:?}");
        if let Some(&id) = self.shared.get(&key) {
            return id;
        }
        self.objects.push(Object { class, values });
        let id = Id(self.objects.len() - 1);
        self.shared.insert(key, id);
        id
    }

    /// The `NSString` `text`.
    pub fn string(&mut self, text: &str) -> Id {
        self.shared(
            "NSString",
            vec![("NS.bytes", Value::Data(text.as_bytes().to_vec()))],
        )
    }

    /// An `NSNumber` holding a double.
    pub fn number(&mut self, value: f64) -> Id {
        self.shared("NSNumber", vec![("NS.dblval", Value::Double(value))])
    }

    /// An array of `class`, `NSArray` or `NSMutableArray`. Empty
    /// `NSArray`s are all one object, as Foundation's are.
    pub fn array(&mut self, class: &'static str, items: &[Id]) -> Id {
        let mut values = vec![("NSInlinedValue", Value::Bool(true))];
        values.extend(items.iter().map(|&item| (ELEMENT, Value::Object(item))));
        if class == "NSArray" && items.is_empty() {
            return self.shared(class, values);
        }
        self.objects.push(Object { class, values });
        Id(self.objects.len() - 1)
    }

    /// An `NSDictionary`, its keys and values alternating.
    pub fn dictionary(&mut self, class: &'static str, entries: &[(Id, Id)]) -> Id {
        let mut values = vec![("NSInlinedValue", Value::Bool(true))];
        for &(key, value) in entries {
            values.push((ELEMENT, Value::Object(key)));
            values.push((ELEMENT, Value::Object(value)));
        }
        self.shared(class, values)
    }

    /// The archive of everything `root` reaches.
    pub fn write(&self, root: Id) -> Vec<u8> {
        let mut order = Vec::new();
        let mut index = HashMap::new();
        self.walk(root, &mut order, &mut index);

        let mut classes: Vec<&str> = Vec::new();
        let mut keys: Vec<&str> = Vec::new();
        let mut class_index = HashMap::new();
        let mut key_index = HashMap::new();
        for &id in &order {
            let object = &self.objects[id.0];
            if !class_index.contains_key(object.class) {
                class_index.insert(object.class, classes.len());
                classes.push(object.class);
            }
            for (key, _) in &object.values {
                if !key_index.contains_key(key) {
                    key_index.insert(*key, keys.len());
                    keys.push(key);
                }
            }
        }
        let primary = classes.len();
        for &(class, fallback) in FALLBACKS {
            if class_index.contains_key(class) && !class_index.contains_key(fallback) {
                class_index.insert(fallback, classes.len());
                classes.push(fallback);
            }
        }

        let mut objects = Vec::new();
        let mut values = Vec::new();
        let mut value_count = 0;
        for &id in &order {
            let object = &self.objects[id.0];
            varint(class_index[object.class], &mut objects);
            varint(value_count, &mut objects);
            varint(object.values.len(), &mut objects);
            for (key, value) in &object.values {
                varint(key_index[key], &mut values);
                encode(value, &index, &mut values);
            }
            value_count += object.values.len();
        }
        let mut key_table = Vec::new();
        for key in &keys {
            varint(key.len(), &mut key_table);
            key_table.extend_from_slice(key.as_bytes());
        }
        let mut class_table = Vec::new();
        for (n, class) in classes.iter().enumerate() {
            let fallbacks: Vec<usize> = if n < primary {
                FALLBACKS
                    .iter()
                    .filter(|(from, _)| from == class)
                    .map(|(_, to)| class_index[to])
                    .collect()
            } else {
                Vec::new()
            };
            varint(class.len() + 1, &mut class_table);
            varint(fallbacks.len(), &mut class_table);
            for fallback in fallbacks {
                class_table.extend_from_slice(&u32::try_from(fallback).unwrap_or(0).to_le_bytes());
            }
            class_table.extend_from_slice(class.as_bytes());
            class_table.push(0);
        }

        let header = 10 + 4 * 10;
        let objects_at = header;
        let keys_at = objects_at + objects.len();
        let values_at = keys_at + key_table.len();
        let classes_at = values_at + values.len();
        let mut out = b"NIBArchive".to_vec();
        for n in [
            1,
            10,
            order.len(),
            objects_at,
            keys.len(),
            keys_at,
            value_count,
            values_at,
            classes.len(),
            classes_at,
        ] {
            out.extend_from_slice(&u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes());
        }
        out.extend(objects);
        out.extend(key_table);
        out.extend(values);
        out.extend(class_table);
        out
    }

    fn walk(&self, id: Id, order: &mut Vec<Id>, index: &mut HashMap<Id, usize>) {
        if index.contains_key(&id) {
            return;
        }
        index.insert(id, order.len());
        order.push(id);
        for (_, value) in &self.objects[id.0].values {
            if let Value::Object(child) = value {
                self.walk(*child, order, index);
            }
        }
    }
}

/// Data for a run of doubles, like a rect or a point, as UIKit writes them:
/// a type byte, then each little-endian.
pub(super) fn doubles(values: &[f64]) -> Value {
    let mut data = vec![7];
    for value in values {
        data.extend_from_slice(&value.to_le_bytes());
    }
    Value::Data(data)
}

fn encode(value: &Value, index: &HashMap<Id, usize>, out: &mut Vec<u8>) {
    match value {
        Value::Int(n) => {
            if let Ok(n) = i8::try_from(*n) {
                out.push(0);
                out.extend_from_slice(&n.to_le_bytes());
            } else if let Ok(n) = i16::try_from(*n) {
                out.push(1);
                out.extend_from_slice(&n.to_le_bytes());
            } else if let Ok(n) = i32::try_from(*n) {
                out.push(2);
                out.extend_from_slice(&n.to_le_bytes());
            } else {
                out.push(3);
                out.extend_from_slice(&n.to_le_bytes());
            }
        }
        Value::Bool(false) => out.push(4),
        Value::Bool(true) => out.push(5),
        Value::Float(x) => {
            out.push(6);
            out.extend_from_slice(&x.to_le_bytes());
        }
        Value::Double(x) => {
            out.push(7);
            out.extend_from_slice(&x.to_le_bytes());
        }
        Value::Data(bytes) => {
            out.push(8);
            varint(bytes.len(), out);
            out.extend_from_slice(bytes);
        }
        Value::Nil => out.push(9),
        Value::Object(id) => {
            out.push(10);
            out.extend_from_slice(&u32::try_from(index[id]).unwrap_or(0).to_le_bytes());
        }
    }
}

/// NIBArchive's variable-length integer: seven bits a byte, low first, the
/// high bit set on the last.
fn varint(mut n: usize, out: &mut Vec<u8>) {
    loop {
        let low = u8::try_from(n & 0x7f).unwrap_or(0);
        n >>= 7;
        if n == 0 {
            out.push(low | 0x80);
            return;
        }
        out.push(low);
    }
}

#[cfg(test)]
pub(super) mod read {
    //! A reader for NIBArchive files, this module's and Xcode's, for the
    //! tests.

    /// A decoded value; an object reference is its index in the archive.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Read {
        Int(i64),
        Bool(bool),
        Float(f32),
        Double(f64),
        Data(Vec<u8>),
        Nil,
        Object(usize),
    }

    #[derive(Debug, Clone)]
    pub struct Object {
        pub class: String,
        pub values: Vec<(String, Read)>,
    }

    fn varint(data: &[u8], at: &mut usize) -> usize {
        let (mut n, mut shift) = (0, 0);
        loop {
            let byte = data[*at];
            *at += 1;
            n |= usize::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 != 0 {
                return n;
            }
        }
    }

    fn u32_at(data: &[u8], at: usize) -> usize {
        u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize
    }

    pub fn parse(data: &[u8]) -> Vec<Object> {
        assert_eq!(&data[..10], b"NIBArchive", "not a NIBArchive");
        let field = |n: usize| u32_at(data, 10 + 4 * n);
        let (object_count, objects_at) = (field(2), field(3));
        let (key_count, keys_at) = (field(4), field(5));
        let (value_count, values_at) = (field(6), field(7));
        let (class_count, classes_at) = (field(8), field(9));

        let mut at = classes_at;
        let mut classes = Vec::new();
        for _ in 0..class_count {
            let length = varint(data, &mut at);
            let extra = varint(data, &mut at);
            at += 4 * extra;
            let name = &data[at..at + length];
            classes.push(
                String::from_utf8_lossy(name)
                    .trim_end_matches('\0')
                    .to_string(),
            );
            at += length;
        }
        let mut at = keys_at;
        let mut keys = Vec::new();
        for _ in 0..key_count {
            let length = varint(data, &mut at);
            keys.push(String::from_utf8_lossy(&data[at..at + length]).to_string());
            at += length;
        }
        let mut at = values_at;
        let mut values = Vec::new();
        for _ in 0..value_count {
            let key = keys[varint(data, &mut at)].clone();
            let kind = data[at];
            at += 1;
            let mut take = |n: usize| {
                let bytes = &data[at..at + n];
                at += n;
                bytes
            };
            let value = match kind {
                0 => Read::Int(i64::from(i8::from_le_bytes(take(1).try_into().unwrap()))),
                1 => Read::Int(i64::from(i16::from_le_bytes(take(2).try_into().unwrap()))),
                2 => Read::Int(i64::from(i32::from_le_bytes(take(4).try_into().unwrap()))),
                3 => Read::Int(i64::from_le_bytes(take(8).try_into().unwrap())),
                4 => Read::Bool(false),
                5 => Read::Bool(true),
                6 => Read::Float(f32::from_le_bytes(take(4).try_into().unwrap())),
                7 => Read::Double(f64::from_le_bytes(take(8).try_into().unwrap())),
                8 => {
                    let length = varint(data, &mut at);
                    let bytes = data[at..at + length].to_vec();
                    at += length;
                    Read::Data(bytes)
                }
                9 => Read::Nil,
                10 => Read::Object(u32::from_le_bytes(take(4).try_into().unwrap()) as usize),
                other => panic!("value type {other}"),
            };
            values.push((key, value));
        }
        let mut at = objects_at;
        let mut objects = Vec::new();
        for _ in 0..object_count {
            let class = classes[varint(data, &mut at)].clone();
            let first = varint(data, &mut at);
            let count = varint(data, &mut at);
            objects.push(Object {
                class,
                values: values[first..first + count].to_vec(),
            });
        }
        objects
    }
}
