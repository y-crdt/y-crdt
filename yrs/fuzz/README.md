# yrs fuzz targets

Coverage-guided fuzz targets for the parts of yrs that process data received from other peers
(updates, state vectors, snapshots, y-sync protocol messages, awareness updates, sticky indexes and
`Any` values). Malformed input must never cause a panic, an abort, a hang or memory unsafety - only
errors.

Targets are built with [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer) and
AddressSanitizer, which requires a nightly toolchain:

```sh
cargo install cargo-fuzz
rustup toolchain install nightly
```

## Targets

| Target          | What is fuzzed                                                                                          |
|-----------------|---------------------------------------------------------------------------------------------------------|
| `update_v1`     | `Update::decode_v1`, applying the update to an empty document and to a document with existing state, then reading, encoding and editing it |
| `update_v2`     | same as above for `Update::decode_v2`                                                                   |
| `merge_diff`    | `merge_updates_v1/v2`, `diff_updates_v1/v2`, `encode_state_vector_from_update_v1/v2`                    |
| `state_vector`  | `StateVector` / `IdSet` decoding and computing document diffs from a remote state vector               |
| `snapshot`      | `Snapshot` decoding, `encode_state_from_snapshot` and `Text::diff_range` with a snapshot               |
| `sync_protocol` | y-sync `MessageReader` stream (sync steps, updates, awareness, auth) and `AwarenessUpdate`              |
| `sticky_index`  | `StickyIndex` decoding and resolving it against a document                                             |
| `any`           | lib0 `Any::decode` and `Any::from_json`                                                                 |

Shared helpers (seed document, reading/editing a document after an update was applied) live in
`src/lib.rs`.

## Running

From the `yrs/fuzz` directory:

```sh
# generate a seed corpus (valid payloads) into corpus/<target>
cargo run --example generate_corpus

# run a target (Ctrl+C to stop)
cargo +nightly fuzz run update_v1

# run in parallel, keep going after crashes and stop after an hour
cargo +nightly fuzz run update_v2 -- -fork=4 -ignore_crashes=1 -max_total_time=3600
```

Crashing inputs are saved into `artifacts/<target>/`. To reproduce one:

```sh
cargo +nightly fuzz run update_v1 artifacts/update_v1/crash-<hash>
```

To minimize a corpus or a crashing input use `cargo fuzz cmin <target>` and
`cargo fuzz tmin <target> <input>`.
