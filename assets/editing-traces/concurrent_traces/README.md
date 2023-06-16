# Concurrent editing traces

This folder contains concurrent editing traces. Concurrent editing traces are made when multiple users concurrently, collaboratively write a text document together.

These are (obviously) useful for benchmarking CRDTs - since the whole point of a text based CRDT is to allow users to do this sort of thing.

These traces are made of a series of *transactions*. In each transaction, one user (`agent` field) made some set of edits (`patches` field) to bring the document from a starting state (specified by `parents`) to some other state.

The patches work the same as they do in the sequential traces. But these data sets are quite complex to replay because of the `parents` field. The parents field contains indexes of other (previous) items in the list of transactions.

If the parents field contains:

- Nothing (`[]`), the document state before this transaction is an empty string.
- A single item (`[some_idx]`), the document state before this transaction is the same as the document state *after* the transaction identified by the index
- Multiple items (`[a, b]`), the parent transactions are first merged (using any algorithm). And *then* this transaction is applied.

None of the data sets here have any concurrent inserts at the same location in the document. As a result, any text based CRDT or OT system should be able to replay these histories and end up with the same resulting document.


## How do I use these data sets with my CRDT?

The easiest way to use these data sets is to convert them to your CRDT's internal format. This can be done somewhat inefficiently in about 100 LOC so long as your CRDT has some equivalent of the following methods:

```
crdt.fork() // Clone the CRDT's entire structure, in memory
crdt.localReplace(position, num_deleted_chars, inserted_string)
crdt.mergeFrom(otherCRDT)
```

Once you've converted these traces to your CRDT's format, you can replay them using whatever methods you like.

Here is a version of this conversion script written for automerge:

https://github.com/josephg/automerge-converter/blob/master/src/main.rs


## Example data set:

```json
{
  "kind": "concurrent",
  "endContent": "yoooo ho ho\n",
  "numAgents": 2,
  "txns": [
    {
      "parents": [],
      "numChildren": 1,
      "agent": 0,
      "patches": [
        [ 0, 0, "hi there\n" ]
      ]
    },
    {
      "parents": [ 0 ],
      "numChildren": 1,
      "agent": 0,
      "patches": [
        [ 0, 8, "" ],
        [ 0, 0, "yoooo" ]
      ]
    },
    {
      "parents": [ 1 ],
      "numChildren": 0,
      "agent": 1,
      "patches": [
        [ 5, 0, " ho ho" ]
      ]
    }
  ]
}
```


## Data format

Each file in this folder is a (gzipped) JSON document with the same format. The files contain JSON objects with the following top level fields:

- `kind` (string): "concurrent"
- `endContent` (string): The expected contents of the document after all changes have been applied. This exists mostly as a checksum / validation that the trace has been replayed correctly.
- `numAgents` (integer): The number of user agents in this editing trace. User agents (below) will be identified using the integers from `0` to `numAgents - 1`.
- `txns`: A list of "transaction" objects. Each object describes 1 or more patches, made by some peer, at some point in time. (Caveat: the last transaction may contain no patches)

Transactions (*txns*) have the following fields:

- `parents`: This names the index of other items in the txn list that this transaction happened causally *after*. (Using 0-based indexing).
  - If the parents list is empty, the item comes after "root" (the start of time). The document always starts as the empty string (`""`) in this state. Only the first item in the list of transactions will have an empty parents list.
  - If the list contains 2 or more items, the state named by those items is merged before the operations contained in this transaction are applied. The list of parents must always be minimal. (All items in the parents list must be concurrent with all other items in the parents list).
- `patches`: A list of patches made, in sequence, to the document. The format here is the same format as patches in the sequential traces folder. Each item in this array is a triple of (*position*, *num characters deleted*, *inserted string*). The position names the unicode codepoint offset into the document where this edit took place. (I may publish ascii-only variants of editing traces as well to make this easier to interpret). If there are multiple patches in a transaction, they are applied in sequence. (Each patch assumes all previous patches in the transaction have already been applied).
- `agent`: This is an integer ID of the user agent which made this transaction. These are in the range from `0..numAgents` (non-inclusive).
- `numChildren`: The number of other (later) txns which contain this txn in their parents list. This is included for convenience - but it can be trivially recomputed.

### Rules

All editing traces listed here maintain some invariants to make processing / replaying them easier:

- To make the trace replayable by multiple CRDTs with different ordering rules, its invalid for 2 users to ever concurrently insert content at the same location in the document. This obviously makes these data sets unsuitable for correctness testing, but I think this is fine in benchmark data because concurrent inserts at the same location in a document are quite rare in practice. Its rare enough that it shouldn't affect benchmark results so long as the behaviour when concurrent inserts do land at the same time, performance isn't pathological.
- Parents:
  - Transactions are chronologically ordered. (Well, as much as possible) So for each transaction, all of the parents have indexes which point earlier in the list.
  - The first transaction must have no parents. The first transaction must be the only transaction with no parents.
  - The parents of each item must all be mutually concurrent
  - There are no "dangling" transactions. The last transaction transitively "contains" (comes after) all other transactions in the trace.
- All transactions except the last must have a non-empty list of patches.
- All the changes from each user agent are fully ordered. Its invalid for a single user agent to make concurrent changes.


## Data sets

I really want more data sets in this folder! Now its easier to make data sets like this, I'll collect a few more over time.

#### friendsforever.json.gz

This document is a description & emotional debrief after watching an episode of *Friends* in 2023. The editing trace is quite short - there are only 26k inserts + deletes, to create a 21kb document. The document is pure ASCII. The 2 users were typing concurrently into the document with 1 second of (artificial) network latency. There are nearly 4000 transactions in this document.
