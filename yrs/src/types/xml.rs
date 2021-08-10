use crate::block::ItemContent;
use crate::types::{Inner, Map, Text};
use crate::Transaction;
use std::cell::RefCell;
use std::rc::Rc;

pub struct XmlElement(XmlFragment);

impl XmlElement {
    pub fn new(inner: Rc<RefCell<Inner>>) -> Self {
        XmlElement(XmlFragment::new(inner))
    }

    pub fn to_string(&self, txn: &Transaction) -> String {
        todo!()
    }

    pub fn remove_attribute(&self, txn: &mut Transaction, attr_name: &str) {
        todo!()
    }

    pub fn insert_attribute(&self, txn: &mut Transaction, attr_name: String, attr_value: String) {
        todo!()
    }

    pub fn get_attribute(&self, txn: &Transaction) -> Option<&str> {
        todo!()
    }

    pub fn attributes<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> Attributes<'b, 'txn> {
        todo!()
    }

    pub fn next_sybling(&self, txn: &Transaction) -> Option<XmlElement> {
        todo!()
    }

    pub fn prev_sybling(&self, txn: &Transaction) -> Option<XmlElement> {
        todo!()
    }

    pub fn first(&self, txn: &Transaction) -> Option<XmlElement> {
        self.0.first(txn)
    }

    pub fn len(&self, txn: &Transaction) -> usize {
        self.0.len(txn)
    }

    pub fn iter<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> Iter<'b, 'txn> {
        self.0.iter(txn)
    }

    pub fn select(&self, txn: &Transaction, query: &str) -> Iter {
        self.0.select(txn, query)
    }

    pub fn insert(&self, txn: &mut Transaction, content: &[XmlElement]) {
        self.0.insert(txn, content)
    }

    pub fn insert_after(&self, txn: &mut Transaction, ref_: XmlElement, content: &[XmlElement]) {
        self.0.insert_after(txn, ref_, content)
    }

    pub fn remove(&self, txn: &mut Transaction, index: u32, len: u32) {
        self.0.remove(txn, index, len)
    }

    pub fn push_back(&self, txn: &mut Transaction, content: &[XmlElement]) {
        self.0.push_back(txn, content)
    }

    pub fn push_front(&self, txn: &mut Transaction, content: &[XmlElement]) {
        self.0.push_front(txn, content)
    }

    pub fn get(&self, txn: &Transaction, index: u32) -> Option<XmlElement> {
        self.0.get(txn, index)
    }
}

pub struct Attributes<'a, 'txn> {}

impl<'a, 'txn> Attributes<'a, 'txn> {}

impl<'a, 'txn> Iterator for Attributes<'a, 'txn> {
    type Item = (&'a str, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

impl Into<XmlElement> for XmlFragment {
    fn into(self) -> XmlElement {
        XmlElement(self)
    }
}

pub struct XmlFragment(Rc<RefCell<Inner>>);

impl XmlFragment {
    pub fn new(inner: Rc<RefCell<Inner>>) -> Self {
        XmlFragment(inner)
    }

    pub fn first(&self, txn: &Transaction) -> Option<XmlElement> {
        todo!()
    }

    pub fn len(&self, txn: &Transaction) -> usize {
        todo!()
    }

    pub fn iter<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> Iter<'b, 'txn> {
        todo!()
    }

    pub fn select(&self, txn: &Transaction, query: &str) -> Iter {
        todo!()
    }

    pub fn to_string(&self, txn: &Transaction) -> String {
        todo!()
    }

    pub fn insert(&self, txn: &mut Transaction, content: &[XmlElement]) {
        todo!()
    }

    pub fn insert_after(&self, txn: &mut Transaction, ref_: XmlElement, content: &[XmlElement]) {
        todo!()
    }

    pub fn remove(&self, txn: &mut Transaction, index: u32, len: u32) {
        todo!()
    }

    pub fn push_back(&self, txn: &mut Transaction, content: &[XmlElement]) {
        todo!()
    }

    pub fn push_front(&self, txn: &mut Transaction, content: &[XmlElement]) {
        todo!()
    }

    pub fn get(&self, txn: &Transaction, index: u32) -> Option<XmlElement> {
        todo!()
    }
}

pub struct Iter<'a, 'txn> {}

impl<'a, 'txn> Iter<'a, 'txn> {}

impl<'a, 'txn> Iterator for Iter<'a, 'txn> {
    type Item = ();

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

pub struct XmlHook(Map);

impl XmlHook {}

impl Into<XmlHook> for Map {
    fn into(self) -> XmlHook {
        XmlHook(self)
    }
}

pub struct XmlText(Text);

impl XmlText {}

impl Into<XmlText> for Text {
    fn into(self) -> XmlText {
        XmlText(self)
    }
}
