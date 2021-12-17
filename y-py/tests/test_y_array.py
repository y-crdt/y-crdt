from test_helper import exchange_updates
import pytest

from y_py import YDoc, YArray

def test_inserts():
    d1 = YDoc(1)
    x = d1.get_array('test');

    with d1.begin_transaction() as txn:
        x.insert(txn, 0, [1, 2.5, 'hello', ['world'], True])
    
    with d1.begin_transaction() as txn:
        x.push(txn, [{"key":'value'}])

    expected = [1, 2.5, 'hello', ['world'], True, {"key":'value'}]

    value = d1.transact(lambda txn : x.to_json(txn))
    assert value == expected # TODO: Make this an arr cmp

    d2 = YDoc(2)
    x = d2.get_array('test');

    exchange_updates([d1, d2])

    value = d2.transact(lambda txn : x.to_json(txn))
    assert value ==expected

def test_inserts_nested():
    d1 = YDoc()
    x = d1.get_array('test')

    nested = YArray()
    d1.transact(lambda txn : nested.push(txn, ['world']))
    d1.transact(lambda txn : x.insert(txn, 0, [1, 2, nested, 3, 4]))
    d1.transact(lambda txn : nested.insert(txn, 0, ['hello']))

    expected = [1, 2, ['hello', 'world'], 3, 4]

    value = d1.transact(lambda txn : x.to_json(txn))
    assert value ==expected

    d2 = YDoc()
    x = d2.get_array('test');

    exchange_updates([d1, d2])

    value = d2.transact(lambda txn : x.to_json(txn))
    assert value ==expected

def test_delete():
    d1 = YDoc(1)
    assert d1.id == 1
    x = d1.get_array('test')

    d1.transact(lambda txn : x.insert(txn, 0, [1, 2, ['hello', 'world'], True]))
    d1.transact(lambda txn : x.delete(txn, 1, 2))

    expected = [1, True]

    value = d1.transact(lambda txn : x.to_json(txn))
    assert value ==expected

    d2 = YDoc(2)
    x = d2.get_array('test')

    exchange_updates([d1, d2])

    value = d2.transact(lambda txn : x.to_json(txn))
    assert value ==expected

def test_get():
    d1 = YDoc()
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

    with pytest.raises(IndexError):
        x = d1.transact(lambda txn : x.get(txn, 20))

def test_iterator():
    d1 = YDoc()
    x = d1.get_array('test')

    d1.transact(lambda txn : x.insert(txn, 0, [1, 2, 3]))
    assert x.length == 3

    with d1.begin_transaction() as txn:
        i = 1
        for v in x.values(txn):
            assert v == i
            i+=1

def test_borrow_mut_edge_case():
    """
    Tests for incorrect overlap in successive mutable borrows of YTransaction and YArray.
    """
    doc = YDoc()
    arr = doc.get_array('test')
    with doc.begin_transaction() as txn:
        arr.insert(txn, 0, [1,2,3])
    
    # Ensure multiple transactions can be called in a row with the same variable name `txn`
    with doc.begin_transaction() as txn:
        # Ensure that multiple mutable borrow functions can be called in a tight loop
        for i in range(2000):
            arr.insert(txn, [1,2,3])
            arr.delete(txn, 0, 3)
