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

## Feature parity among projects

|                                         |                  yjs <br/>(13.6)                  |               yrs<br/>(0.18)               |                   ywasm<br/>(0.18)                    | yffi<br/>(0.18) |                  y-rb<br/>(0.5)                  |                y-py<br/>(0.6)                | ydotnet<br/>(0.4) | yswift<br/>(0.2) |
| --------------------------------------- | :-----------------------------------------------: | :----------------------------------------: | :---------------------------------------------------: | :-------------: | :----------------------------------------------: | :------------------------------------------: | :---------------: | :--------------: |
| YText: insert/delete                    |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x2705;                     |                   &#x2705;                   |     &#x2705;      |     &#x2705;     |
| YText: formatting attributes and deltas |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x2705;                     |                   &#x2705;                   |     &#x2705;      |     &#x2705;     |
| YText: embeded elements                 |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x2705;                     |                   &#x2705;                   |     &#x2705;      |     &#x2705;     |
| YMap: update/delete                     |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x2705;                     |                   &#x2705;                   |     &#x2705;      |     &#x2705;     |
| YMap: weak links                        | &#x2705; <br/> <small>(weak-links branch)</small> |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x274C;                     |                   &#x274C;                   |     &#x274C;      |     &#x274C;     |
| YArray: insert/delete                   |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x2705;                     |                   &#x2705;                   |     &#x2705;      |     &#x2705;     |
| YArray & YText quotations               | &#x2705; <br/> <small>(weak links branch)</small> |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x274C;                     |                   &#x274C;                   |     &#x274C;      |     &#x274C;     |
| YArray: move                            |    &#x2705; <br/> <small>(move branch)</small>    |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x274C;                     |                   &#x2705;                   |     &#x2705;      |     &#x274C;     |
| XML Element, Fragment and Text          |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x2705;                     |                   &#x2705;                   |     &#x2705;      |     &#x274C;     |
| Sub-documents                           |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x274C;                     |                   &#x274C;                   |     &#x2705;      |     &#x274C;     |
| Shared collections: observers           |                     &#x2705;                      |                  &#x2705;                  | &#x2705; <br/> <small>(incompatible with yjs)</small> |    &#x2705;     |                     &#x2705;                     |                   &#x2705;                   |     &#x2705;      |     &#x2705;     |
| Shared collections: recursive nesting   |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x274C;                     |                   &#x274C;                   |     &#x274C;      |     &#x274C;     |
| Document observers                      |                     &#x2705;                      |                  &#x2705;                  | &#x2705; <br/> <small>(incompatible with yjs)</small> |    &#x2705;     |                     &#x2705;                     |                   &#x2705;                   |     &#x2705;      |                  |
| Transaction: origins                    |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x274C;                     |                   &#x274C;                   |     &#x2705;      |     &#x2705;     |
| Snapshots                               |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x274C;                     |                   &#x274C;                   |     &#x274C;      |     &#x274C;     |
| Sticky indexes                          |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x274C;                     |                   &#x274C;                   |     &#x2705;      |     &#x274C;     |
| Undo Manager                            |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x2705;     |                     &#x274C;                     |                   &#x274C;                   |     &#x2705;      |     &#x2705;     |
| Awareness                               |                     &#x2705;                      |                  &#x2705;                  |                       &#x2705;                        |    &#x274C;     |                     &#x2705;                     |                   &#x274C;                   |     &#x2705;      |     &#x274C;     |
| Network provider: WebSockets            |    &#x2705; <br/> <small>(y-websocket)</small>    |  &#x2705; <br/> <small>(yrs-warp)</small>  |                       &#x274C;                        |    &#x274C;     | &#x2705; <br/> <small>(y-rb_actioncable)</small> | &#x2705; <br/><small>(ypy-websocket)</small> |     &#x2705;      |     &#x274C;     |
| Network provider: WebRTC                |     &#x2705; <br/> <small>(y-webrtc)</small>      | &#x2705; <br/> <small>(yrs-webrtc)</small> |                       &#x274C;                        |    &#x274C;     |                     &#x274C;                     |                   &#x274C;                   |     &#x274C;      |     &#x274C;     |

## Maintainers

- [Bartosz Sypytkowski](https://github.com/Horusiath)
- [Kevin Jahns](https://github.com/dmonad)
- [John Waidhofer](https://github.com/Waidhoferj)

## Sponsors

[![NLNET](https://nlnet.nl/image/logo_nlnet.svg)](https://nlnet.nl/)

[![Ably](https://voltaire.ably.com/static/ably-logo-46433d9937b94509fc62ef6dd6d94ff1.png)](https://ably.com/)

<a href="https://www.appflowy.io/"><img src="https://raw.githubusercontent.com/AppFlowy-IO/AppFlowy/main/doc/imgs/appflowy-logo-white.svg" height="75px"/></a>
