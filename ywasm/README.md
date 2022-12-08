# Ywasm

This project is a wrapper around [Yrs](../yrs/README.md) and targets Web Assembly bindings.

It's a library used on collaborative document editing using Conflict-free Replicated Data Types.
This enables to provide a shared document editing experience on a client devices without explicit requirement for hosting a single server - CRDTs can resolve potential update conflicts on their own with no central authority - as well as provide first-class offline editing capabilities, where document replicas are modified without having connection to each other, and then synchronize automatically once such connection is enabled.

## [Documentation](https://docs.rs/ywasm/)

## Example

```js
import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm';

YDoc.prototype.transact = callback => {
    const txn = this.beginTransaction()
    try {
        return callback(txn)
    } finally {
        txn.free()
    }
}

const doc = new YDoc()
const text = doc.getText('name')

// append text to our collaborative document
text.insert(txn, 0, 'hello world')

// simulate update with remote peer
const remoteDoc = new YDoc()
const remoteText = remoteDoc.getText('name')

// in order to exchange data with other documents 
// we first need to create a state vector
const remoteSV = encodeStateVector(remoteDoc)
// now compute a differential update based on remote document's state vector
const update = encodeStateAsUpdate(doc, remoteSV)
// both update and state vector are serializable, we can pass them over the wire
// now apply update to a remote document
applyUpdate(remoteDoc, update)

const str = remoteText.toString(txn)
console.log(str)
```