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
    t.compare(json, {
        key: {
            a: 'A',
            b: 'B'
        }
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

/**
 * @param {t.TestCase} tc
 */
export const testObserver = tc => {
    const d1 = new Y.YDoc()
    /**
     * @param {Y.YMap} tc
     */
    const getValue = (x) => d1.transact(txn => x.toJson(txn))
    const x = d1.getMap('test')
    let target = null
    let entries = null
    let observer = x.observe(e => {
        target = e.target
        entries = e.keys
    })

    // insert initial data to an empty YMap
    d1.transact(txn => {
        x.set(txn, 'key1', 'value1')
        x.set(txn, 'key2', 2)
    })
    t.compare(getValue(target), getValue(x))
    t.compare(entries, {
        key1: { action: 'add', newValue: 'value1' },
        key2: { action: 'add', newValue: 2 }
    })
    target = null
    entries = null

    // remove an entry and update another on
    d1.transact(txn => {
        x.delete(txn, 'key1')
        x.set(txn, 'key2', 'value2')
    })
    t.compare(getValue(target), getValue(x))
    t.compare(entries, {
        key1: { action: 'delete', oldValue: 'value1' },
        key2: { action: 'update', oldValue: 2, newValue: 'value2' }
    })
    target = null
    entries = null

    // free the observer and make sure that callback is no longer called
    observer.free()
    d1.transact(txn => x.insert(txn, 1, [6]))
    t.compare(target, null)
    t.compare(entries, null)
}