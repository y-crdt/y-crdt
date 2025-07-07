import {exchangeUpdates} from './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {t.TestCase} tc
 */
export const testOnUpdate = tc => {
    const d1 = new Y.YDoc({clientID: 1})
    const text1 = d1.getText('text')
    text1.insert(0, 'hello')
    let update = Y.encodeStateAsUpdate(d1)

    const d2 = new Y.YDoc({clientID: 2})
    const text2 = d2.getText('text')
    let actual;
    let origin;
    const callback = (e, tx) => {
        actual = e
        origin = tx.origin
    }
    d2.on('update', callback);
    Y.applyUpdate(d2, update, d1.id)

    t.compare(origin, d1.id)
    t.compare(text1.toString(), text2.toString())
    t.compare(actual, update)

    // check unsubscribe
    t.assert(d2.off('update', callback), 'off "update" failed')
    actual = null
    origin = null

    text1.insert(5, 'world')
    update = Y.encodeStateAsUpdate(d1)
    Y.applyUpdate(d2, update, d1.id)

    t.compare(text1.toString(), text2.toString())
    t.compare(actual, null) // subscription was released, we should get no more updates
    t.compare(origin, null)
}

/**
 * @param {t.TestCase} tccls
 */
export const testOnUpdateV2 = tc => {
    const d1 = new Y.YDoc({clientID: 1})
    const text1 = d1.getText('text')
    text1.insert(0, 'hello')
    let expected = Y.encodeStateAsUpdateV2(d1)

    const d2 = new Y.YDoc({clientID: 2})
    const text2 = d2.getText('text')
    let actual;
    const callback = e => actual = e
    d2.on('updateV2', callback);
    Y.applyUpdateV2(d2, expected)

    t.compare(text1.toString(), text2.toString())
    t.compare(actual, expected)

    // check unsubscribe
    t.assert(d2.off('updateV2', callback), 'off "updateV2" failed')
    actual = null

    text1.insert(5, 'world')
    expected = Y.encodeStateAsUpdateV2(d1)
    Y.applyUpdateV2(d2, expected)

    t.compare(text1.toString(), text2.toString())
    t.compare(actual, null) // subscription was release, we should get no more updates
}

/**
 * @param {t.TestCase} tc
 */
export const testOnAfterTransaction = tc => {
    const doc = new Y.YDoc({clientID: 1})
    const text = doc.getText('text')
    let event;
    let callback = txn => {
        let beforeState = txn.beforeState
        let afterState = txn.afterState
        let deleteSet = txn.deleteSet
        event = {beforeState, afterState, deleteSet}
    }
    doc.on('afterTransaction', callback);

    text.insert(0, 'hello world')

    t.compare(event.beforeState, new Map());
    let state = new Map()
    state.set(1, 11)
    t.compare(event.afterState, state);
    t.compare(event.deleteSet, new Map());

    event = null
    text.delete(2, 7)

    t.compare(event.beforeState, state);
    t.compare(event.afterState, state);
    state = new Map()
    state.set(1, [[2, 7]])
    t.compare(event.deleteSet, state);

    t.assert(doc.off('afterTransaction', callback), 'off "afterTransaction" failed');
    event = null
    text.insert(4, ' the door')

    t.compare(event, null)
}

/**
 * @param {t.TestCase} tc
 */
export const testSnapshots = tc => {
    const doc = new Y.YDoc({clientID: 1})
    const text = doc.getText('text')
    text.insert(0, 'hello')
    const prev = Y.snapshot(doc)
    text.insert(5, ' world')
    const next = Y.snapshot(doc)

    const delta = text.toDelta(next, prev)
    t.compare(delta, [
        {insert: 'hello'},
        {insert: ' world', attributes: {ychange: {type: 'added'}}}
    ])
}

/**
 * @param {t.TestCase} tc
 */
export const testSnapshotState = tc => {
    const d1 = new Y.YDoc({clientID: 1, gc: false})
    const txt1 = d1.getText('text')
    txt1.insert(0, 'hello')
    const prev = Y.snapshot(d1)
    txt1.insert(5, ' world')
    const state = Y.encodeStateFromSnapshotV1(d1, prev)

    const d2 = new Y.YDoc({clientID: 2})
    const txt2 = d2.getText('text')
    Y.applyUpdate(d2, state)

    t.compare(txt2.toString(), 'hello')
}

/**
 * @param {t.TestCase} tc
 */
export const testSubdoc = tc => {
    const doc = new Y.YDoc()
    doc.load() // doesn't do anything
    {
        /**
         * @type {Array<any>|null}
         */
        let event = /** @type {any} */ (null)
        const callback = subdocs => {
            let added = Array.from(subdocs.added).map(x => x.guid).sort()
            let removed = Array.from(subdocs.removed).map(x => x.guid).sort()
            let loaded = Array.from(subdocs.loaded).map(x => x.guid).sort()
            event = [added, removed, loaded]
        }
        doc.on('subdocs', callback)
        const subdocs = doc.getMap('mysubdocs')
        const docA = new Y.YDoc({guid: 'a'})
        docA.load()
        subdocs.set('a', docA)
        t.compare(event, [['a'], [], ['a']])

        event = null
        subdocs.get('a').load()
        t.assert(event === null)

        event = null
        subdocs.get('a').destroy()
        t.compare(event, [['a'], ['a'], []])
        subdocs.get('a').load()
        t.compare(event, [[], [], ['a']])

        subdocs.set('b', new Y.YDoc({guid: 'a', shouldLoad: false}))
        t.compare(event, [['a'], [], []])
        subdocs.get('b').load()
        t.compare(event, [[], [], ['a']])

        const docC = new Y.YDoc({guid: 'c'})
        docC.load()
        subdocs.set('c', docC)
        t.compare(event, [['c'], [], ['c']])

        t.compare(doc.getSubdocGuids(), new Set(['a', 'c']))
    }

    const doc2 = new Y.YDoc()
    // root-level types must be prepared in advance for subdocs to work atm
    const subdocs2 = doc2.getMap('mysubdocs')
    {
        t.compare(Array.from(doc2.getSubdocs()), [])
        /**
         * @type {Array<any>|null}
         */
        let event = /** @type {any} */ (null)
        doc2.on('subdocs', subdocs => {
            let added = Array.from(subdocs.added).map(x => x.guid).sort()
            let removed = Array.from(subdocs.removed).map(x => x.guid).sort()
            let loaded = Array.from(subdocs.loaded).map(x => x.guid).sort()
            event = [added, removed, loaded]
        })
        Y.applyUpdate(doc2, Y.encodeStateAsUpdate(doc))
        t.compare(event, [['a', 'a', 'c'], [], []])

        let inner = subdocs2.get('a')
        t.assert(inner.parentDoc != null, 'parent doc must be present')
        t.compare(inner.shouldLoad, false, 'after decoding shouldLoad is false by default')
        inner.load()
        t.compare(inner.shouldLoad, true, 'after YDoc.load, shouldLoad is false by default')
        t.compare(event, [[], [], ['a']])

        t.compare(Array.from(doc2.getSubdocGuids()).sort(), ['a', 'c'])

        doc2.getMap('mysubdocs').delete('a')
        t.compare(event, [[], ['a'], []])
        t.compare(Array.from(doc2.getSubdocGuids()).sort(), ['a', 'c'])
    }
}

/**
 * @param {t.TestCase} tc
 */
export const testLiveness = tc => {
    const d1 = new Y.YDoc()
    const r1 = d1.getMap('root')
    const d2 = new Y.YDoc()
    const r2 = d2.getMap('root')
    const a1 = new Y.YMap()
    r1.set('a', a1)
    const aa1 = new Y.YMap()
    a1.set('aa', aa1)

    // all nodes should be alive
    d1.transact(tx => {
        t.assert(r1.alive(tx), 'root is always alive')
        t.assert(a1.alive(tx), '1st level nesting (local)')
        t.assert(aa1.alive(tx), '2nd level nesting (local)')
    })

    exchangeUpdates([d1, d2])

    const a2 = r2.get('a')
    const aa2 = a2.get('aa')

    // all nodes should be alive on remote as well
    d2.transact(tx => {
        t.assert(r2.alive(tx), 'root is always alive')
        t.assert(a2.alive(tx), '1st level nesting (local)')
        t.assert(aa2.alive(tx), '2nd level nesting (local)')
    })

    // drop nodes on local
    r1.delete('a')
    exchangeUpdates([d1, d2])

    // child nodes should be marked as dead
    d1.transact(tx => {
        t.assert(r1.alive(tx), 'root is always alive')
        t.assert(!a1.alive(tx), 'child is deleted (local)')
        t.assert(!aa1.alive(tx), 'parent was deleted (local)')
    })
    d2.transact(tx => {
        t.assert(r2.alive(tx), 'root is always alive')
        t.assert(!a2.alive(tx), 'child is deleted (local)')
        t.assert(!aa2.alive(tx), 'parent was deleted (local)')
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testRoots = tc => {
    const d1 = new Y.YDoc()
    const a = d1.getMap('a')
    const b = d1.getText('b')
    const c = d1.getArray('c')
    const d = d1.getXmlFragment('d')

    const roots = d1.roots()
        .sort((a, b) => a[0].localeCompare(b[0]))
        .map(([k, v]) => [k, v.constructor])
    t.compare(roots, [
        ['a', Y.YMap],
        ['b', Y.YText],
        ['c', Y.YArray],
        ['d', Y.YXmlFragment]
    ])

    a.set('hello', 'world')

    let d2 = new Y.YDoc()
    exchangeUpdates([d1, d2])
    t.compare(d2.roots(), [['a', undefined]])
}

/**
 * @param {t.TestCase} tc
 */
export const testIds = tc => {
    const d1 = new Y.YDoc()
    const a1 = d1.getArray('a')

    const d2 = new Y.YDoc()
    d2.getArray('a') // root types need to be pre-initialized

    const m1 = new Y.YMap({'key1': 'value1'})
    a1.push([m1]) // set nested type on a first doc

    const arrayId = a1.id
    const mapId = m1.id

    // sync
    exchangeUpdates([d1, d2])

    // resolve instances using identifiers
    const m2 = d2.transact(tx => tx.get(mapId))

    t.compare(m2.toJson(), {'key1': 'value1'})

    const a2 = d2.transact(tx => tx.get(arrayId))

    // modify instance on remote end and sync
    m2.set('key1', 'value2')
    a2.insert(0, ['abc'])

    exchangeUpdates([d1, d2])

    // check if first doc has correctly updated values
    t.compare(a1.toJson(), ['abc', {'key1': 'value2'}])
}


/**
 * @param {t.TestCase} tc
 */
export const testSelectAll = tc => {
    const doc = new Y.YDoc({clientID: 1})
    const store = doc.getMap('store')
    store.set('book', new Y.YArray([
        new Y.YMap({
            "author": "Nigel Rees",
            "title": "Sayings of the Century",
            "price": 8.95
        }),
        new Y.YMap({
            "author": "J. R. R. Tolkien",
            "title": "The Lord of the Rings",
            "isbn": "0-395-19395-8",
            "price": 22.99
        })
    ]))
    store.set('bicycle', {
        "color": "red",
        "price": 399
    })

    t.compare([8.95, 22.99], doc.selectAll('$.store.book[*].price'))

    let result = doc.selectAll('$...price')
    // we sort results because YMap doesn't guarantee order
    result.sort((a, b) => a - b)
    t.compare(result, [8.95, 22.99, 399])

    result = doc.selectOne('$.store.book[1].price')
    t.compare(result, 22.99)
}

/**
 * @param {t.TestCase} tc
 */
export const testMergeUpdatesV1 = tc => {
    // Create three separate documents with different changes
    const d1 = new Y.YDoc({clientID: 1})
    const text1 = d1.getText('text')
    text1.insert(0, 'hello')
    const update1 = Y.encodeStateAsUpdate(d1)

    const d2 = new Y.YDoc({clientID: 2})
    const text2 = d2.getText('text')
    text2.insert(0, 'world')
    const update2 = Y.encodeStateAsUpdate(d2)

    const d3 = new Y.YDoc({clientID: 3})
    const map3 = d3.getMap('map')
    map3.set('key', 'value')
    const update3 = Y.encodeStateAsUpdate(d3)

    // Test merging multiple updates
    const mergedUpdate = Y.mergeUpdatesV1([update1, update2, update3])

    // Apply merged update to a new document
    const dMerged = new Y.YDoc({clientID: 4})
    const textMerged = dMerged.getText('text')
    const mapMerged = dMerged.getMap('map')
    Y.applyUpdate(dMerged, mergedUpdate)

    // Apply updates individually to another document for comparison
    const dSequential = new Y.YDoc({clientID: 5})
    const textSequential = dSequential.getText('text')
    const mapSequential = dSequential.getMap('map')
    Y.applyUpdate(dSequential, update1)
    Y.applyUpdate(dSequential, update2)
    Y.applyUpdate(dSequential, update3)

    // Both documents should have identical content
    t.compare(textMerged.toString(), textSequential.toString())
    t.compare(mapMerged.get('key'), mapSequential.get('key'))
    t.compare(mapMerged.get('key'), 'value')
}

/**
 * @param {t.TestCase} tc
 */
export const testMergeUpdatesV2 = tc => {
    // Create three separate documents with different changes
    const d1 = new Y.YDoc({clientID: 1})
    const array1 = d1.getArray('array')
    array1.insert(0, [1, 2, 3])
    const update1 = Y.encodeStateAsUpdateV2(d1)

    const d2 = new Y.YDoc({clientID: 2})
    const array2 = d2.getArray('array')
    array2.insert(0, [4, 5, 6])
    const update2 = Y.encodeStateAsUpdateV2(d2)

    const d3 = new Y.YDoc({clientID: 3})
    const text3 = d3.getText('text')
    text3.insert(0, 'merged')
    const update3 = Y.encodeStateAsUpdateV2(d3)

    // Test merging multiple v2 updates
    const mergedUpdate = Y.mergeUpdatesV2([update1, update2, update3])

    // Apply merged update to a new document
    const dMerged = new Y.YDoc({clientID: 4})
    const arrayMerged = dMerged.getArray('array')
    const textMerged = dMerged.getText('text')
    Y.applyUpdateV2(dMerged, mergedUpdate)

    // Apply updates individually to another document for comparison
    const dSequential = new Y.YDoc({clientID: 5})
    const arraySequential = dSequential.getArray('array')
    const textSequential = dSequential.getText('text')
    Y.applyUpdateV2(dSequential, update1)
    Y.applyUpdateV2(dSequential, update2)
    Y.applyUpdateV2(dSequential, update3)

    // Both documents should have identical content
    t.compare(arrayMerged.toJson(), arraySequential.toJson())
    t.compare(textMerged.toString(), textSequential.toString())
    t.compare(textMerged.toString(), 'merged')

    // Test merging empty array
    const emptyMerge = Y.mergeUpdatesV2([])
    t.assert(emptyMerge.length === 0, 'merging empty array should return empty update')
}