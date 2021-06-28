use crate::block::Block;
use crate::*;
use lib0::any::Any;

pub struct Map {
    ptr: types::TypePtr,
}

impl Map {
    pub fn new() -> Self {
        todo!()
    }

    pub fn to_json(&self) -> Any {
        todo!()
    }

    pub fn len(&self) -> usize {
        todo!()
    }

    pub fn keys(&self) -> Keys<'_> {
        todo!()
    }

    pub fn values(&self) -> Values<'_> {
        todo!()
    }

    pub fn iter(&self) -> Iter<'_> {
        todo!()
    }

    pub fn insert(&self, txn: &mut Transaction<'_>, key: String, value: Any) -> Option<Any> {
        todo!()
    }

    pub fn remove(&self, txn: &mut Transaction<'_>, key: &String) -> Option<Any> {
        todo!()
    }

    pub fn get(&self, txn: &Transaction<'_>, key: &String) -> Option<Any> {
        todo!()
    }

    pub fn contains(&self, txn: &Transaction<'_>, key: &String) -> bool {
        todo!()
    }

    pub fn clear(&self, txn: &mut Transaction<'_>) {
        todo!()
    }
}

impl Default for Map {
    fn default() -> Self {
        todo!()
    }
}

impl<T> From<T> for Map
where
    T: IntoIterator<Item = Any>,
{
    fn from(iterable: T) -> Self {
        todo!()
    }
}

struct Iter<'a> {}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a String, &'a Block);

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

struct Keys<'a> {}

impl<'a> Iterator for Keys<'a> {
    type Item = &'a String;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

struct Values<'a> {}

impl<'a> Iterator for Values<'a> {
    type Item = &'a Block;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

#[cfg(test)]
mod test {

    #[test]
    fn map_from() {
        todo!()
    }

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
