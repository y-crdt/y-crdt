import { exchangeUpdates } from './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {t.TestCase} tc
 */
export const testSet = tc => {
    const d1 = new Y.YDoc()
    const x = d1.getMap('test')

    var value = d1.transact(txn => x.get(txn, 'key'))
    t.compare(value, undefined)

    d1.transact(txn => x.set(txn, 'key', 'value1'))
    value = d1.transact(txn => x.get(txn, 'key'))
    t.compare(value, 'value1')

    d1.transact(txn => x.set(txn, 'key', 'value2'))
    value = d1.transact(txn => x.get(txn, 'key'))
    t.compare(value, 'value2')
}

/**
 * @param {t.TestCase} tc
 */
export const testSetNested = tc => {
    const d1 = new Y.YDoc()
    const x = d1.getMap('test')
    const nested = new Y.YMap({ a: 'A' })

    d1.transact(txn => x.set(txn, 'key', nested))
    d1.transact(txn => nested.set(txn, 'b', 'B'))

    let json = d1.transact(txn => x.toJson(txn))
    t.compareObjects(json.key, {
        a: 'A',
        b: 'B'
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testDelete = tc => {
    const d1 = new Y.YDoc()
    const x = d1.getMap('test')

    d1.transact(txn => x.set(txn, 'key', 'value1'))
    var len = d1.transact(txn => x.length(txn))
    var value = d1.transact(txn => x.get(txn, 'key'))
    t.compare(len, 1)
    t.compare(value, 'value1')

    d1.transact(txn => x.delete(txn, 'key'))
    len = d1.transact(txn => x.length(txn))
    value = d1.transact(txn => x.get(txn, 'key'))
    t.compare(len, 0)
    t.compare(value, undefined)

    d1.transact(txn => x.set(txn, 'key', 'value2'))
    len = d1.transact(txn => x.length(txn))
    value = d1.transact(txn => x.get(txn, 'key'))
    t.compare(len, 1)
    t.compare(value, 'value2')
}

/**
 * @param {t.TestCase} tc
 */
export const testIterator = tc => {
    const d1 = new Y.YDoc()
    const x = d1.getMap('test')

    d1.transact(txn => {
        x.set(txn, 'a', 1)
        x.set(txn, 'b', 2)
        x.set(txn, 'c', 3)
    })

    let txn = d1.beginTransaction()
    try {
        let expected = {
            'a': 1,
            'b': 2,
            'c': 3
        }
        for (let [key, value] of x.entries(txn)) {
            let v = expected[key]
            t.compare(value, v)
            delete expected[key]
        }
    } finally {
        txn.free()
    }
}