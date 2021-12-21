import pytest
from test_helper import exchange_updates

import y_py as Y


def test_inserts():
    d1 = Y.YDoc()
    x = d1.get_text('test')

    d1.transact(lambda txn : x.push(txn, "hello "))
    d1.transact(lambda txn : x.push(txn, "world!"))
    expected = "hello world!"
    value = d1.transact(lambda txn : x.to_string(txn))
    assert value == expected

    d2 = Y.YDoc(2)
    x = d2.get_text('test')

    exchange_updates([d1, d2])

    value = d2.transact(lambda txn : x.to_string(txn))
    assert value == expected

def test_deletes():
    d1 = Y.YDoc()
    x = d1.get_text('test')

    d1.transact(lambda txn : x.push(txn, "hello world!"))

    assert x.length == 12
    d1.transact(lambda txn : x.delete(txn, 5, 6))
    assert x.length == 6
    d1.transact(lambda txn : x.insert(txn, 5, " Yrs"))
    assert x.length == 10

    expected = "hello Yrs!"

    value = d1.transact(lambda txn : x.to_string(txn))
    assert value == expected

    d2 = Y.YDoc(2)
    x = d2.get_text('test')

    exchange_updates([d1, d2])

    value = d2.transact(lambda txn : x.to_string(txn))
    assert value == expected