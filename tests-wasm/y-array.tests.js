import { exchangeUpdates } from './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {t.TestCase} tc
 */
export const testInserts = tc => {
    const d1 = new Y.YDoc({clientID:1})
    t.compare(d1.id, 1)
    var x = d1.getArray('test');

    x.insert(0, [1, 2.5, 'hello', ['world'], true])
    x.push( [{key:'value'}])

    const expected = [1, 2.5, 'hello', ['world'], true, {key:'value'}]

    var value = x.toJson()
    t.compare(value, expected)

    const d2 = new Y.YDoc({clientID:2})
    x = d2.getArray('test');

    exchangeUpdates([d1, d2])

    value = x.toJson()
    t.compare(value, expected)
}

/**
 * @param {t.TestCase} tc
 */
export const testInsertsNested = tc => {
    const d1 = new Y.YDoc()
    var x = d1.getArray('test');

    const nested = new Y.YArray();
    nested.push(['world'])
    x.insert(0, [1, 2, nested, 3, 4])
    nested.insert(0, ['hello'])

    const expected = [1, 2, ['hello', 'world'], 3, 4]

    var value = x.toJson()
    t.compare(value, expected)

    const d2 = new Y.YDoc()
    x = d2.getArray('test');

    exchangeUpdates([d1, d2])

    value = x.toJson()
    t.compare(value, expected)
}

/**
 * @param {t.TestCase} tc
 */
export const testDelete = tc => {
    const d1 = new Y.YDoc({clientID:1})
    t.compare(d1.id, 1)
    var x = d1.getArray('test')

    x.insert(0, [1, 2, ['hello', 'world'], true])
    x.delete(1, 2)

    const expected = [1, true]

    var value = x.toJson()
    t.compare(value, expected)

    const d2 = new Y.YDoc({clientID:2})
    x = d2.getArray('test')

    exchangeUpdates([d1, d2])

    value = x.toJson()
    t.compare(value, expected)
}

/**
 * @param {t.TestCase} tc
 */
export const testGet = tc => {
    const d1 = new Y.YDoc()
    const x = d1.getArray('test')

    x.insert(0, [1, 2, true])
    x.insert(1, ['hello', 'world'])

    const zeroed = x.get(0)
    const first = x.get(1)
    const second = x.get(2)
    const third = x.get(3)
    const fourth = x.get(4)

    t.compare(zeroed, 1)
    t.compare(first, 'hello')
    t.compare(second, 'world')
    t.compare(third, 2)
    t.compare(fourth, true)

    t.fails(() => {
        // should fail because it's outside the bounds
        x.get(5)
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testIterator = tc => {
    const d1 = new Y.YDoc()
    const x = d1.getArray('test')

    x.insert(0, [1, 2, 3])
    t.compare(x.length(), 3)

    let i = 1;
    let txn = d1.readTransaction()
    for (let v of x.values(txn)) {
        t.compare(v, i)
        i++
    }
    txn.free()
}

/**
 * @param {t.TestCase} tc
 */
export const testObserver = tc => {
    const d1 = new Y.YDoc()
    /**
     * @param {Y.YArray} tc
     */
    const x = d1.getArray('test')
    let target = null
    let delta = null
    let origin = null
    let observer = x.observe(e => {
        target = e.target
        delta = e.delta
        origin = e.origin
    })

    // insert initial data to an empty YArray
    d1.transact((txn) => {
        x.insert(0, [1,2,3,4], txn)
    }, 'TEST_ORIGIN')
    t.compare(target.toJson(), x.toJson())
    t.compare(delta, [{insert: [1,2,3,4]}])
    t.compare(origin, 'TEST_ORIGIN')
    target = null
    delta = null

    // remove 2 items from the middle
    x.delete(1, 2)
    t.compare(target.toJson(), x.toJson())
    t.compare(delta, [{retain:1}, {delete: 2}])
    t.compare(origin, undefined)
    target = null
    delta = null

    // insert new item in the middle
    x.insert(1, [5])
    t.compare(target.toJson(), x.toJson())
    t.compare(delta, [{retain:1}, {insert: [5]}])
    target = null
    delta = null

    // free the observer and make sure that callback is no longer called
    observer.free()
    x.insert(1, [6])
    t.compare(target, null)
    t.compare(delta, null)
}

/**
 * @param {t.TestCase} tc
 */
export const testObserveDeepEventOrder = tc => {
    const d1 = new Y.YDoc()
    const arr = d1.getArray('array')

    /**
     * @type {Array<any>}
     */
    let paths = []
    let subscription = arr.observeDeep(events => {
        paths = events.map(e => e.path())
    })
    arr.insert(0, [new Y.YMap()])
    d1.transact(txn => {
        arr.get(0, txn).set('a', 'a', txn)
        arr.insert(0, [0], txn)
    })
    t.compare(paths, [ [], [ 1 ] ])
    subscription.free()
}

/**
 * @param {t.TestCase} tc
 */
export const testMove = tc => {
    const d1 = new Y.YDoc()
    const arr = d1.getArray('array')

    let e = null
    arr.observe(event => {
        e = event
    })
    arr.insert(0, [1, 2, 3])
    arr.move(1, 0)
    t.compare(arr.toJson(), [2, 1, 3])
    arr.move(0, 2)
    t.compare(arr.toJson(), [1, 2, 3])
}