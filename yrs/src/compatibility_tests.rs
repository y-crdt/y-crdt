use crate::block::{Block, BlockPtr, Item, ItemContent};
use crate::id_set::{DeleteSet, IdSet};
use crate::types::TypePtr;
use crate::{Doc, ID};
use std::cell::Cell;
use std::rc::Rc;

#[test]
fn text_insert_delete() {
    /** Generated via:
        ```js
           const doc = new Y.Doc()
           const ytext = doc.getText('type')
           doc.transact(function () {
               ytext.insert(0, 'def')
               ytext.insert(0, 'abc')
               ytext.insert(6, 'ghi')
               ytext.delete(2, 5)
           })
           const update = Y.encodeStateAsUpdate(doc)
           ytext.toString() // => 'abhi'
        ```
        This way we confirm that we can decode and apply:
        1. blocks without left/right origin consisting of multiple characters
        2. blocks with left/right origin consisting of multiple characters
        3. delete sets
    */
    let update = &[
        1, 5, 152, 234, 173, 126, 0, 1, 1, 4, 116, 121, 112, 101, 3, 68, 152, 234, 173, 126, 0, 2,
        97, 98, 193, 152, 234, 173, 126, 4, 152, 234, 173, 126, 0, 1, 129, 152, 234, 173, 126, 2,
        1, 132, 152, 234, 173, 126, 6, 2, 104, 105, 1, 152, 234, 173, 126, 2, 0, 3, 5, 2,
    ];
    const CLIENT_ID: u64 = 264992024;
    let expected_blocks = vec![
        Block::Item(Item {
            id: ID::new(CLIENT_ID, 0),
            left: None,
            right: None,
            origin: None,
            right_origin: None,
            content: ItemContent::Deleted(3),
            parent: TypePtr::Named("type".to_string()),
            parent_sub: None,
            deleted: false,
        }),
        Block::Item(Item {
            id: ID::new(CLIENT_ID, 3),
            left: None,
            right: None,
            origin: None,
            right_origin: Some(ID::new(CLIENT_ID, 0)),
            content: ItemContent::String("ab".to_string()),
            parent: TypePtr::Id(BlockPtr::new(ID::new(CLIENT_ID, 0), 0)),
            parent_sub: None,
            deleted: false,
        }),
        Block::Item(Item {
            id: ID::new(CLIENT_ID, 5),
            left: None,
            right: None,
            origin: Some(ID::new(CLIENT_ID, 4)),
            right_origin: Some(ID::new(CLIENT_ID, 0)),
            content: ItemContent::Deleted(1),
            parent: TypePtr::Id(BlockPtr::new(ID::new(CLIENT_ID, 4), 4)),
            parent_sub: None,
            deleted: false,
        }),
        Block::Item(Item {
            id: ID::new(CLIENT_ID, 6),
            left: None,
            right: None,
            origin: Some(ID::new(CLIENT_ID, 2)),
            right_origin: None,
            content: ItemContent::Deleted(1),
            parent: TypePtr::Id(BlockPtr::new(ID::new(CLIENT_ID, 2), 2)),
            parent_sub: None,
            deleted: false,
        }),
        Block::Item(Item {
            id: ID::new(CLIENT_ID, 7),
            left: None,
            right: None,
            origin: Some(ID::new(CLIENT_ID, 6)),
            right_origin: None,
            content: ItemContent::String("hi".to_string()),
            parent: TypePtr::Id(BlockPtr::new(ID::new(CLIENT_ID, 6), 6)),
            parent_sub: None,
            deleted: false,
        }),
    ];
    let expected_ds = {
        let mut ds = IdSet::new();
        ds.insert(ID::new(CLIENT_ID, 0), 3);
        ds.insert(ID::new(CLIENT_ID, 5), 2);
        DeleteSet::from(ds)
    };
    let visited = Rc::new(Cell::new(false));
    let setter = visited.clone();

    let mut doc = Doc::new();
    let _sub = doc.on_update(move |e| {
        for (actual, expected) in e.update.blocks().zip(expected_blocks.as_slice()) {
            //println!("{}", actual);
            assert_eq!(actual, expected);
        }
        assert_eq!(&e.delete_set, &expected_ds);
        setter.set(true);
    });
    let mut txn = doc.transact();
    let txt = txn.get_text("type");
    doc.apply_update(&mut txn, update);
    assert_eq!(txt.to_string(&txn), "abhi".to_string());
    assert!(visited.get());
}
