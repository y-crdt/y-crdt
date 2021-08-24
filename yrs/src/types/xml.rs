use crate::block::{Item, ItemContent, ItemPosition};
use crate::types::{
    Entries, Inner, InnerRef, Map, Text, TypePtr, TypeRefs, TYPE_REFS_XML_ELEMENT,
    TYPE_REFS_XML_TEXT,
};
use crate::Transaction;
use lib0::any::Any;
use std::cell::Ref;
use std::fmt::Write;

#[derive(Debug, Eq, PartialEq)]
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
        write!(&mut s, "<{}", tag);
        let attributes = Attributes(inner.entries(txn));
        for (k, v) in attributes {
            write!(&mut s, " \"{}\"=\"{}\"", k, v);
        }
        write!(&mut s, ">");
        for i in inner.iter(txn) {
            for content in i.content.get_content(txn) {
                write!(&mut s, "{}", content);
            }
        }
        write!(&mut s, "</{}>", tag);
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

    pub fn next_sibling<T: From<InnerRef>>(&self, txn: &Transaction) -> Option<T> {
        next_sibling(self.inner(), txn)
    }

    pub fn prev_sibling<T: From<InnerRef>>(&self, txn: &Transaction) -> Option<T> {
        prev_sibling(self.inner(), txn)
    }

    pub fn parent<T: From<InnerRef>>(&self, txn: &Transaction) -> Option<T> {
        self.0.parent(txn)
    }

    pub fn first_child<T: From<InnerRef>>(&self, txn: &Transaction) -> Option<T> {
        self.0.first_child(txn)
    }

    pub fn len(&self, txn: &Transaction) -> u32 {
        self.0.len(txn)
    }

    pub fn iter<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> TreeWalker<'b, 'txn> {
        self.0.iter(txn)
    }

    pub fn select<'a, 'b, 'c, 'txn>(
        &'a self,
        txn: &'b Transaction<'txn>,
        query: &'c str,
    ) -> Select<'b, 'c, 'txn> {
        self.0.select(txn, query)
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

#[derive(Debug, Eq, PartialEq)]
pub struct XmlFragment(InnerRef);

impl XmlFragment {
    pub fn new(inner: InnerRef) -> Self {
        XmlFragment(inner)
    }

    fn inner(&self) -> Ref<Inner> {
        self.0.borrow()
    }

    pub fn first_child<T: From<InnerRef>>(&self, txn: &Transaction) -> Option<T> {
        let inner = self.inner();
        let first = inner.first(txn)?;
        match &first.content {
            ItemContent::Type(c) => {
                let value = T::from(c.clone());
                Some(value)
            }
            _ => None,
        }
    }

    pub fn parent<T: From<InnerRef>>(&self, txn: &Transaction) -> Option<T> {
        parent(self.inner(), txn)
    }

    pub fn len(&self, _txn: &Transaction) -> u32 {
        self.inner().len()
    }

    pub fn iter<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> TreeWalker<'b, 'txn> {
        TreeWalker::new(txn, &*self.inner())
    }

    pub fn select<'a, 'b, 'c, 'txn>(
        &'a self,
        txn: &'b Transaction<'txn>,
        query: &'c str,
    ) -> Select<'b, 'c, 'txn> {
        Select::new(self.iter(txn), query)
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
        let (content, idx) = inner.get_at(txn, index)?;
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
        let mut current = parent
            .start
            .as_ref()
            .and_then(|p| txn.store.blocks.get_item(p));

        TreeWalker { txn, current, root }
    }
}

impl<'a, 'txn> Iterator for TreeWalker<'a, 'txn> {
    type Item = XmlElement;

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

                result = Some(XmlElement::from(inner.clone()));
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

pub struct Select<'a, 'b, 'txn> {
    inner: TreeWalker<'a, 'txn>,
    query: &'b str,
}

impl<'a, 'b, 'txn> Select<'a, 'b, 'txn> {
    fn new(inner: TreeWalker<'a, 'txn>, query: &'b str) -> Self {
        Select { inner, query }
    }
}

impl<'a, 'b, 'txn> Iterator for Select<'a, 'b, 'txn> {
    type Item = XmlElement;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(elem) = self.inner.next() {
            let inner = elem.inner();
            if let Some(tag) = inner.name.as_ref() {
                if tag == self.query {
                    drop(inner);
                    return Some(elem);
                }
            }
        }
        None
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

#[derive(Debug, Eq, PartialEq)]
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

    pub fn next_sibling<T: From<InnerRef>>(&self, txn: &Transaction) -> Option<T> {
        next_sibling(self.0.inner(), txn)
    }

    pub fn prev_sibling<T: From<InnerRef>>(&self, txn: &Transaction) -> Option<T> {
        prev_sibling(self.0.inner(), txn)
    }

    pub fn parent<T: From<InnerRef>>(&self, txn: &Transaction) -> Option<T> {
        parent(self.inner(), txn)
    }

    pub fn len(&self, txn: &Transaction) -> u32 {
        self.0.len()
    }

    pub fn insert(&self, txn: &mut Transaction, index: u32, content: &str) {
        self.0.insert(txn, index, content)
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

fn next_sibling<T: From<InnerRef>>(inner: Ref<Inner>, txn: &Transaction) -> Option<T> {
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
                    return Some(T::from(inner.clone()));
                }
            }
        }
    }

    None
}

fn prev_sibling<T: From<InnerRef>>(inner: Ref<Inner>, txn: &Transaction) -> Option<T> {
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
                    return Some(T::from(inner.clone()));
                }
            }
        }
    }

    None
}

fn parent<T: From<InnerRef>>(inner: Ref<Inner>, txn: &Transaction) -> Option<T> {
    let item = txn.store.blocks.get_item(inner.item.as_ref()?)?;
    let parent = txn.store.get_type(&item.parent)?;
    Some(T::from(parent.clone()))
}

#[cfg(test)]
mod test {
    use crate::types::xml::XmlElement;
    use crate::Doc;

    #[test]
    fn insert_attribute() {
        let d1 = Doc::with_client_id(1);
        let mut t1 = d1.transact();
        let xml1 = t1.get_xml_element("xml", "UNDEFINED");
        xml1.insert_attribute(&mut t1, "height", 10);
        assert_eq!(xml1.get_attribute(&t1, "height"), Some("10".to_string()));

        let d2 = Doc::with_client_id(1);
        let mut t2 = d2.transact();
        let xml2 = t2.get_xml_element("xml", "UNDEFINED");
        d2.apply_update(&mut t2, d1.encode_state_as_update(&t1).as_slice());
        assert_eq!(xml2.get_attribute(&t2, "height"), Some("10".to_string()));
    }

    #[test]
    fn tree_walker() {
        let doc = Doc::with_client_id(1);
        let mut txn = doc.transact();
        /*
            <root>
                <p>{txt1}{txt2}</p>
                <p></p>
                <img/>
            </root>
        */
        let root = txn.get_xml_element("xml", "root");
        let p1 = root.push_elem_back(&mut txn, "p");
        let txt1 = p1.push_text_back(&mut txn);
        let txt2 = p1.push_text_back(&mut txn);
        let p2 = root.push_elem_back(&mut txn, "p");
        let img = root.push_elem_back(&mut txn, "img");

        let mut all_paragraphs = root.select(&txn, "p");
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
        let root = txn.get_xml_element("root", "root");
        println!("1st: {}", txn.store);
        let first = root.push_text_back(&mut txn);
        println!("2nd: {}", txn.store);
        first.push(&mut txn, "hello");
        println!("3rd: {}", txn.store);
        let second = root.push_elem_back(&mut txn, "p");
        println!("5th: {}", txn.store);

        assert_eq!(
            first.next_sibling(&txn).as_ref(),
            Some(&second),
            "first.next_sibling should point to second"
        );
        assert_eq!(
            second.prev_sibling(&txn).as_ref(),
            Some(&first),
            "second.prev_sibling should point to first"
        );
        assert_eq!(
            first.parent(&txn).as_ref(),
            Some(&root),
            "first.parent should point to root"
        );
        assert_eq!(
            root.parent::<XmlElement>(&txn).as_ref(),
            None,
            "root parent should not exist"
        );
        assert_eq!(
            root.first_child(&txn).as_ref(),
            Some(&first),
            "root.first_child should point to first"
        );
    }
}
