import y_py as Y

def transact(self,callback):
    with self.begin_transaction() as txn:
        return callback(txn)



def exchange_updates(docs):
    for d1 in docs:
        for d2 in docs:
            if d1 != d2:
                state_vector = Y.encode_state_vector(d1)
                diff = Y.encode_state_as_update(d2, state_vector)
                Y.apply_update(d1, diff)