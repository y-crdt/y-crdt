# Y CRDT
<p align="center">
  <img src="logo-yrs.svg" width="200"/>
</p>

A collection of Rust libraries oriented around implementing [Yjs](https://yjs.dev/) algorithm and protocol with cross-language and cross-platform support in mind. It aims to maintain behavior and binary protocol compatibility with Yjs, therefore projects using Yjs/Yrs should be able to interoperate with each other.

Project organization:

- **lib0** is a serialization library used for efficient (and fairly fast) data exchange.
- **yrs** (read: *wires*) is a core Rust library, a foundation stone for other projects.
- **yffi** (read: *wifi*) is a wrapper around *yrs* used to provide a native C foreign function interface. See also: [C header file](https://github.com/y-crdt/y-crdt/blob/main/tests-ffi/include/libyrs.h).
- **ywasm** is a wrapper around *yrs* that targets WebAssembly and JavaScript API.

Other projects using *yrs*:

- [ypy](https://github.com/y-crdt/ypy) - Python bindings.
- [yrb](https://github.com/y-crdt/yrb) - Ruby bindings.

## Feature parity with Yjs project

-  Supported collaborative types:
  - [x] Text
    - [x] text insertion (with variable offsets including configurable UTF-8, UTF-16 and UTF-32 mappings)
    - [x] embedded elements insertion
    - [x] insertion of formatting attributes
    - [x] observe events and deltas
  - [x] Map
    - [x] insertion, update and removal of primitive JSON-like elements
    - [x] recursive insertion, update and removal of other collaborative elements of any type
    - [x] observe events and deltas
    - [x] deep observe events bubbling up from nested collections
  - [x] Array
    - [x] insertion and removal of primitive JSON-like elements
    - [x] recursive insertion of other collaborative elements of any type
    - [x] observe events and deltas
    - [x] deep observe events bubbling up from nested collections
    - [x] move index positions
  - [x] XmlElement
    - [x] insertion, update and removal of XML attributes
    - [x] insertion, update and removal of XML children nodes
    - [x] observe events and deltas
    - [x] deep observe events bubbling up from nested collections
  - [x] XmlText
    - [x] insertion, update and removal of XML attributes
    - [x] text insertion (with variable offsets including configurable UTF-8, UTF-16 and UTF-32 mappings)
    - [x] observe events and deltas
  - [x] XmlFragment
  - [x] XmlHook (*deprecated*)
  - [x] Subdocuments
  - [x] Transaction origin
  - [x] Undo/redo manager
- Encoding formats:
  - [x] lib0 v1 encoding
  - [x] lib0 v2 encoding
- Transaction events:
  - [x] on event update
  - [x] on after transaction

## Maintainers

- [Bartosz Sypytkowski](https://github.com/Horusiath)
- [Kevin Jahns](https://github.com/dmonad)
- [John Waidhofer](https://github.com/Waidhoferj)

## Sponsors

[![NLNET](https://nlnet.nl/image/logo_nlnet.svg)](https://nlnet.nl/)

[![Ably](https://ably.com/assets/ably_ui/core/images/ably-logo-ad51bb21f40afd34a70df857594d6b7b84f6ceca0518f1d4d94e2b9579486351.png)](https://ably.com/)