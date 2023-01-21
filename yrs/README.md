# Yrs

Yrs (read: *wires*) is a Rust port of the [Yjs framework](https://yjs.dev/). 

It's a library used on collaborative document editing using Conflict-free Replicated Data Types.
This enables to provide a shared document editing experience on a client devices without explicit requirement for hosting a single server - CRDTs can resolve potential update conflicts on their own with no central authority - as well as provide first-class offline editing capabilities, where document replicas are modified without having connection to each other, and then synchronize automatically once such connection is enabled.

This library contains Rust API, that's used further on by other projects in this repository:

- [C foreign function interface](../yffi/README.md) to provide native interop that could be used by other host languages like Swift or Java.
- [ywasm](../ywasm/README.md) which targets Web Assembly bindings and can be used directly from JavaScript.

## Example

```rust
use yrs::*;

fn main() {    
    let doc = Doc::new();
    let text = doc.get_or_insert_text("name");
    // every operation in Yrs happens in scope of a transaction
    let mut txn = doc.transact_mut(); 
    // append text to our collaborative document
    text.push(&mut txn, "Hello from yrs!");
    // add formatting section to part of the text
    text.format(&mut txn, 11, 3, HashMap::from([
      ("link".into(), "https://github.com/y-crdt/y-crdt".into())
    ]));
    
    // simulate update with remote peer
    let remote_doc = Doc::new();
    let remote_text = remote_doc.get_or_insert_text("name");
    let mut remote_txn = remote_doc.transact();

    // in order to exchange data with other documents 
    // we first need to create a state vector
    let state_vector = remote_txn.state_vector();
    
    // now compute a differential update based on remote document's 
    // state vector
    let bytes = txn.encode_diff_v1(&state_vector);
    
    // both update and state vector are serializable, we can pass them 
    // over the wire now apply update to a remote document
    let update = Update::decode_v1(&bytes).unwrap();
    remote_txn.apply_update(update);

    // display raw text (no attributes)
    println!("{}", remote_text.to_string(&remote_txn));
  
    // create sequence of text chunks with optional format attributes
    let diff = remote_text.diff(&remote_txn, YChange::identity);
}

```

## [Documentation](https://docs.rs/yrs/)

## Features

We're in ongoing process of reaching the feature compatibility with Yjs project. Current feature list:

- [x] Yjs update binary format (v1).
- [x] Yjs update binary format (v2).
- [x] Support for state vectors, delta diff updates and merges.
- [x] Subscription events for incoming updates.
- [x] Support for shared (CRDT) types:
  - [x] Text
  - [x] Array
  - [x] Map
  - [x] XML data types (XmlFragment, XmlElement, XmlText, XmlHook)
  - [x] Subdocuments
  - [x] Subscription events on particular data type
- [x] Cross-platform support for unicode code points
- [x] Transaction origins
- [x] Undo manager

## Internal Documentation

Yrs implements the same algorithm and uses the same data structures as Yjs. We
hope to achieve better performance because we can manually manage memory.
Eventually Yrs might replace the Yjs module by generating a wasm module from
this package.

In this package we want to use better terminology and experiment with new data
structures to index data.

Each change on the document generates a small piece of information that is
integrated into the document. In Yrs we call these small pieces **blocks**.
Think of them as small Lego blocks that compose your document. Each block is
uniquely identified by an identifier consisting of client id and sequence number 
(see: [Lamport Clock](https://en.wikipedia.org/wiki/Lamport_timestamp)). 
When you receive a duplicate block, you discard it. In some cases blocks can be put 
together to form a larger block. If necessary, blocks can be disassembled if only 
a part of the composed block is needed.

More information on the implemented algorithm can be found here:

* [Paper describing the YATA conflict resolution algorithm](https://www.researchgate.net/publication/310212186_Near_Real-Time_Peer-to-Peer_Shared_Editing_on_Extensible_Data_Types)
* [Short overview of the Yjs optimizations](https://github.com/yjs/yjs/blob/main/README.md#yjs-crdt-algorithm)
* [Motivation for composable blocks](https://blog.kevinjahns.de/are-crdts-suitable-for-shared-editing/)
* [Internal documentation of the Yjs project](https://github.com/yjs/yjs/blob/main/INTERNALS.md)
* [Walkthrough of the Yjs codebase](https://youtu.be/0l5XgnQ6rB4)
