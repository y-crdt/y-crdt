use std::rc::Rc;
use yrs::*;

const ITERATIONS: u32 = 2000;

/// This is a complete example creating several Yjs documents that sync with each other.
/// * The sync-steps are used to sync documents manually.
/// * The YProvider is used to automatically sync a document with doc1. It shows how you could implement
///   a network adapter for Yjs.
struct YProvider {
    doc: Rc<Doc>,
}

fn main() {
    // this doc will receive updates from doc1
    let doc_synced = Rc::from(Doc::new());
    let provider = Rc::new(YProvider {
        doc: doc_synced.clone(),
    });

    let doc1 = Doc::new();
    let tr = &mut doc1.transact();
    let t = doc1.get_text(tr, "");
    {
        // scope the transaction so that it is droped and the update is synced
        // to doc_synced
        t.insert(tr, 0, "x");
        for _ in 0..ITERATIONS {
            t.insert(tr, 0, "a")
        }
    }
    println!("doc1 content {}", t.to_string(tr));
    let update = doc1.encode_state_as_update(tr);
    println!("update.len: {}", update.len());

    println!(
        "doc_synced content (should be the same as doc1) {}",
        doc_synced.get_text(tr, "").to_string(tr)
    );

    let bs: Vec<u8> = doc1.client_id.to_ne_bytes().iter().map(|x| *x).collect();
    println!("client_id: {}, ne_bytes: {:?}", doc1.client_id, bs);
    let doc2 = Doc::new();
    let tr2 = &mut doc2.transact();
    let t2 = doc2.get_text(tr2, "");
    doc2.apply_update(tr2, &update);
    println!(
        "doc2 content (this is manually synced from doc1) {}",
        t2.to_string(tr2)
    );
}
