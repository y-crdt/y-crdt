use crate::block::{BlockPtr, Item, ItemContent};
use crate::types::{Entries, Inner, Map, Text, TypePtr, TYPE_REFS_XML_ELEMENT, TYPE_REFS_XML_TEXT};
use crate::Transaction;
use lib0::any::Any;
use std::borrow::Borrow;
use std::cell::{Ref, RefCell};
use std::fmt::Write;
use std::rc::Rc;

pub struct XmlElement(XmlFragment);

impl XmlElement {
    fn inner(&self) -> Ref<Inner> {
        let fragment = &self.0;
        (*fragment.0).borrow()
    }

    pub fn to_string(&self, txn: &Transaction) -> String {
        let mut s = String::new();
        let inner = self.inner();
        let node_name = inner.name.as_ref().unwrap();
        write!(&mut s, "<{}", node_name);
        for (k, v) in self.attributes(txn) {
            write!(&mut s, " \"{}\"=\"{}\"", k, v);
        }
        write!(&mut s, ">");
        for i in inner.iter(txn) {
            for content in i.content.get_content(txn) {
                write!(&mut s, "{}", content);
            }
        }
        write!(&mut s, "</{}>", node_name);
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

impl Into<ItemContent> for XmlElement {
    fn into(self) -> ItemContent {
        self.0.into()
    }
}

impl From<Rc<RefCell<Inner>>> for XmlElement {
    fn from(inner: Rc<RefCell<Inner>>) -> Self {
        XmlElement(XmlFragment::new(inner))
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
        TreeWalker::new(txn, &*self.inner())
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

impl Into<ItemContent> for XmlFragment {
    fn into(self) -> ItemContent {
        ItemContent::Type(self.0.clone())
    }
}

pub struct TreeWalker<'a, 'txn> {
    txn: &'a Transaction<'txn>,
    current: Option<&'a Item>,
    first_call: bool,
    root: TypePtr,
}

impl<'a, 'txn> TreeWalker<'a, 'txn> {
    fn new<'b>(txn: &'a Transaction<'txn>, parent: &'b Inner) -> Self {
        let root = parent.ptr.clone();
        let current = parent
            .start
            .get()
            .and_then(|p| txn.store.blocks.get_item(&p));
        TreeWalker {
            txn,
            current,
            root,
            first_call: true,
        }
    }
}

impl<'a, 'txn> Iterator for TreeWalker<'a, 'txn> {
    type Item = XmlElement;

    /// Tree walker used depth-first search to move over the xml tree.
    fn next(&mut self) -> Option<Self::Item> {
        let mut next = self.current.take();
        if !self.first_call {
            while let Some(n) = next {
                if n.is_deleted() {
                    if let ItemContent::Type(inner) = &n.content {
                        // walk down in the tree
                        let refc: &RefCell<_> = inner.borrow();
                        let parent = refc.borrow();
                        next = parent
                            .start
                            .get()
                            .and_then(|ptr| self.txn.store.blocks.get_item(&ptr));

                        continue;
                    }
                }

                // walk right or up in the tree
                while let Some(n) = next {
                    if let Some(right) = n.right.as_ref() {
                        next = self.txn.store.blocks.get_item(right);
                        break;
                    } else if n.parent == self.root {
                        next = None;
                    } else {
                        next = self
                            .txn
                            .store
                            .get_type(&n.parent)
                            .and_then(|inner| {
                                let i: &RefCell<_> = inner.borrow();
                                i.borrow().start.get()
                            })
                            .and_then(|ptr| self.txn.store.blocks.get_item(&ptr));
                    }
                }
            }
        }

        self.first_call = false;
        let result = if let ItemContent::Type(inner) = &next?.content {
            Some(XmlElement::from(inner.clone()))
        } else {
            None
        };
        self.current = next;
        result
    }
}

pub struct XmlHook(Map);

impl XmlHook {
    pub fn new(map: Map) -> Self {
        XmlHook(map)
    }

    pub fn len(&self, txn: &Transaction) -> usize {
        self.0.len(txn)
    }

    pub fn to_json(&self, txn: &Transaction) -> Any {
        self.0.to_json(txn)
    }

    pub fn keys<'a, 'b, 'txn>(
        &'a self,
        txn: &'b Transaction<'txn>,
    ) -> crate::types::map::Keys<'b, 'txn> {
        self.0.keys(txn)
    }

    pub fn values<'a, 'b, 'txn>(
        &self,
        txn: &'b Transaction<'txn>,
    ) -> crate::types::map::Values<'b, 'txn> {
        self.0.values(txn)
    }

    pub fn iter<'a, 'b, 'txn>(
        &self,
        txn: &'b Transaction<'txn>,
    ) -> crate::types::map::Iter<'b, 'txn> {
        self.0.iter(txn)
    }

    pub fn insert<V: Into<ItemContent>>(
        &self,
        txn: &mut Transaction,
        key: String,
        value: V,
    ) -> Option<Any> {
        self.0.insert(txn, key, value)
    }

    pub fn remove(&self, txn: &mut Transaction, key: &str) -> Option<Any> {
        self.0.remove(txn, key)
    }

    pub fn get(&self, txn: &Transaction, key: &str) -> Option<Any> {
        self.0.get(txn, key)
    }

    pub fn contains(&self, txn: &Transaction, key: &String) -> bool {
        self.0.contains(txn, key)
    }

    pub fn clear(&self, txn: &mut Transaction) {
        self.0.clear(txn)
    }
}

impl Into<ItemContent> for XmlHook {
    fn into(self) -> ItemContent {
        self.0.into()
    }
}

impl From<Rc<RefCell<Inner>>> for XmlHook {
    fn from(inner: Rc<RefCell<Inner>>) -> Self {
        XmlHook(Map::from(inner))
    }
}

impl Into<XmlHook> for Map {
    fn into(self) -> XmlHook {
        XmlHook(self)
    }
}

pub struct XmlText(Text);

impl XmlText {
    pub fn new(inner: Rc<RefCell<Inner>>) -> Self {
        XmlText(Text::from(inner))
    }

    fn inner(&self) -> Ref<Inner> {
        self.0.inner()
    }

    pub fn to_string(&self, txn: &Transaction) -> String {
        todo!()
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
        todo!()
    }

    pub fn prev_sibling(&self, txn: &Transaction) -> Option<XmlElement> {
        todo!()
    }

    pub fn len(&self, txn: &Transaction) -> u32 {
        todo!()
    }

    pub fn insert(&self, txn: &mut Transaction, index: u32, content: &str) {
        todo!()
    }

    pub fn push(&self, txn: &mut Transaction, content: &str) {
        let len = self.len(txn);
        self.insert(txn, len, content);
    }

    pub fn remove(&self, txn: &mut Transaction, index: u32, len: u32) {
        todo!()
    }
}

impl Into<ItemContent> for XmlText {
    fn into(self) -> ItemContent {
        self.0.into()
    }
}

impl From<Rc<RefCell<Inner>>> for XmlText {
    fn from(inner: Rc<RefCell<Inner>>) -> Self {
        XmlText::new(inner)
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
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let paragraph1 = t1.get_xml_element("p");
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
