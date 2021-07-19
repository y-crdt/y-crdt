use crate::block::{Block, BlockPtr, Item, ItemContent, ItemPosition};
use crate::types::{Inner, TypePtr};
use crate::*;
use lib0::any::Any;
use std::collections::HashMap;

pub struct Map {
    ptr: TypePtr,
}

impl Map {
    pub fn from(ptr: TypePtr) -> Self {
        Map { ptr }
    }

    pub fn to_json(&self, txn: &Transaction<'_>) -> Any {
        match txn.store.get_type(&self.ptr) {
            None => Any::Null,
            Some(t) => {
                let mut res = HashMap::new();
                for (key, ptr) in t.map.iter() {
                    if let Some(item) = txn.store.blocks.get_item(ptr) {
                        if !item.deleted {
                            let value = item.content.value().unwrap_or(Any::Null);
                            res.insert(key.clone(), value);
                        }
                    }
                }
                Any::Map(res)
            }
        }
    }

    pub fn len(&self, txn: &Transaction<'_>) -> usize {
        match txn.store.get_type(&self.ptr) {
            None => 0,
            Some(t) => {
                let mut len = 0;
                for ptr in t.map.values() {
                    //TODO: maybe it would be better to just cache len in the map itself?
                    if let Some(item) = txn.store.blocks.get_item(ptr) {
                        if !item.deleted {
                            len += 1;
                        }
                    }
                }
                len
            }
        }
    }

    pub fn keys<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> Keys<'b, 'txn> {
        Keys(Iter::new(self, txn))
    }

    pub fn values<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> Values<'b, 'txn> {
        Values(Iter::new(self, txn))
    }

    pub fn iter<'a, 'b, 'txn>(&'a self, txn: &'b Transaction<'txn>) -> Iter<'b, 'txn> {
        Iter::new(self, txn)
    }

    pub fn insert<T: Into<ItemContent>>(
        &self,
        txn: &mut Transaction<'_>,
        key: String,
        value: T,
    ) -> Option<Any> {
        let previous = self.get(txn, &key);

        let parent = self.ptr.clone();
        let t = txn.store.get_type(&self.ptr).unwrap();
        let left = t.map.get(&key);
        let pos = ItemPosition {
            parent,
            left: left.cloned(),
            right: None,
            index: 0,
        };

        txn.create_item(&pos, value.into(), Some(key));
        previous
    }

    pub fn remove(&self, txn: &mut Transaction<'_>, key: &String) -> Option<Any> {
        let t = txn.store.get_type(&self.ptr)?;
        let ptr = t.map.get(key)?;
        let item = txn.store.blocks.get_item(ptr)?;

        if item.deleted {
            None
        } else {
            let previous = item.content.value();
            item.mark_as_deleted();
            previous
        }
    }

    pub fn get(&self, txn: &Transaction<'_>, key: &String) -> Option<Any> {
        let t = txn.store.get_type(&self.ptr)?;
        let ptr = t.map.get(key)?;
        let item = txn.store.blocks.get_item(ptr)?;
        if item.deleted {
            None
        } else {
            item.content.value()
        }
    }

    pub fn contains(&self, txn: &Transaction<'_>, key: &String) -> bool {
        let t = txn.store.get_type(&self.ptr).unwrap();
        if let Some(ptr) = t.map.get(key) {
            if let Some(item) = txn.store.blocks.get_item(ptr) {
                return !item.deleted;
            }
        }
        false
    }

    pub fn clear(&self, txn: &mut Transaction<'_>) {
        let t = txn.store.get_type(&self.ptr).unwrap();
        for (_, ptr) in t.map.iter() {
            if let Some(item) = txn.store.blocks.get_item(ptr) {
                if !item.deleted {
                    item.mark_as_deleted();
                }
            }
        }
    }
}

pub struct Iter<'a, 'txn> {
    txn: &'a Transaction<'txn>,
    iter: std::collections::hash_map::Iter<'a, String, BlockPtr>,
}

impl<'a, 'txn> Iter<'a, 'txn> {
    fn new<'b>(map: &'b Map, txn: &'a Transaction<'txn>) -> Self {
        let t = txn.store.get_type(&map.ptr).unwrap();
        let iter = t.map.iter();
        Iter { txn, iter }
    }
}

impl<'a, 'txn> Iterator for Iter<'a, 'txn> {
    type Item = (&'a String, Vec<Any>);

    fn next(&mut self) -> Option<Self::Item> {
        let (mut key, ptr) = self.iter.next()?;
        let mut block = self.txn.store.blocks.get_item(ptr);
        loop {
            match block {
                Some(item) if !item.deleted => {
                    break;
                }
                _ => {
                    let (k, ptr) = self.iter.next()?;
                    key = k;
                    block = self.txn.store.blocks.get_item(ptr);
                }
            }
        }
        let item = block.unwrap();
        Some((key, item.content.get_content()))
    }
}

pub struct Keys<'a, 'txn>(Iter<'a, 'txn>);

impl<'a, 'txn> Iterator for Keys<'a, 'txn> {
    type Item = &'a String;

    fn next(&mut self) -> Option<Self::Item> {
        let (key, _) = self.0.next()?;
        Some(key)
    }
}

pub struct Values<'a, 'txn>(Iter<'a, 'txn>);

impl<'a, 'txn> Iterator for Values<'a, 'txn> {
    type Item = Vec<Any>;

    fn next(&mut self) -> Option<Self::Item> {
        let (_, value) = self.0.next()?;
        Some(value)
    }
}

#[cfg(test)]
mod test {

    #[test]
    fn map_basic() {
        todo!()
    }

    #[test]
    fn map_get_set() {
        todo!()
    }

    #[test]
    fn map_get_set_array() {
        todo!()
    }

    #[test]
    fn map_get_set_sync() {
        todo!()
    }

    #[test]
    fn map_get_set_sync_with_conflicts() {
        todo!()
    }

    #[test]
    fn map_remove() {
        todo!()
    }

    #[test]
    fn map_clear() {
        todo!()
    }

    #[test]
    fn map_clear_sync() {
        todo!()
    }

    #[test]
    fn map_get_set_with_3_way_conflicts() {
        todo!()
    }

    #[test]
    fn map_get_set_remove_with_3_way_conflicts() {
        todo!()
    }
}
