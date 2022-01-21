from test_helper import exchange_updates
import unittest
import y_py as Y


def test_insert():
    d1 = Y.YDoc()
    root = d1.get_xml_element('test')
    with d1.begin_transaction() as txn:
        b = root.push_xml_text(txn)
        a = root.insert_xml_element(txn, 0, 'p')
        aa = a.push_xml_text(txn)

        aa.push(txn, 'hello')
        b.push(txn, 'world')

        s = root.to_string(txn)
    assert s == '<UNDEFINED><p>hello</p>world</UNDEFINED>'

def test_attributes():
    d1 = Y.YDoc()
    root = d1.get_xml_element('test')
    with d1.begin_transaction() as txn:
        root.set_attribute(txn, 'key1', 'value1')
        root.set_attribute(txn, 'key2', 'value2')

        actual = {}
        for key,value in root.attributes(txn):
            actual[key] = value
    assert actual == {
        "key1": 'value1',
        "key2": 'value2'
    }

    with d1.begin_transaction() as txn:
        root.remove_attribute(txn, 'key1')
        actual = {
            "key1": root.get_attribute(txn, 'key1'),
            "key2": root.get_attribute(txn, 'key2')
        }

    assert actual == {
        "key1": None,
        "key2": 'value2'
    }


def test_siblings():
    d1 = Y.YDoc()
    root = d1.get_xml_element('test')
    with d1.begin_transaction() as txn:
        b = root.push_xml_text(txn)
        a = root.insert_xml_element(txn, 0, 'p')
        aa = a.push_xml_text(txn)

        aa.push(txn, 'hello')
        b.push(txn, 'world')
        first = a
        assert first.prev_sibling(txn) == None

    with d1.begin_transaction() as txn:
        second = first.next_sibling(txn)
        s = second.to_string(txn)
        assert s == "world"
        assert second.nextSibling(txn) == None

    with d1.begin_transaction() as txn:
        actual = second.prev_sibling(txn).to_string(txn)
        expected = first.to_string(txn)
        assert actual == expected


def test_tree_walker():
    d1 = Y.YDoc()
    root = d1.get_xml_element('test')
    with d1.begin_transaction() as txn:
        b = root.push_xml_text(txn)
        a = root.insert_xml_element(txn, 0, 'p')
        aa = a.push_xml_text(txn)
        aa.push(txn, 'hello')
        b.push(txn, 'world')

    with d1.begin_transaction() as txn:
        actual = []
        for child in root.tree_walker(txn):
            actual.push(child.to_string(txn))

        expected = [
            '<p>hello</p>',
            'hello',
            'world'
        ]
        assert actual == expected

    with d1.begin_transaction() as txn:
        actual = []
        for child in root.tree_walker(txn):
            actual.push(child.to_string(txn))

        expected = [
            '<p>hello</p>',
            'hello',
            'world'
        ]
        assert actual == expected


def test_xml_text_observer():
    d1 = Y.YDoc()
    def get_value(x: Y.YXmlText) -> str:
        with d1.begin_transaction() as txn:
            return x.to_string(txn)
    x = d1.get_xml_text('test')
    target = None
    attributes = None
    delta = None

    def callback(e):
        nonlocal target
        nonlocal attributes
        nonlocal delta
        target = e.target
        attributes = e.keys
        delta = e.delta

    observer = x.observe(callback)

    # set initial attributes
    with d1.begin_transaction() as txn:
        x.set_attribute(txn, 'attr1', 'value1')
        x.set_attribute(txn, 'attr2', 'value2')

    assert get_value(target) == get_value(x)
    assert delta == []
    assert attributes == {
        "attr1": { "action": 'add', "newValue": 'value1' },
        "attr2": { "action": 'add', "newValue": 'value2' }
    }
    target = None
    attributes = None
    delta = None

    # update attributes
    with d1.begin_transaction() as txn:
        x.set_attribute(txn, 'attr1', 'value11')
        x.remove_attribute(txn, 'attr2')

    assert get_value(target) == get_value(x)
    assert delta == []
    assert attributes == {
        "attr1": { "action": 'update', "oldValue": 'value1', "newValue": 'value11' },
        "attr2": { "action": 'delete', "oldValue": 'value2' }
    }
    target = None
    attributes = None
    delta = None

    # insert initial data to an empty YText
    with d1.begin_transaction() as txn:
        x.insert(txn, 0, 'abcd')
    assert get_value(target), get_value(x)
    assert delta == [{"insert": "abcd"}] 
    assert attributes == {}
    target = None
    attributes = None
    delta = None
    # remove 2 chars from the middle
    with d1.begin_transaction() as txn:
        x.delete(txn, 1, 2)
    assert get_value(target) == get_value(x)
    assert delta == [{"retain":1}, {"delete": 2}]
    assert attributes == {}
    target = None
    attributes = None
    delta = None

    # insert item in the middle
    with d1.begin_transaction() as txn:
        x.insert(txn, 1, 'e')
    assert get_value(target) == get_value(x)
    assert delta == [{"retain":1}, {"insert": 'e'}]
    assert attributes == {}
    target = None
    attributes = None
    delta = None

    # free the observer and make sure that callback is no longer called
    del observer
    with d1.begin_transaction() as txn: 
        x.insert(txn, 1, 'fgh')
    assert target == None
    assert attributes == None
    assert delta == None

def test_xml_element_observer():
    d1 = Y.YDoc()
    def get_value(x: Y.YXmlElement) -> str:
        with d1.begin_transaction() as txn:
            return x.to_string(txn)

    x = d1.get_xml_element('test')
    target = None
    attributes = None
    nodes = None

    def callback(e):
        nonlocal target
        nonlocal attributes 
        nonlocal nodes
        target = e.target
        attributes = e.keys
        nodes = e.delta
    
    observer = x.observe(callback)

    # insert initial attributes
    with d1.begin_transaction() as txn:
        x.set_attribute(txn, 'attr1', 'value1')
        x.set_attribute(txn, 'attr2', 'value2')

    assert get_value(target) == get_value(x)
    assert nodes == []
    assert attributes == {
        "attr1": { "action": 'add', "newValue": 'value1' },
        "attr2": { "action": 'add', "newValue": 'value2' }
    }
    target = None
    attributes = None
    nodes = None

    # update attributes
    with d1.begin_transaction() as txn:
        x.set_attribute(txn, 'attr1', 'value11')
        x.remove_attribute(txn, 'attr2')

    assert get_value(target), get_value(x)
    assert nodes == []
    assert attributes == {
        "attr1": { "action": 'update', "oldValue": 'value1', "newValue": 'value11' },
        "attr2": { "action": 'delete', "oldValue": 'value2' }
    }

    target = None
    attributes = None
    nodes = None

    # add children
    with d1.begin_transaction() as txn:
        x.insert_xml_element(txn, 0, 'div')
        x.insert_xml_element(txn, 1, 'p')

    assert get_value(target) == get_value(x)
    assert len(nodes[0]["insert"]) == 2 # [{ insert: [div, p] }
    assert attributes ==  {}
    target = None
    attributes = None
    nodes = None

    # remove a child
    with d1.begin_transaction() as txn:
        x.delete(txn, 0, 1)
    assert get_value(target) == get_value(x)
    assert nodes == [{ "delete": 1 }]
    assert attributes == {}
    target = None
    attributes = None
    nodes = None

    # insert child again
    with d1.begin_transaction() as txn:
        txt = x.insert_xml_text(txn, x.length(txn))

    assert get_value(target) == get_value(x)
    assert nodes[0] == { "retain": 1 }
    assert nodes[1]["insert"] != None
    assert attributes ==  {}
    target = None
    attributes = None
    nodes = None

    # free the observer and make sure that callback is no longer called
    del observer
    with d1.begin_transaction() as txn:
        x.insert_xml_element(txn, 0, 'head')
    assert target == None
    assert nodes == None
    assert attributes == None
