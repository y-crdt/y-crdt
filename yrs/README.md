# Yrs

> Yjs port to Rust (WIP). [Rust Docs](https://docs.rs/yrs/)

Yrs "wires" is a Rust port of the Yjs framework. The [Ywasm](https://github.com/yjs/Ywasm) project generates a wasm library based on this package.

## Status

> :warning: Highly experimental. Feedback and involvement is welcome!

* Partially implemented the binary encoding protocol. Still need to port the rest of lib0/encoding.
* Only implements the Text type.
* No conflict resolution yet.

I'm currently mainly experimenting with designing a convenient API for the Yrs CRDT. I want to make it work very similar to Yjs, but contrary to Yjs you should always know what you get back (proper types, no duck typing).

## Run an example

```sh
cargo run --example [example-name]
```

## Run benchmarks

```sh
cargo bench
```

## Run tests & documentation examples

```sh
cargo test
```

## Internal Documentation

Yrs implements the same algorithm and uses the same data structures as Yjs. We
hope to achieve better performance because we can manually manage memory.
Eventually Yrs might replace the Yjs module by generating a wasm module from
this package.

In this package we want to use better terminology and experiment with new data
structures to index data. Most CRDTs need to iterate the list structure to find
an insert position. When the document is large, finding the correct position
might be very expensive. Yjs uses a form of caching to index list positions. In
Yrs we want to use skip lists to index positions (e.g.
[skiplistrs](https://github.com/josephg/skiplistrs)).

In Yjs we use the term *Struct* to refer to the building blocks of which Yjs
types are composed. In some CRDTs, the term operation is prefered. In Yjs the
document is composed out of the operations objects. Since operations usually
represent a change on a document, the term seems inapproriate in implementations
of Yjs. Now the term *Struct* is ambigious in the Rust language because 
*struct* is a keyword and is used in a different context.

Each change on the document generates a small piece of information that is
integrated into the document. In Yrs we call these small pieces **blocks**.
Thinks of them as small Lego blocks that compose your document. Each block is
uniquely identified by an identifier. When you receive a duplicate block, you
discard it. In some cases blocks can be put together to form a larger block. If
necessary, blocks can be disassembled if only a part of the composed block is needed.

More information on the implemented algorithm can be found here:

* [Paper describing the YATA conflict resolution algorithm](https://www.researchgate.net/publication/310212186_Near_Real-Time_Peer-to-Peer_Shared_Editing_on_Extensible_Data_Types)
* [Short overview of the Yjs optimizations](https://github.com/yjs/yjs/blob/main/README.md#yjs-crdt-algorithm)
* [Motivation for composable blocks](https://blog.kevinjahns.de/are-crdts-suitable-for-shared-editing/)
* [Internal documentation of the Yjs project](https://github.com/yjs/yjs/blob/main/INTERNALS.md)
* [Walkthrough of the Yjs codebase](https://youtu.be/0l5XgnQ6rB4)
