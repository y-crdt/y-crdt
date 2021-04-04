pub mod map;
pub mod text;

use crate::*;
pub use map::Map;
pub use text::Text;

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

#[derive(Clone)]
pub struct Inner {
    pub start: Cell<Option<block::BlockPtr>>,
    pub ptr: TypePtr,
}

#[derive(Clone)]
pub enum TypePtr {
    NamedRef(u32),
    Id(block::BlockPtr),
    Named(String),
}
