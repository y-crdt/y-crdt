pub mod array;
pub mod map;
pub mod text;

use crate::*;
pub use map::Map;
pub use text::Text;

use crate::block::BlockPtr;
use lib0::any::Any;
use std::cell::Cell;
use std::collections::HashMap;
use std::hash::Hasher;
use std::rc::Rc;

pub struct Array {
    ptr: types::TypePtr,
}

pub struct XmlElement {
    ptr: types::TypePtr,
}

pub struct XmlFragment {
    ptr: types::TypePtr,
}

pub struct XmlHook {
    ptr: types::TypePtr,
}

pub struct XmlText {
    ptr: types::TypePtr,
}

pub enum SharedType {
    Text(Text),
    Array(Array),
    Map(Map),
    XmlElement(XmlElement),
    XmlFragment(XmlFragment),
    XmlHook(XmlHook),
    XmlText(XmlText),
}

pub type TypeRefs = u8;

pub const TYPE_REFS_ARRAY: TypeRefs = 0;
pub const TYPE_REFS_MAP: TypeRefs = 1;
pub const TYPE_REFS_TEXT: TypeRefs = 2;
pub const TYPE_REFS_XML_ELEMENT: TypeRefs = 3;
pub const TYPE_REFS_XML_FRAGMENT: TypeRefs = 4;
pub const TYPE_REFS_XML_HOOK: TypeRefs = 5;
pub const TYPE_REFS_XML_TEXT: TypeRefs = 6;

/// Placeholder for non-specialized AbstractType.
pub const TYPE_REFS_UNDEFINED: TypeRefs = 7;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Inner {
    pub start: Cell<Option<BlockPtr>>,
    pub map: HashMap<String, BlockPtr>,
    pub ptr: TypePtr,
    pub name: Option<String>,
    pub len: u32,
    type_ref: TypeRefs,
}

impl Inner {
    pub fn new(ptr: TypePtr, name: Option<String>, type_ref: TypeRefs) -> Self {
        Self {
            start: Cell::from(None),
            map: HashMap::default(),
            len: 0,
            ptr,
            name,
            type_ref,
        }
    }

    pub fn type_ref(&self) -> TypeRefs {
        self.type_ref & 0b1111
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn to_json(&self, txn: &Transaction) -> Any {
        match self.type_ref() {
            TYPE_REFS_ARRAY => todo!(),
            TYPE_REFS_MAP => Map::to_json_inner(self, txn),
            TYPE_REFS_TEXT => Any::String(Text::to_string_inner(self, txn)),
            TYPE_REFS_XML_ELEMENT => todo!(),
            TYPE_REFS_XML_FRAGMENT => todo!(),
            TYPE_REFS_XML_HOOK => todo!(),
            TYPE_REFS_XML_TEXT => todo!(),
            other => panic!("Defect: unknown type reference: {}", other),
        }
    }
}

impl std::fmt::Display for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.type_ref() {
            TYPE_REFS_ARRAY => write!(f, "YArray(start: {})", self.start.get().unwrap()),
            TYPE_REFS_MAP => {
                write!(f, "YMap(")?;
                let mut iter = self.map.iter();
                if let Some((k, v)) = iter.next() {
                    write!(f, "'{}': {}", k, v)?;
                }
                while let Some((k, v)) = iter.next() {
                    write!(f, ", '{}': {}", k, v)?;
                }
                write!(f, ")")
            }
            TYPE_REFS_TEXT => write!(f, "YText(start: {})", self.start.get().unwrap()),
            TYPE_REFS_XML_ELEMENT => todo!(),
            TYPE_REFS_XML_FRAGMENT => todo!(),
            TYPE_REFS_XML_HOOK => todo!(),
            TYPE_REFS_XML_TEXT => todo!(),
            other => {
                write!(f, "UnknownRef")?;
                if let Some(start) = self.start.get() {
                    write!(f, "(start: {})", start)?;
                }
                if !self.map.is_empty() {
                    write!(f, " {{")?;
                    let mut iter = self.map.iter();
                    if let Some((k, v)) = iter.next() {
                        write!(f, "'{}': {}", k, v)?;
                    }
                    while let Some((k, v)) = iter.next() {
                        write!(f, ", '{}': {}", k, v)?;
                    }
                    write!(f, "}}")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypePtr {
    Id(block::BlockPtr),
    Named(Rc<String>),
}

#[derive(Default)]
pub(crate) struct XorHasher(u64);

impl Hasher for XorHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut i = 0;
        let mut buf = [0u8; 8];
        while i <= bytes.len() - 8 {
            buf.copy_from_slice(&bytes[i..i + 8]);
            self.0 ^= u64::from_ne_bytes(buf);
            i += 8;
        }
        while i < bytes.len() {
            self.0 ^= bytes[i] as u64;
            i += 1;
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.0 ^= value as u64;
    }

    fn write_u64(&mut self, value: u64) {
        self.0 ^= value;
    }

    fn write_usize(&mut self, value: usize) {
        self.0 ^= value as u64;
    }
}
