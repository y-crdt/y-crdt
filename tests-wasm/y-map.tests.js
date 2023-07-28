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

    x.set('key', 'value1')
    value = x.get('key')
    t.compare(value, 'value1')

    x.set('key', 'value2')
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

    x.set('key', nested)
    nested.set('b', 'B')

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

    x.set('key', 'value1')
    var len = x.length()
    var value = x.get('key')
    t.compare(len, 1)
    t.compare(value, 'value1')

    x.delete('key')
    len = x.length()
    value = x.get('key')
    t.compare(len, 0)
    t.compare(value, undefined)

    x.set('key', 'value2')
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
        x.set('a', 1, txn)
        x.set('b', 2, txn)
        x.set('c', 3, txn)
    })

    let expected = {
        'a': 1,
        'b': 2,
        'c': 3
    }
    d1.transact(txn => {
        let entries = x.entries(txn);
        for (let key in entries) {
            let v = expected[key]
            t.compare(entries[key], v)
            delete expected[key]
        }
    })
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
    let origin = null
    let observer = x.observe(e => {
        target = e.target
        entries = e.keys
        origin = e.origin
    })

    // insert initial data to an empty YMap
    d1.transact(txn => {
        x.set('key1', 'value1', txn)
        x.set('key2', 2, txn)
    }, 'TEST_ORIGIN')
    t.compare(target.toJson(), x.toJson())
    t.compare(entries, {
        key1: { action: 'add', newValue: 'value1' },
        key2: { action: 'add', newValue: 2 }
    })
    t.compare(origin, 'TEST_ORIGIN')
    target = null
    entries = null

    // remove an entry and update another on
    d1.transact(txn => {
        x.delete('key1', txn)
        x.set('key2', 'value2', txn)
    })
    t.compare(target.toJson(), x.toJson())
    t.compare(entries, {
        key1: { action: 'delete', oldValue: 'value1' },
        key2: { action: 'update', oldValue: 2, newValue: 'value2' }
    })
    t.compare(origin, undefined)
    target = null
    entries = null

    // free the observer and make sure that callback is no longer called
    observer.free()
    x.set('key1', [6])
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
    map.set('map', new Y.YMap())
    map.get('map').set('array', new Y.YArray())
    map.get('map').get('array').insert(0, ['content'])
    t.assert(calls === 3)
    t.compare(paths, [[], ['map'], ['map', 'array']])
}