import { exchangeUpdates } from './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {t.TestCase} tc
 */
export const testInsert = tc => {
    const d1 = new Y.YDoc()
    const root = d1.getXmlFragment('test')
    d1.transact(txn => {
        let b = root.pushXmlText(txn)
        let a = root.insertXmlElement(0, 'p', txn)
        let aa = a.pushXmlText(txn)

        aa.push('hello', null, txn)
        b.push('world', null, txn)
    })

    const s = root.toString()

    t.compareStrings(s, '<p>hello</p>world')
}

/**
 * @param {t.TestCase} tc
 */
export const testAttributes = tc => {
    const d1 = new Y.YDoc()
    const root = d1.getXmlFragment('test')
    const xml = root.insertXmlElement(0, 'div')
    let actual = d1.transact(txn => {
        xml.setAttribute('key1', 'value1', txn)
        xml.setAttribute('key2', 'value2', txn)

        let obj = {}
        let attrs = xml.attributes(txn);
        for (let key in attrs) {
            // we test iterator here
            obj[key] = attrs[key]
        }
        return obj
    });

    t.compareObjects(actual, {
        key1: 'value1',
        key2: 'value2'
    })

    actual = d1.transact(txn => {
        xml.removeAttribute('key1', txn)
        return {
            key1: xml.getAttribute('key1', txn),
            key2: xml.getAttribute('key2', txn)
        }
    })

    t.compareObjects(actual, {
        key1: undefined,
        key2: 'value2'
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testSiblings = tc => {
    const d1 = new Y.YDoc()
    const root = d1.getXmlFragment('test')
    const first = d1.transact(txn => {
        let b = root.pushXmlText(txn)
        let a = root.insertXmlElement(0, 'p', txn)
        let aa = a.pushXmlText(txn)

        aa.push('hello', null, txn)
        b.push('world', null, txn)

        return a
    })

    t.compare(first.prevSibling(), undefined)

    let second = first.nextSibling()
    let s = second.toString()
    t.compare(s, 'world')
    t.compare(second.nextSibling(), undefined)

    let actual = second.prevSibling().toString()
    let expected = first.toString()
    t.compare(actual, expected)
}

/**
 * @param {t.TestCase} tc
 */
export const testTreeWalker = tc => {
    const d1 = new Y.YDoc()
    const root = d1.getXmlFragment('test')
    d1.transact(txn => {
        let b = root.pushXmlText(txn)
        let a = root.insertXmlElement(0, 'p', txn)
        let aa = a.pushXmlText(txn)

        aa.push('hello', null, txn)
        b.push('world', null, txn)
    })

    const actual = []
    d1.transact(txn => {
        for (let child of root.treeWalker(txn)) {
            actual.push(child.toString(txn))
        }
    })

    const expected = [
        '<p>hello</p>',
        'hello',
        'world'
    ]
    t.compareArrays(actual, expected)
}

/**
 * @param {t.TestCase} tc
 */
export const testXmlTextObserver = tc => {
    const d1 = new Y.YDoc()
    const f = d1.getXmlFragment('test');
    const x = f.insertXmlText(0)
    let target = null
    let attributes = null
    let delta = null
    let origin = null
    let observer = x.observe(e => {
        target = e.target
        attributes = e.keys
        delta = e.delta
        origin = e.origin
    })

    // set initial attributes
    d1.transact(txn => {
        x.setAttribute('attr1', 'value1', txn)
        x.setAttribute('attr2', 'value2', txn)
    }, 'TEST_ORIGIN')
    t.compare(target.toString(), x.toString())
    t.compare(delta, [])
    t.compare(attributes, {
        attr1: { action: 'add', newValue: 'value1' },
        attr2: { action: 'add', newValue: 'value2' }
    })
    t.compare(origin, 'TEST_ORIGIN')
    target = null
    attributes = null
    delta = null

    // update attributes
    d1.transact(txn => {
        x.setAttribute('attr1', 'value11', txn)
        x.removeAttribute('attr2', txn)
    }, 'TEST_ORIGIN2')
    t.compare(target.toString(), x.toString())
    t.compare(delta, [])
    t.compare(attributes, {
        attr1: { action: 'update', oldValue: 'value1', newValue: 'value11' },
        attr2: { action: 'delete', oldValue: 'value2' }
    })
    t.compare(origin, 'TEST_ORIGIN2')
    target = null
    attributes = null
    delta = null

    // insert initial data to an empty YText
    x.insert(0, 'abcd')
    t.compare(target.toString(), x.toString())
    t.compare(delta, [{insert: 'abcd'}])
    t.compare(attributes, {})
    target = null
    attributes = null
    delta = null

    // remove 2 chars from the middle
    x.delete(1, 2)
    t.compare(target.toString(), x.toString())
    t.compare(delta, [{retain:1}, {delete: 2}])
    t.compare(attributes, {})
    target = null
    attributes = null
    delta = null

    // insert new item in the middle
    x.insert(1, 'e')
    t.compare(target.toString(), x.toString())
    t.compare(delta, [{retain:1}, {insert: 'e'}])
    t.compare(attributes, {})
    target = null
    attributes = null
    delta = null

    // free the observer and make sure that callback is no longer called
    observer.free()
    x.insert(1, 'fgh')
    t.compare(target, null)
    t.compare(attributes, null)
    t.compare(delta, null)
}
/**
 * @param {t.TestCase} tc
 */
export const testXmlElementObserver = tc => {
    const d1 = new Y.YDoc()
    const f = d1.getXmlFragment('test');
    const x = f.insertXmlElement(0,'div')
    let target = null
    let attributes = null
    let nodes = null
    let observer = x.observe(e => {
        target = e.target
        attributes = e.keys
        nodes = e.delta
    })

    // insert initial attributes
    d1.transact(txn => {
        x.setAttribute('attr1', 'value1', txn)
        x.setAttribute('attr2', 'value2', txn)
    })
    t.compare(target.toString(), x.toString())
    t.compare(nodes, [])
    t.compare(attributes, {
        attr1: { action: 'add', newValue: 'value1' },
        attr2: { action: 'add', newValue: 'value2' }
    })
    target = null
    attributes = null
    nodes = null

    // update attributes
    d1.transact(txn => {
        x.setAttribute('attr1', 'value11', txn)
        x.removeAttribute('attr2', txn)
    })
    t.compare(target.toString(), x.toString())
    t.compare(nodes, [])
    t.compare(attributes, {
        attr1: { action: 'update', oldValue: 'value1', newValue: 'value11' },
        attr2: { action: 'delete', oldValue: 'value2' }
    })
    target = null
    attributes = null
    nodes = null

    // add children
    d1.transact(txn => {
        x.insertXmlElement(0, 'div', txn)
        x.insertXmlElement(1, 'p', txn)
    })
    t.compare(target.toString(), x.toString())
    t.compare(nodes[0].insert.length, 2) // [{ insert: [div, p] }]
    t.compare(attributes,  {})
    target = null
    attributes = null
    nodes = null

    // remove a child
    x.delete(0, 1)
    t.compare(target.toString(), x.toString())
    t.compare(nodes, [{ delete: 1 }])
    t.compare(attributes, {})
    target = null
    attributes = null
    nodes = null

    // insert child again
    let txt = x.insertXmlText(x.length())
    t.compare(target.toString(), x.toString())
    t.compare(nodes[0], { retain: 1 });
    t.assert(nodes[1].insert != null)
    t.compare(attributes,  {})
    target = null
    attributes = null
    nodes = null

    // free the observer and make sure that callback is no longer called
    observer.free()
    x.insertXmlElement(0, 'head')
    t.compare(target, null)
    t.compare(nodes, null)
    t.compare(attributes, null)
}