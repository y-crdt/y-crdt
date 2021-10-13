# Yrs C foreign function interface

This project is a wrapper around [Yrs](../yrs/README.md) and targets native interop with possible host languages.

It's a library used on collaborative document editing using Conflict-free Replicated Data Types.
This enables to provide a shared document editing experience on a client devices without explicit requirement for hosting a single server - CRDTs can resolve potential update conflicts on their own with no central authority - as well as provide first-class offline editing capabilities, where document replicas are modified without having connection to each other, and then synchronize automatically once such connection is enabled.

## [Documentation](https://docs.rs/yffi~~~~/)

It's also possible to read it straight from a generated [C header file](../tests-ffi/include/libyrs.h).

## Example

```c
#include <stdio.h>
#include "libyrs.h"

int main(void) {
    YDoc* doc = ydoc_new();
    YTransaction* txn = ytransaction_new(doc);
    YText* txt = ytext(txn, "name");
    
    // append text to our collaborative document
    ytext_insert(txt1, t1, 0, "hello world");
    
    // simulate update with remote peer
    YDoc* remote_doc = ydoc_new();
    YTransaction* remote_txn = ytransaction_new(remote_doc);
    YText* remote_txt = ytext(remote_txn, "name");
    
    // in order to exchange data with other documents
    // we first need to create a state vector
    int sv_length = 0;
    unsigned char* remote_sv = ytransaction_state_vector_v1(remote_txn, &sv_length);
    
    // now compute a differential update based on remote document's state vector
    int update_length = 0;
    unsigned char* update = ytransaction_state_diff_v1(txn, remote_sv, sv_length, &update_length);
    
    // release resources no longer in use in the rest of the example
    ybinary_destroy(remote_sv, sv_length);
    ytext_destroy(txt);
    ytransaction_commit(txn);
    ydoc_destroy(doc);
    
    // both update and state vector are serializable, we can pass them over the wire
    // now apply update to a remote document
    ytransaction_apply(remote_txn, update, update_length);
    ybinary_destroy(update, update_length);
    
    // retrieve string from remote peer YText instance
    char* str = ytext_string(remote_txt, remote_txn);
    printf("%s", str);
    
    // release remaining resources
    ystring_destroy(str);
    ytext_destroy(remote_txt);
    ytransaction_commit(remote_txn);
    ydoc_destroy(remote_doc);
    
    return 0;
}
```