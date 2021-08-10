fn main() {
    let doc1 = yrs::Doc::new();
    let doc2 = yrs::Doc::new();

    let tr = &mut doc1.transact();
    let ytext = doc1.get_text(tr, "mytype");
    ytext.insert(tr, 0, "a");

    let update = tr.encode_update();

    let tr2 = &mut doc2.transact();
    doc2.apply_update(tr2, &update);
    let ytext2 = doc2.get_text(tr2, "mytext");
    let txt2 = ytext2.to_string(tr);
    println!("synced document state: {}", txt2);
    assert_eq!(txt2, "a");
}
