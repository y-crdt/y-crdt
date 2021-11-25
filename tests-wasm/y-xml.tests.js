import { exchangeUpdates } from './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {t.TestCase} tc
 */
export const testInsert = tc => {
    const d1 = new Y.YDoc()
    const root = d1.getXmlElement('test')
    const s = d1.transact(txn => {
        let b = root.pushXmlText(txn)
        let a = root.insertXmlElement(txn, 0, 'p')
        let aa = a.pushXmlText(txn)

        aa.push(txn, 'hello')
        b.push(txn, 'world')

        return root.toString(txn)
    })

    t.compareStrings(s, '<UNDEFINED><p>hello</p>world</UNDEFINED>')
}

/**
 * @param {t.TestCase} tc
 */
export const testAttributes = tc => {
    const d1 = new Y.YDoc()
    const root = d1.getXmlElement('test')
    var actual = d1.transact(txn => {
        root.setAttribute(txn, 'key1', 'value1')
        root.setAttribute(txn, 'key2', 'value2')

        let obj = {}
        for (let [key,value] of root.attributes(txn)) {
            obj[key] = value
        }
        return obj
    })

    t.compareObjects(actual, {
        key1: 'value1',
        key2: 'value2'
    })

    actual = d1.transact(txn => {
        root.removeAttribute(txn, 'key1')
        return {
            key1: root.getAttribute(txn, 'key1'),
            key2: root.getAttribute(txn, 'key2')
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
    const root = d1.getXmlElement('test')
    const first = d1.transact(txn => {
        let b = root.pushXmlText(txn)
        let a = root.insertXmlElement(txn, 0, 'p')
        let aa = a.pushXmlText(txn)

        aa.push(txn, 'hello')
        b.push(txn, 'world')

        return a
    })

    t.compare(d1.transact(txn => first.prevSibling(txn)), undefined)

    let second = d1.transact(txn => first.nextSibling(txn))
    let s = d1.transact(txn => second.toString(txn))
    t.compare(s, 'world')
    t.compare(d1.transact(txn => second.nextSibling(txn)), undefined)

    let actual = d1.transact(txn => second.prevSibling(txn).toString(txn))
    let expected = d1.transact(txn => first.toString(txn))
    t.compare(actual, expected)
}

/**
 * @param {t.TestCase} tc
 */
export const testTreeWalker = tc => {
    const d1 = new Y.YDoc()
    const root = d1.getXmlElement('test')
    d1.transact(txn => {
        let b = root.pushXmlText(txn)
        let a = root.insertXmlElement(txn, 0, 'p')
        let aa = a.pushXmlText(txn)

        aa.push(txn, 'hello')
        b.push(txn, 'world')
    })

    const txn = d1.beginTransaction()
    try {
        const actual = []
        for (let child of root.treeWalker(txn)) {
            actual.push(child.toString(txn))
        }

        const expected = [
            '<p>hello</p>',
            'hello',
            'world'
        ]
        t.compareArrays(actual, expected)
    } finally {
        txn.free()
    }
}

/**
 * @param {t.TestCase} tc
 */
export const testXmlTextObserver = tc => {
    const d1 = new Y.YDoc()
    /**
     * @param {Y.YXmlText} tc
     */
    const getValue = (x) => d1.transact(txn => x.toString(txn))
    const x = d1.getXmlText('test')
    let target = null
    let attributes = null
    let delta = null
    let observer = x.observe(e => {
        target = e.target
        attributes = e.keys
        delta = e.delta
    })

    // set initial attributes
    d1.transact(txn => {
        x.setAttribute(txn, 'attr1', 'value1')
        x.setAttribute(txn, 'attr2', 'value2')
    })
    t.compare(getValue(target), getValue(x))
    t.compare(delta, [])
    t.compare(attributes, {
        attr1: { action: 'add', newValue: 'value1' },
        attr2: { action: 'add', newValue: 'value2' }
    })
    target = null
    attributes = null
    delta = null

    // update attributes
    d1.transact(txn => {
        x.setAttribute(txn, 'attr1', 'value11')
        x.removeAttribute(txn, 'attr2')
    })
    t.compare(getValue(target), getValue(x))
    t.compare(delta, [])
    t.compare(attributes, {
        attr1: { action: 'update', oldValue: 'value1', newValue: 'value11' },
        attr2: { action: 'delete', oldValue: 'value2' }
    })
    target = null
    attributes = null
    delta = null

    // insert initial data to an empty YText
    d1.transact(txn => x.insert(txn, 0, 'abcd'))
    t.compare(getValue(target), getValue(x))
    t.compare(delta, [{insert: ['a','b','c','d']}])
    t.compare(attributes, {})
    target = null
    attributes = null
    delta = null

    // remove 2 chars from the middle
    d1.transact(txn => x.delete(txn, 1, 2))
    t.compare(getValue(target), getValue(x))
    t.compare(delta, [{retain:1}, {delete: 2}])
    t.compare(attributes, {})
    target = null
    attributes = null
    delta = null

    // insert new item in the middle
    d1.transact(txn => x.insert(txn, 1, 'e'))
    t.compare(getValue(target), getValue(x))
    t.compare(delta, [{retain:1}, {insert: ['e']}])
    t.compare(attributes, {})
    target = null
    attributes = null
    delta = null

    // free the observer and make sure that callback is no longer called
    observer.free()
    d1.transact(txn => x.insert(txn, 1, 'fgh'))
    t.compare(target, null)
    t.compare(attributes, null)
    t.compare(delta, null)
}
/**
 * @param {t.TestCase} tc
 */
export const testXmlElementObserver = tc => {
    const d1 = new Y.YDoc()
    /**
     * @param {Y.YXmlElement} tc
     */
    const getValue = (x) => d1.transact(txn => x.toString(txn))
    const x = d1.getXmlElement('test')
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
        x.setAttribute(txn, 'attr1', 'value1')
        x.setAttribute(txn, 'attr2', 'value2')
    })
    t.compare(getValue(target), getValue(x))
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
        x.setAttribute(txn, 'attr1', 'value11')
        x.removeAttribute(txn, 'attr2')
    })
    t.compare(getValue(target), getValue(x))
    t.compare(nodes, [])
    t.compare(attributes, {
        attr1: { action: 'update', oldValue: 'value1', newValue: 'value11' },
        attr2: { action: 'delete', oldValue: 'value2' }
    })
    target = null
    attributes = null
    nodes = null

    // add children
    let [div,p] = d1.transact(txn => {
        let div = x.insertXmlElement(txn, 0, 'div')
        let p = x.insertXmlElement(txn, 1, 'p')
        return [div, p]
    })
    t.compare(getValue(target), getValue(x))
    t.compare(nodes, [{ insert: [div, p] }])
    t.compare(attributes,  {})
    target = null
    attributes = null
    nodes = null

    // remove a child
    d1.transact(txn => x.delete(txn, 0, 1))
    t.compare(getValue(target), getValue(x))
    t.compare(nodes, [{ delete: 1 }])
    t.compare(attributes,  {})
    target = null
    attributes = null
    nodes = null

    // insert child again
    let txt = d1.transact(txn => x.insertXmlText(txn, x.length(txn)))
    t.compare(getValue(target), getValue(x))
    t.compare(nodes, [
        { retain: 1 },
        { insert: [txt] }
    ])
    t.compare(attributes,  {})
    target = null
    attributes = null
    nodes = null

    // free the observer and make sure that callback is no longer called
    observer.free()
    d1.transact(txn => x.insert(txn, 1, [6]))
    t.compare(target, null)
    t.compare(nodes, null)
    t.compare(attributes, null)
}