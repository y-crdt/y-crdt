use crate::block::{Item, ItemContent, ItemPosition};
use crate::types::{
    Entries, Inner, InnerRef, Map, Text, TypePtr, TypeRefs, TYPE_REFS_XML_ELEMENT,
    TYPE_REFS_XML_TEXT,
};
use crate::Transaction;
use lib0::any::Any;
use std::cell::Ref;
use std::fmt::Write;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Xml {
    Element(XmlElement),
    Text(XmlText),
}

impl From<InnerRef> for Xml {
    fn from(inner: InnerRef) -> Self {
        let type_ref = { inner.borrow().type_ref & 0b1111 };
        match type_ref {
            TYPE_REFS_XML_ELEMENT => Xml::Element(XmlElement::from(inner)),
            TYPE_REFS_XML_TEXT => Xml::Text(XmlText::from(inner)),
            other => panic!("Unsupported type: {}", other),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct XmlElement(XmlFragment);

impl XmlElement {
    fn inner(&self) -> Ref<Inner> {
        self.0.inner()
    }

    pub fn to_string(&self, txn: &Transaction) -> String {
        let inner = self.inner();
        Self::to_string_inner(&*inner, txn)
    }

    pub(crate) fn to_string_inner(inner: &Inner, txn: &Transaction) -> String {
        let mut s = String::new();
        let tag = inner
            .name
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or(&"UNDEFINED");
        write!(&mut s, "<{}", tag).unwrap();
        let attributes = Attributes(inner.entries(txn));
        for (k, v) in attributes {
            write!(&mut s, " \"{}\"=\"{}\"", k, v).unwrap();
        }
        write!(&mut s, ">").unwrap();
        for i in inner.iter(txn) {
            for content in i.content.get_content(txn) {
                write!(&mut s, "{}", content).unwrap();
            }
        }
        write!(&mut s, "</{}>", tag).unwrap();
        s
    }

    pub fn tag(&self) -> &str {
        let inner = self.0 .0.as_ref();
        inner.name.as_ref().unwrap()
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
        let key = attr_name.to_string();
        let value = attr_value.to_string();
        let pos = {
            let inner = self.inner();
            let left = inner.map.get(&key);
            ItemPosition {
                parent: inner.ptr.clone(),
                left: left.cloned(),
                right: None,
                index: 0,
            }
        };

        txn.create_item(&pos, value.into(), Some(key));
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

    pub fn next_sibling(&self, txn: &Transaction) -> Option<Xml> {
        next_sibling(self.inner(), txn)
    }

    pub fn prev_sibling(&self, txn: &Transaction) -> Option<Xml> {
        prev_sibling(self.inner(), txn)
    }

    pub fn parent(&self, txn: &Transaction) -> Option<XmlElement> {
        self.0.parent(txn)
    }

    pub fn first_child(&self, txn: &Transaction) -> Option<Xml> {
        self.0.first_child(txn)
    }

    pub fn len(&self, txn: &Transaction) -> u32 {
        self.0.len(txn)
    }

    pub fn iter<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> TreeWalker<'b, 'txn> {
        self.0.iter(txn)
    }

    pub fn insert_elem<S: ToString>(
        &self,
        txn: &mut Transaction,
        index: u32,
        name: S,
    ) -> XmlElement {
        self.0.insert_elem(txn, index, name)
    }

    pub fn insert_text(&self, txn: &mut Transaction, index: u32) -> XmlText {
        self.0.insert_text(txn, index)
    }

    pub fn remove(&self, txn: &mut Transaction, index: u32, len: u32) {
        self.0.remove(txn, index, len)
    }

    pub fn push_elem_back<S: ToString>(&self, txn: &mut Transaction, name: S) -> XmlElement {
        self.0.push_elem_back(txn, name)
    }

    pub fn push_elem_front<S: ToString>(&self, txn: &mut Transaction, name: S) -> XmlElement {
        self.0.push_elem_front(txn, name)
    }

    pub fn push_text_back(&self, txn: &mut Transaction) -> XmlText {
        self.0.push_text_back(txn)
    }

    pub fn push_text_front(&self, txn: &mut Transaction) -> XmlText {
        self.0.push_text_front(txn)
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

impl From<InnerRef> for XmlElement {
    fn from(inner: InnerRef) -> Self {
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

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct XmlFragment(InnerRef);

impl XmlFragment {
    pub fn new(inner: InnerRef) -> Self {
        XmlFragment(inner)
    }

    fn inner(&self) -> Ref<Inner> {
        self.0.borrow()
    }

    pub fn first_child(&self, txn: &Transaction) -> Option<Xml> {
        let inner = self.inner();
        let first = inner.first(txn)?;
        match &first.content {
            ItemContent::Type(c) => {
                let value = Xml::from(c.clone());
                Some(value)
            }
            _ => None,
        }
    }

    pub fn parent(&self, txn: &Transaction) -> Option<XmlElement> {
        parent(self.inner(), txn)
    }

    pub fn len(&self, _txn: &Transaction) -> u32 {
        self.inner().len()
    }

    pub fn iter<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> TreeWalker<'b, 'txn> {
        TreeWalker::new(txn, &*self.inner())
    }

    pub fn to_string(&self, txn: &Transaction) -> String {
        let mut s = String::new();
        let inner = self.inner();
        for i in inner.iter(txn) {
            for content in i.content.get_content(txn) {
                write!(&mut s, "{}", content).unwrap();
            }
        }
        s
    }

    pub fn insert_elem<S: ToString>(
        &self,
        txn: &mut Transaction,
        index: u32,
        name: S,
    ) -> XmlElement {
        let inner = self.insert(txn, index, TYPE_REFS_XML_ELEMENT, Some(name.to_string()));
        XmlElement::from(inner)
    }

    pub fn insert_text(&self, txn: &mut Transaction, index: u32) -> XmlText {
        let inner = self.insert(txn, index, TYPE_REFS_XML_TEXT, None);
        XmlText::from(inner)
    }

    fn insert(
        &self,
        txn: &mut Transaction,
        index: u32,
        type_refs: TypeRefs,
        name: Option<String>,
    ) -> InnerRef {
        let (start, parent) = {
            let parent = self.inner();
            if index <= parent.len() {
                (parent.start, parent.ptr.clone())
            } else {
                panic!("Cannot insert item at index over the length of an array")
            }
        };
        let (left, right) = if index == 0 {
            (None, None)
        } else {
            Inner::index_to_ptr(txn, start, index)
        };
        let content = {
            let parent = self.inner();
            let inner = Inner::new(parent.ptr.clone(), type_refs, name);
            InnerRef::new(inner)
        };
        let pos = ItemPosition {
            parent,
            left,
            right,
            index: 0,
        };
        txn.create_item(&pos, ItemContent::Type(content.clone()), None);
        content
    }

    pub fn remove(&self, txn: &mut Transaction, index: u32, len: u32) {
        self.0.remove_at(txn, index, len)
    }

    pub fn push_elem_back<S: ToString>(&self, txn: &mut Transaction, name: S) -> XmlElement {
        let len = self.len(txn);
        self.insert_elem(txn, len, name)
    }

    pub fn push_elem_front<S: ToString>(&self, txn: &mut Transaction, name: S) -> XmlElement {
        self.insert_elem(txn, 0, name)
    }

    pub fn push_text_back(&self, txn: &mut Transaction) -> XmlText {
        let len = self.len(txn);
        self.insert_text(txn, len)
    }

    pub fn push_text_front(&self, txn: &mut Transaction) -> XmlText {
        self.insert_text(txn, 0)
    }

    pub fn get<T: From<InnerRef>>(&self, txn: &Transaction, index: u32) -> Option<T> {
        let inner = self.inner();
        let (content, _) = inner.get_at(txn, index)?;
        if let ItemContent::Type(inner) = content {
            Some(T::from(inner.clone()))
        } else {
            None
        }
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
    root: TypePtr,
}

impl<'a, 'txn> TreeWalker<'a, 'txn> {
    fn new<'b>(txn: &'a Transaction<'txn>, parent: &'b Inner) -> Self {
        let root = parent.ptr.clone();
        let current = parent
            .start
            .as_ref()
            .and_then(|p| txn.store.blocks.get_item(p));

        TreeWalker { txn, current, root }
    }
}

impl<'a, 'txn> Iterator for TreeWalker<'a, 'txn> {
    type Item = Xml;

    /// Tree walker used depth-first search to move over the xml tree.
    fn next(&mut self) -> Option<Self::Item> {
        let mut result = None;
        while let Some(current) = self.current.take() {
            if current.is_deleted() {
                self.current = current
                    .right
                    .as_ref()
                    .and_then(|ptr| self.txn.store.blocks.get_item(ptr));
                continue;
            }

            if let ItemContent::Type(inner) = &current.content {
                // walk down in the tree
                self.current = {
                    inner
                        .borrow()
                        .start
                        .as_ref()
                        .and_then(|ptr| self.txn.store.blocks.get_item(ptr))
                };

                result = Some(Xml::from(inner.clone()));
            }

            if self.current.is_none() {
                self.current = if let Some(right) = current.right.as_ref() {
                    self.txn.store.blocks.get_item(right) // walk to right neighbor
                } else if current.parent != self.root {
                    // walk up in the tree hierarchy
                    self.txn
                        .store
                        .get_type(&current.parent)
                        .and_then(|inner| inner.borrow().item.clone())
                        .and_then(|ptr| self.txn.store.blocks.get_item(&ptr))
                } else {
                    None
                };
            }
            break;
        }
        result
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
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

impl From<InnerRef> for XmlHook {
    fn from(inner: InnerRef) -> Self {
        XmlHook(Map::from(inner))
    }
}

impl Into<XmlHook> for Map {
    fn into(self) -> XmlHook {
        XmlHook(self)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct XmlText(Text);

impl XmlText {
    fn inner(&self) -> Ref<Inner> {
        self.0.inner()
    }

    pub fn to_string(&self, txn: &Transaction) -> String {
        self.0.to_string(txn)
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
        let key = attr_name.to_string();
        let value = attr_value.to_string();
        let pos = {
            let inner = self.inner();
            let left = inner.map.get(&key);
            ItemPosition {
                parent: inner.ptr.clone(),
                left: left.cloned(),
                right: None,
                index: 0,
            }
        };

        txn.create_item(&pos, value.into(), Some(key));
    }

    pub fn get_attribute(&self, txn: &Transaction, attr_name: &str) -> Option<String> {
        let inner: Ref<_> = self.inner();
        let value = inner.get(txn, attr_name)?;
        Some(value.to_string())
    }

    pub fn attributes<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> Attributes<'b, 'txn> {
        Attributes(self.inner().entries(txn))
    }

    pub fn next_sibling(&self, txn: &Transaction) -> Option<Xml> {
        next_sibling(self.0.inner(), txn)
    }

    pub fn prev_sibling(&self, txn: &Transaction) -> Option<Xml> {
        prev_sibling(self.0.inner(), txn)
    }

    pub fn parent(&self, txn: &Transaction) -> Option<XmlElement> {
        parent(self.inner(), txn)
    }

    pub fn len(&self, _txn: &Transaction) -> u32 {
        self.0.len()
    }

    pub fn insert(&self, txn: &mut Transaction, index: u32, content: &str) {
        if let Some(mut pos) = self.0.find_position(txn, index) {
            let parent = { TypePtr::Id(self.inner().item.unwrap()) };
            pos.parent = parent;
            txn.create_item(&pos, ItemContent::String(content.to_owned()), None);
        } else {
            panic!("The type or the position doesn't exist!");
        }
    }

    pub fn push(&self, txn: &mut Transaction, content: &str) {
        let len = self.len(txn);
        self.insert(txn, len, content);
    }

    pub fn remove(&self, txn: &mut Transaction, index: u32, len: u32) {
        self.0.remove(txn, index, len)
    }
}

impl Into<ItemContent> for XmlText {
    fn into(self) -> ItemContent {
        self.0.into()
    }
}

impl From<InnerRef> for XmlText {
    fn from(inner: InnerRef) -> Self {
        XmlText(Text::from(inner))
    }
}

impl Into<XmlText> for Text {
    fn into(self) -> XmlText {
        XmlText(self)
    }
}

fn next_sibling(inner: Ref<Inner>, txn: &Transaction) -> Option<Xml> {
    let mut current = inner
        .item
        .as_ref()
        .and_then(|ptr| txn.store.blocks.get_item(ptr));
    while let Some(item) = current {
        current = item
            .right
            .as_ref()
            .and_then(|ptr| txn.store.blocks.get_item(ptr));
        if let Some(right) = current {
            if !right.is_deleted() {
                if let ItemContent::Type(inner) = &right.content {
                    return Some(Xml::from(inner.clone()));
                }
            }
        }
    }

    None
}

fn prev_sibling(inner: Ref<Inner>, txn: &Transaction) -> Option<Xml> {
    let mut current = inner
        .item
        .as_ref()
        .and_then(|ptr| txn.store.blocks.get_item(ptr));
    while let Some(item) = current {
        current = item
            .left
            .as_ref()
            .and_then(|ptr| txn.store.blocks.get_item(ptr));
        if let Some(left) = current {
            if !left.is_deleted() {
                if let ItemContent::Type(inner) = &left.content {
                    return Some(Xml::from(inner.clone()));
                }
            }
        }
    }

    None
}

fn parent(inner: Ref<Inner>, txn: &Transaction) -> Option<XmlElement> {
    let item = txn.store.blocks.get_item(inner.item.as_ref()?)?;
    let parent = txn.store.get_type(&item.parent)?;
    Some(XmlElement::from(parent.clone()))
}

#[cfg(test)]
mod test {
    use crate::types::xml::{Xml, XmlElement};
    use crate::Doc;

    #[test]
    fn insert_attribute() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let xml1 = t1.get_xml_element("xml");
        xml1.insert_attribute(&mut t1, "height", 10);
        assert_eq!(xml1.get_attribute(&t1, "height"), Some("10".to_string()));

        let d2 = Doc::with_client_id(1);
        let mut t2 = d2.transact();
        let xml2 = t2.get_xml_element("xml");
        d2.apply_update(&mut t2, d1.encode_state_as_update(&t1).as_slice());
        assert_eq!(xml2.get_attribute(&t2, "height"), Some("10".to_string()));
    }

    #[test]
    fn tree_walker() {
        let doc = Doc::with_client_id(1);
        let mut txn = doc.transact();
        /*
            <UNDEFINED>
                <p>{txt1}{txt2}</p>
                <p></p>
                <img/>
            </UNDEFINED>
        */
        let root = txn.get_xml_element("xml");
        let p1 = root.push_elem_back(&mut txn, "p");
        p1.push_text_back(&mut txn);
        p1.push_text_back(&mut txn);
        let p2 = root.push_elem_back(&mut txn, "p");
        root.push_elem_back(&mut txn, "img");

        let all_paragraphs = root.iter(&txn).filter_map(|n| match n {
            Xml::Element(e) if e.tag() == "p" => Some(e),
            _ => None,
        });
        let actual: Vec<_> = all_paragraphs.collect();

        assert_eq!(
            actual.len(),
            2,
            "query selector should found two paragraphs"
        );
        assert_eq!(actual[0], p1, "query selector found 1st paragraph");
        assert_eq!(actual[1], p2, "query selector found 2nd paragraph");
    }

    #[test]
    fn text_attributes() {
        let doc = Doc::with_client_id(1);
        let mut txn = doc.transact();
        let txt = txn.get_xml_text("txt");
        txt.insert_attribute(&mut txn, "test", 42);

        assert_eq!(txt.get_attribute(&txn, "test"), Some("42".to_string()));
        let actual: Vec<_> = txt.attributes(&txn).collect();
        assert_eq!(actual, vec![("test", "42".to_string())]);
    }

    #[test]
    fn siblings() {
        let doc = Doc::with_client_id(1);
        let mut txn = doc.transact();
        let root = txn.get_xml_element("root");
        let first = root.push_text_back(&mut txn);
        first.push(&mut txn, "hello");
        let second = root.push_elem_back(&mut txn, "p");

        assert_eq!(
            first.next_sibling(&txn).as_ref(),
            Some(&Xml::Element(second.clone())),
            "first.next_sibling should point to second"
        );
        assert_eq!(
            second.prev_sibling(&txn).as_ref(),
            Some(&Xml::Text(first.clone())),
            "second.prev_sibling should point to first"
        );
        assert_eq!(
            first.parent(&txn).as_ref(),
            Some(&root),
            "first.parent should point to root"
        );
        assert_eq!(
            root.parent(&txn).as_ref(),
            None,
            "root parent should not exist"
        );
        assert_eq!(
            root.first_child(&txn).as_ref(),
            Some(&Xml::Text(first)),
            "root.first_child should point to first"
        );
    }

    #[test]
    fn serialization() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let r1 = t1.get_xml_element("root");
        let first = r1.push_text_back(&mut t1);
        first.push(&mut t1, "hello");
        let second = r1.push_elem_back(&mut t1, "p");

        let expected = "<UNDEFINED>hello<p></p></UNDEFINED>";
        assert_eq!(r1.to_string(&t1), expected);

        let u1 = d1.encode_state_as_update(&t1);

        let d2 = Doc::with_client_id(2);
        let mut t2 = d2.transact();
        let r2 = t2.get_xml_element("root");

        d2.apply_update(&mut t2, u1.as_slice());
        assert_eq!(r2.to_string(&t2), expected);
    }

    #[test]
    fn serialization_compatibility() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let r1 = t1.get_xml_element("root");
        let first = r1.push_text_back(&mut t1);
        first.push(&mut t1, "hello");
        let second = r1.push_elem_back(&mut t1, "p");

        /* This binary is result of following Yjs code (matching Rust code above):
        ```js
            let d1 = new Y.Doc()
            d1.clientID = 1
            let root = d1.get('root', Y.XmlElement)
            let first = new Y.XmlText()
            first.insert(0, 'hello')
            let second = new Y.XmlElement('p')
            root.insert(0, [first,second])

            let expected = Y.encodeStateAsUpdate(d1)
        ``` */
        let expected = &[
            1, 3, 1, 0, 7, 1, 4, 114, 111, 111, 116, 6, 4, 0, 1, 0, 5, 104, 101, 108, 108, 111,
            135, 1, 0, 3, 1, 112, 0,
        ];
        let u1 = d1.encode_state_as_update(&t1);
        assert_eq!(u1.as_slice(), expected);
    }
}
