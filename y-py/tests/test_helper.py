import y_py as Y

def transact(self,callback):
    txn = self.beginTransaction()
    try:
        return callback(txn)
    finally:
        txn.commit()
        txn.free()


Y.YDoc.transact = transact

def exchange_updates(docs):
    for d1 in docs:
        for d2 in docs:
            if d1 != d2:
                state_vector = Y.encodeStateVector(d1)
                diff = Y.encodeStateAsUpdate(d2, state_vector)
                Y.applyUpdate(d1, diff)