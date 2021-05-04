pub mod map;
pub mod text;

use crate::*;
pub use map::Map;
pub use text::Text;

use std::convert::TryFrom;
use std::convert::Into;
use std::hash::Hasher;

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
  XmlText(XmlText)
}
#[derive(std::cmp::PartialEq)]
pub enum TypeRefs {
  YArray,
  YMap,
  YText,
  YXmlElement,
  YXmlFragment,
  YXmlHook,
  YXmlText,
}

impl TryFrom<u8> for TypeRefs {
  type Error = &'static str;

  fn try_from(value: u8) -> Result<Self, Self::Error> {
    match value {
      0 => Ok(TypeRefs::YArray),
      1 => Ok(TypeRefs::YMap),
      2 => Ok(TypeRefs::YText),
      3 => Ok(TypeRefs::YXmlElement),
      4 => Ok(TypeRefs::YXmlFragment),
      5 => Ok(TypeRefs::YXmlHook),
      6 => Ok(TypeRefs::YXmlText),
      _ => Err("Unknown shared type"),
    }
  }
}

impl Into<u8> for TypeRefs {
  fn into(self) -> u8 {
    match self {
        TypeRefs::YArray => { 0 }
        TypeRefs::YMap => { 1 }
        TypeRefs::YText => { 2 }
        TypeRefs::YXmlElement => { 3 }
        TypeRefs::YXmlFragment => { 4 }
        TypeRefs::YXmlHook => { 5 }
        TypeRefs::YXmlText => { 6 }
    }
  } 
}

pub struct Inner {
    pub start: Cell<Option<block::BlockPtr>>,
    pub ptr: TypePtr,
    pub name: Option<String>,
    pub type_ref: TypeRefs,
}

impl Inner {
  pub fn new (ptr: TypePtr, name: Option<String>, type_ref: TypeRefs) -> Self {
    Self {
      start: Cell::from(None),
      ptr,
      name,
      type_ref
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypePtr {
    NamedRef(u32),
    Id(block::BlockPtr),
    Named(String),
}

#[derive(Default)]
pub(crate) struct XorHasher(u64);

impl Hasher for XorHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut i = 0;
        let mut buf = [0u8;8];
        while i <= bytes.len() - 8 {
            buf.copy_from_slice(&bytes[i..i+8]);
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