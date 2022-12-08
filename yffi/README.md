# Yrs C foreign function interface

This project is a wrapper around [Yrs](../yrs/README.md) and targets native interop with possible host languages.

It's a library used on collaborative document editing using Conflict-free Replicated Data Types.
This enables to provide a shared document editing experience on a client devices without explicit requirement for hosting a single server - CRDTs can resolve potential update conflicts on their own with no central authority - as well as provide first-class offline editing capabilities, where document replicas are modified without having connection to each other, and then synchronize automatically once such connection is enabled.

## [Documentation](https://docs.rs/yffi/0.9.3/yrs/)

It's also possible to read it straight from a generated [C header file](https://github.com/y-crdt/y-crdt/blob/main/tests-ffi/include/libyrs.h).

## Example

```c
#include <stdio.h>
#include "libyrs.h"

int main(void) {
    YDoc* doc = ydoc_new();
    YText* txt = ytext(doc, "name");
    YTransaction* txn = ydoc_write_transaction(doc);
    
    // append text to our collaborative document with no attributes
    ytext_insert(txt, txn, 0, "hello world", NULL);
    
    // simulate update with remote peer
    YDoc* remote_doc = ydoc_new();
    YText* remote_txt = ytext(remote_doc, "name");
    YTransaction* remote_txn = ydoc_write_transaction(remote_doc);
    
    // in order to exchange data with other documents
    // we first need to create a state vector
    int sv_length = 0;
    unsigned char* remote_sv = ytransaction_state_vector_v1(remote_txn, &sv_length);
    
    // now compute a differential update based on remote document's state vector
    int update_length = 0;
    unsigned char* update = ytransaction_state_diff_v1(txn, remote_sv, sv_length, &update_length);
    
    // release resources no longer in use in the rest of the example
    ybinary_destroy(remote_sv, sv_length);
    ytransaction_commit(txn);
    ydoc_destroy(doc);
    
    // both update and state vector are serializable, we can pass them over the wire
    // now apply update to a remote document
    int err_code = ytransaction_apply(remote_txn, update, update_length);
    if (0 != err_code) {
        // error occurred when trying to apply an update
        exit(err_code);
    }
    ybinary_destroy(update, update_length);
    
    // retrieve string from remote peer YText instance
    char* str = ytext_string(remote_txt, remote_txn);
    printf("%s", str);
    
    // release remaining resources
    ystring_destroy(str);
    ytransaction_commit(remote_txn);
    ydoc_destroy(remote_doc);
    
    return 0;
}
```