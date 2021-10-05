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