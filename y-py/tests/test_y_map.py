from test_helper import exchange_updates
import y_py as Y

def test_set():
    d1 = Y.YDoc()
    x = d1.get_map('test')

    value = d1.transact(lambda txn : x.get(txn, 'key'))
    assert value == None

    d1.transact(lambda txn : x.set(txn, 'key', 'value1'))
    value = d1.transact(lambda txn : x.get(txn, 'key'))
    assert value == 'value1'

    d1.transact(lambda txn : x.set(txn, 'key', 'value2'))
    value = d1.transact(lambda txn : x.get(txn, 'key'))
    assert value == "value2"

def test_set_nested():
    d1 = Y.YDoc()
    x = d1.get_map('test')
    nested = Y.YMap({ "a": 'A' })

    # check out to_json(), setting a nested map in set(), adding to an integrated value

    d1.transact(lambda txn : x.set(txn, 'key', nested))
    d1.transact(lambda txn : nested.set(txn, 'b', 'B'))

    json = d1.transact(lambda txn : x.to_json(txn))
    # TODO: Make this a deep diff
    assert json == {
        "key": {
            "a": 'A',
            "b": 'B'
        }
    }


def test_delete():
    d1 = Y.YDoc()
    x = d1.get_map('test')

    d1.transact(lambda txn : x.set(txn, 'key', 'value1'))
    len = d1.transact(lambda txn : x.length(txn))
    value = d1.transact(lambda txn : x.get(txn, 'key'))
    assert len == 1
    assert value == "value1"
    # TODO: Get length with __len__()
    d1.transact(lambda txn : x.delete(txn, 'key'))
    len = d1.transact(lambda txn : x.length(txn))
    value = d1.transact(lambda txn : x.get(txn, 'key'))
    assert len == 0
    assert value == None

    d1.transact(lambda txn : x.set(txn, 'key', 'value2'))
    len = d1.transact(lambda txn : x.length(txn))
    value = d1.transact(lambda txn : x.get(txn, 'key'))
    assert len == 1
    assert value == "value2"


def test_iterator():
    d1 = Y.YDoc()
    x = d1.get_map('test')

    def test(txn):
        x.set(txn, 'a', 1)
        x.set(txn, 'b', 2)
        x.set(txn, 'c', 3)
        expected = {
            'a': 1,
            'b': 2,
            'c': 3
        }
        for (key, val) in x.entries(txn):
            v = expected[key]
            assert val == v
            del expected[key]
    d1.transact(test)
    
