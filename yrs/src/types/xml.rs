use crate::block::ItemContent;
use crate::types::{Entries, Inner, Map, Text};
use crate::Transaction;
use std::borrow::Borrow;
use std::cell::{Ref, RefCell};
use std::fmt::Write;
use std::rc::Rc;

pub enum Xml {
    Element(XmlElement),
    Text(XmlText),
    Hook(XmlHook),
}

impl Into<Xml> for XmlElement {
    fn into(self) -> Xml {
        Xml::Element(self)
    }
}

impl Into<Xml> for XmlText {
    fn into(self) -> Xml {
        Xml::Text(self)
    }
}

impl Into<Xml> for XmlHook {
    fn into(self) -> Xml {
        Xml::Hook(self)
    }
}

pub struct XmlElement(XmlFragment);

impl XmlElement {
    pub fn new(inner: Rc<RefCell<Inner>>) -> Self {
        XmlElement(XmlFragment::new(inner))
    }

    fn inner(&self) -> Ref<Inner> {
        let fragment = &self.0;
        (*fragment.0).borrow()
    }

    pub fn to_string(&self, txn: &Transaction) -> String {
        let mut s = String::new();
        {
            let inner = self.inner();
            let node_name = inner.name.as_ref().unwrap();
            write!(&mut s, "<{}", node_name);
        }
        for (k, v) in self.attributes(txn) {
            write!(&mut s, " \"{}\"=\"{}\"", k, v);
        }
        write!(&mut s, ">");
        write!(&mut s, "{}", self.0.to_string(txn)); //TODO: optimize to_string to use formatter
        {
            let inner = self.inner();
            let node_name = inner.name.as_ref().unwrap();
            write!(&mut s, "</{}>", node_name);
        }
        s
    }

    pub fn remove_attribute(&self, txn: &mut Transaction, attr_name: &str) {
        self.inner().remove(txn, attr_name);
    }

    pub fn insert_attribute<K: ToString, V: ToString>(
        &self,
        txn: &mut Transaction,
        attr_name: K,
        attr_value: V,
    ) {
        self.inner()
            .insert(txn, attr_name.to_string(), attr_value.to_string())
    }

    pub fn get_attribute(&self, txn: &Transaction, attr_name: &str) -> Option<String> {
        let inner: Ref<_> = self.inner();
        let value = inner.get(txn, attr_name)?;
        Some(value.to_string())
    }

    pub fn attributes<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> Attributes<'b, 'txn> {
        let inner = self.inner();
        let blocks = inner.entries(txn);
        Attributes(blocks)
    }

    pub fn next_sibling(&self, txn: &Transaction) -> Option<XmlElement> {
        let inner = self.inner();
        let item = txn.store.blocks.get_item(&inner.start.get().unwrap())?;
        let mut next = item
            .right
            .as_ref()
            .and_then(|p| txn.store.blocks.get_item(p));
        while let Some(item) = next {
            if !item.is_deleted() {
                break;
            }
            next = item
                .right
                .as_ref()
                .and_then(|p| txn.store.blocks.get_item(p));
        }

        let item = next?;
        if let ItemContent::Type(inner) = &item.content {
            Some(XmlElement(XmlFragment(inner.clone())))
        } else {
            None
        }
    }

    pub fn prev_sibling(&self, txn: &Transaction) -> Option<XmlElement> {
        let inner = self.inner();
        let item = txn.store.blocks.get_item(&inner.start.get().unwrap())?;
        let mut next = item
            .left
            .as_ref()
            .and_then(|p| txn.store.blocks.get_item(p));
        while let Some(item) = next {
            if !item.is_deleted() {
                break;
            }
            next = item
                .left
                .as_ref()
                .and_then(|p| txn.store.blocks.get_item(p));
        }

        let item = next?;
        if let ItemContent::Type(inner) = &item.content {
            Some(XmlElement(XmlFragment(inner.clone())))
        } else {
            None
        }
    }

    pub fn first(&self, txn: &Transaction) -> Option<XmlElement> {
        self.0.first(txn)
    }

    pub fn len(&self, txn: &Transaction) -> u32 {
        self.0.len(txn)
    }

    pub fn iter<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> TreeWalker<'b, 'txn> {
        self.0.iter(txn)
    }

    pub fn select(&self, txn: &Transaction, query: &str) -> TreeWalker {
        self.0.select(txn, query)
    }

    pub fn insert(&self, txn: &mut Transaction, index: u32, content: &[XmlElement]) {
        self.0.insert(txn, index, content)
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

pub struct Attributes<'a, 'txn>(Entries<'a, 'txn>);

impl<'a, 'txn> Iterator for Attributes<'a, 'txn> {
    type Item = (&'a str, String);

    fn next(&mut self) -> Option<Self::Item> {
        let (key, block) = self.0.next()?;
        let value = block
            .content
            .get_content_last(self.0.txn)
            .map(|v| v.to_string())
            .unwrap_or(String::default());

        Some((key.as_str(), value))
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

    fn inner(&self) -> Ref<Inner> {
        (*self.0).borrow()
    }

    pub fn first(&self, txn: &Transaction) -> Option<XmlElement> {
        let inner = self.inner();
        let first = inner.first(txn)?;
        //first.content.get_content_first()
        todo!()
    }

    pub fn len(&self, txn: &Transaction) -> u32 {
        self.inner().len()
    }

    pub fn iter<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> TreeWalker<'b, 'txn> {
        todo!()
    }

    pub fn select(&self, txn: &Transaction, query: &str) -> TreeWalker {
        todo!()
    }

    pub fn to_string(&self, txn: &Transaction) -> String {
        let mut s = String::new();
        let inner = self.inner();
        for i in inner.iter(txn) {
            for content in i.content.get_content(txn) {
                write!(&mut s, "{}", content);
            }
        }
        s
    }

    pub fn insert(&self, txn: &mut Transaction, index: u32, content: &[XmlElement]) {
        let inner = self.inner();
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

pub struct TreeWalker<'a, 'txn>(Entries<'a, 'txn>);

impl<'a, 'txn> TreeWalker<'a, 'txn> {}

impl<'a, 'txn> Iterator for TreeWalker<'a, 'txn> {
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

impl XmlText {
    pub fn to_string(&self, txn: &Transaction) -> String {
        todo!()
    }

    pub fn next_sibling(&self, txn: &Transaction) -> Option<XmlElement> {
        todo!()
    }

    pub fn prev_sibling(&self, txn: &Transaction) -> Option<XmlElement> {
        todo!()
    }
}

impl Into<XmlText> for Text {
    fn into(self) -> XmlText {
        XmlText(self)
    }
}

#[cfg(test)]
mod test {
    use crate::Doc;

    #[test]
    fn insert() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let xml1 = t1.get_xml_element("xml");
        xml1.insert_attribute(&mut t1, "height", "10");
        assert_eq!(xml1.get_attribute(&t1, "height"), Some("10".to_string()));

        let d2 = Doc::with_client_id(1);
        let mut t2 = d2.transact();
        let xml2 = t2.get_xml_element("xml");
        d2.apply_update(&mut t2, d1.encode_state_as_update(&t1).as_slice());
        assert_eq!(xml2.get_attribute(&t2, "height"), Some("10".to_string()));
    }

    #[test]
    fn tree_walker() {
        todo!()
    }

    #[test]
    fn text_attributes() {
        todo!()
    }

    #[test]
    fn siblings() {
        todo!()
    }

    #[test]
    fn insert_after() {
        todo!()
    }
}
