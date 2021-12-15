# Y CRDT Ports

[![Join the chat at https://gitter.im/Yjs/y-crdt](https://badges.gitter.im/Yjs/y-crdt.svg)](https://gitter.im/Yjs/y-crdt?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)

> Yjs ports to other programming languages (WIP).

Yrs "wires" is a Rust port of the Yjs framework. The
[Ywasm](https://github.com/yjs/Ywasm) project generates a wasm library based on
this package.

## Status

> :warning: Highly experimental. Feedback and involvement is welcome!

* Partially implemented the binary encoding protocol. Still need to port the rest of lib0/encoding.
* Only implements the Text type.
* No conflict resolution yet.

I'm currently mainly experimenting with designing a convenient API for the Yrs CRDT. I want to make it work very similar to Yjs, but contrary to Yjs you should always know what you get back (proper types, no duck typing).

## Project Organization

The Yrs project is organized in a monorepo of components that work with each
other.

* **[./yrs](./yrs)** Contains the CRDT implementation (including shared types)
  that is the baseline for the other projects.
* **[./ywasm](./ywasm)** Maintains the wasm bindings and the npm module.

## Sponsors

[![NLNET](https://nlnet.nl/image/logo_nlnet.svg)](https://nlnet.nl/)