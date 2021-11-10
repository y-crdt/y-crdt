from test_helper import exchange_updates
import pytest

import y_py as Y

def testInserts():
    d1 = Y.YDoc(1)
    assert d1.id == 1
    x = d1.get_array('test');

    d1.transact(lambda txn : x.insert(txn, 0, [1, 2.5, 'hello', ['world'], True]))
    d1.transact(lambda txn : x.push(txn, [{"key":'value'}]))

    expected = [1, 2.5, 'hello', ['world'], True, {"key":'value'}]

    value = d1.transact(lambda txn : x.to_json(txn))
    assert value == expected # TODO: Make this an arr cmp

    d2 = Y.YDoc(2)
    x = d2.get_array('test');

    exchange_updates([d1, d2])

    value = d2.transact(lambda txn : x.to_json(txn))
    assert value ==expected

def testInsertsNested():
    d1 = Y.YDoc()
    x = d1.get_array('test');

    nested = Y.YArray();
    d1.transact(lambda txn : nested.push(txn, ['world']))
    d1.transact(lambda txn : x.insert(txn, 0, [1, 2, nested, 3, 4]))
    d1.transact(lambda txn : nested.insert(txn, 0, ['hello']))

    expected = [1, 2, ['hello', 'world'], 3, 4]

    value = d1.transact(lambda txn : x.to_json(txn))
    assert value ==expected

    d2 = Y.YDoc()
    x = d2.get_array('test');

    exchange_updates([d1, d2])

    value = d2.transact(lambda txn : x.to_json(txn))
    assert value ==expected

def test_delete ():
    d1 = Y.YDoc(1)
    assert d1.id == 1
    x = d1.get_array('test')

    d1.transact(lambda txn : x.insert(txn, 0, [1, 2, ['hello', 'world'], True]))
    d1.transact(lambda txn : x.delete(txn, 1, 2))

    expected = [1, True]

    value = d1.transact(lambda txn : x.to_json(txn))
    assert value ==expected

    d2 = Y.YDoc(2)
    x = d2.get_array('test')

    exchange_updates([d1, d2])

    value = d2.transact(lambda txn : x.to_json(txn))
    assert value ==expected

def test_get():
    d1 = Y.YDoc()
    x = d1.get_array('test')

    d1.transact(lambda txn : x.insert(txn, 0, [1, 2, True]))
    d1.transact(lambda txn : x.insert(txn, 1, ['hello', 'world']));

    zeroed = d1.transact(lambda txn : x.get(txn, 0))
    first = d1.transact(lambda txn : x.get(txn, 1))
    second = d1.transact(lambda txn : x.get(txn, 2))
    third = d1.transact(lambda txn : x.get(txn, 3))
    fourth = d1.transact(lambda txn : x.get(txn, 4))

    assert zeroed == 1

    assert first == 'hello'

    assert second == 'world'

    assert third == 2

    assert fourth == True

    # t.fails(() => {
    #     // should fail because it's outside of the bounds
    #     d1.transact(lambda txn : x.get(txn, 5))
    # })

def test_iterator():
    d1 = Y.YDoc()
    x = d1.get_array('test')

    d1.transact(lambda txn : x.insert(txn, 0, [1, 2, 3]))
    assert x.length == 3

    txn = d1.begin_transaction()
    try:
        i = 1
        for v in x.values(txn):
            assert v == i
            i+=1
    finally:
        txn.free()