import { exchangeUpdates } from './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {t.TestCase} tc
 */
export const testInserts = tc => {
    const d1 = new Y.YDoc(1)
    t.compare(d1.id, 1)
    var x = d1.getArray('test');

    d1.transact(txn => x.insert(txn, 0, [1, 2.5, 'hello', ['world'], true]))
    d1.transact(txn => x.push(txn, [{key:'value'}]))

    const expected = [1, 2.5, 'hello', ['world'], true, {key:'value'}]

    var value = d1.transact(txn => x.toJson(txn))
    t.compare(value, expected)

    const d2 = new Y.YDoc(2)
    x = d2.getArray('test');

    exchangeUpdates([d1, d2])

    value = d2.transact(txn => x.toJson(txn))
    t.compare(value, expected)
}

/**
 * @param {t.TestCase} tc
 */
export const testInsertsNested = tc => {
    const d1 = new Y.YDoc()
    var x = d1.getArray('test');

    const nested = new Y.YArray();
    d1.transact(txn => nested.push(txn, ['world']))
    d1.transact(txn => x.insert(txn, 0, [1, 2, nested, 3, 4]))
    d1.transact(txn => nested.insert(txn, 0, ['hello']))

    const expected = [1, 2, ['hello', 'world'], 3, 4]

    var value = d1.transact(txn => x.toJson(txn))
    t.compare(value, expected)

    const d2 = new Y.YDoc()
    x = d2.getArray('test');

    exchangeUpdates([d1, d2])

    value = d2.transact(txn => x.toJson(txn))
    t.compare(value, expected)
}

/**
 * @param {t.TestCase} tc
 */
export const testDelete = tc => {
    const d1 = new Y.YDoc(1)
    t.compare(d1.id, 1)
    var x = d1.getArray('test')

    d1.transact(txn => x.insert(txn, 0, [1, 2, ['hello', 'world'], true]))
    d1.transact(txn => x.delete(txn, 1, 2))

    const expected = [1, true]

    var value = d1.transact(txn => x.toJson(txn))
    t.compare(value, expected)

    const d2 = new Y.YDoc(2)
    x = d2.getArray('test')

    exchangeUpdates([d1, d2])

    value = d2.transact(txn => x.toJson(txn))
    t.compare(value, expected)
}

/**
 * @param {t.TestCase} tc
 */
export const testGet = tc => {
    const d1 = new Y.YDoc()
    const x = d1.getArray('test')

    d1.transact(txn => x.insert(txn, 0, [1, 2, true]))
    d1.transact(txn => x.insert(txn, 1, ['hello', 'world']));

    const zeroed = d1.transact(txn => x.get(txn, 0))
    const first = d1.transact(txn => x.get(txn, 1))
    const second = d1.transact(txn => x.get(txn, 2))
    const third = d1.transact(txn => x.get(txn, 3))
    const fourth = d1.transact(txn => x.get(txn, 4))

    t.compare(zeroed, 1)
    t.compare(first, 'hello')
    t.compare(second, 'world')
    t.compare(third, 2)
    t.compare(fourth, true)

    t.fails(() => {
        // should fail because it's outside of the bounds
        d1.transact(txn => x.get(txn, 5))
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testIterator = tc => {
    const d1 = new Y.YDoc()
    const x = d1.getArray('test')

    d1.transact(txn => x.insert(txn, 0, [1, 2, 3]))
    t.compare(x.length, 3)

    const txn = d1.beginTransaction()
    try {
        let i = 1;
        for (let v of x.values(txn)) {
            t.compare(v, i)
            i++
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
    const x = d1.getArray('test')
    let target = null
    let delta = null
    let observer = x.observe(e => {
        target = e.target
        delta = e.delta
    })

    // insert initial data to an empty YArray
    d1.transact(txn => x.insert(txn, 0, [1,2,3,4]))
    t.compare(target, x)
    t.compare(delta, [{insert: [1,2,3,4]}])
    target = null
    delta = null

    // remove 2 items from the middle
    d1.transact(txn => x.delete(txn, 1, 2))
    t.compare(target, x)
    t.compare(delta, [{retain:1}, {delete: 2}])
    target = null
    delta = null

    // insert new item in the middle
    d1.transact(txn => x.insert(txn, 1, [5]))
    t.compare(target, x)
    t.compare(delta, [{retain:1}, {insert: [5]}])
    target = null
    delta = null

    // free the observer and make sure that callback is no longer called
    observer.free()
    d1.transact(txn => x.insert(txn, 1, [6]))
    t.compare(target, null)
    t.compare(delta, null)
}