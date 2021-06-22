use crate::Doc;

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
    let mut doc = Doc::new();
    let mut txn = doc.transact();
    let txt = txn.get_text("type");
    doc.apply_update(&mut txn, update);
    assert_eq!(txt.to_string(&txn), "abhi".to_string());
}
