import { exchangeUpdates } from './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {t.TestCase} tc
 */
export const testSet = tc => {
    const d1 = new Y.YDoc()
    const x = d1.getMap('test')

    var value = x.get('key')
    t.compare(value, undefined)

    d1.transact(txn => x.set(txn, 'key', 'value1'))
    value = x.get('key')
    t.compare(value, 'value1')

    d1.transact(txn => x.set(txn, 'key', 'value2'))
    value = x.get('key')
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

    let json = x.toJson()
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
    var len = x.length()
    var value = x.get('key')
    t.compare(len, 1)
    t.compare(value, 'value1')

    d1.transact(txn => x.delete(txn, 'key'))
    len = x.length()
    value = x.get('key')
    t.compare(len, 0)
    t.compare(value, undefined)

    d1.transact(txn => x.set(txn, 'key', 'value2'))
    len = x.length()
    value = x.get('key')
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

    let expected = {
        'a': 1,
        'b': 2,
        'c': 3
    }
    for (let [key, value] of x.entries()) {
        let v = expected[key]
        t.compare(value, v)
        delete expected[key]
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
    t.compare(target.toJson(), x.toJson())
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
    t.compare(target.toJson(), x.toJson())
    t.compare(entries, {
        key1: { action: 'delete', oldValue: 'value1' },
        key2: { action: 'update', oldValue: 2, newValue: 'value2' }
    })
    target = null
    entries = null

    // free the observer and make sure that callback is no longer called
    observer.free()
    d1.transact(txn => x.set(txn, 'key1', [6]))
    t.compare(target, null)
    t.compare(entries, null)
}
/**
 * @param {t.TestCase} tc
 */
export const testObserversUsingObservedeep = tc => {
    const d1 = new Y.YDoc()
    const map = d1.getMap('map')

    /**
     * @type {Array<Array<string|number>>}
     */
    const paths = []
    let calls = 0
    map.observeDeep(events => {
        for (let e of events) {
            paths.push(e.path())
        }
        calls++
    })
    d1.transact(txn => map.set(txn, 'map', new Y.YMap()))
    d1.transact(txn => map.get('map').set(txn, 'array', new Y.YArray()))
    d1.transact(txn => map.get('map').get('array').insert(txn, 0, ['content']))
    t.assert(calls === 3)
    t.compare(paths, [[], ['map'], ['map', 'array']])
}