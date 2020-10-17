# Yrs
> Yjs port to Rust (WIP)

Yrs "wires" is a Rust port of the Yjs framework. The [Ywasm](https://github.com/yjs/Ywasm) project generates a wasm library based on this package.

## Run examples

```sh
cargo run --example [example-name]
```

## Run benchmarks

```sh
cargo bench
```

## Status

> :warning: Highly experimental. Feedback and involvement is welcome!

* Partially implemented the binary encoding protocol. Still need to port the rest of lib0/encoding.
* Only implements the Text type.
* No conflict resolution yet.

I'm currently mainly experimenting with designing a convenient API for the Yrs CRDT. I want to make it work very similar to Yjs, but contrary to Yjs you should always know what you get back (proper types, no duck typing).
