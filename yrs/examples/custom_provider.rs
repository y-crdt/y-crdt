use std::rc::Rc;

struct MyProvider {
    doc: yrs::Doc
}

impl yrs::Subscriber<yrs::events::UpdateEvent> for MyProvider {
    fn on_change (&self, event: yrs::events::UpdateEvent) {
        self.doc.apply_update(&event.update);
    }
}

fn main () {
    let doc1 = yrs::Doc::new();
    let doc2 = yrs::Doc::new();

    // register update observer
    let provider = Rc::from(MyProvider {
        doc: doc2.clone()
    });
    doc1.on_update(Rc::downgrade(&provider));
    
    let my_type = doc1.get_type("my first shared type");

    {
        // All changes must happen within a transaction. 
        // When the transaction is dropped, the yrs::Doc fires event (e.g. the update event)
        let tr = doc1.transact();
        my_type.insert(&tr, 0, 'a');
    } // transaction is dropped and changes are automatically synced to doc2
    
    println!("synced document state: {}", doc2.get_type("my first shared type").to_string());
    assert_eq!(doc2.get_type("my first shared type").to_string(), "a");
}
