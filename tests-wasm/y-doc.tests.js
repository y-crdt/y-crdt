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
    const sub = d2.onUpdate((e, tx) => {
        actual = e
        origin = tx.origin
    });
    Y.applyUpdate(d2, update, d1.id)

    t.compare(origin, d1.id)
    t.compare(text1.toString(), text2.toString())
    t.compare(actual, update)

    // check unsubscribe
    sub.free()
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

    const d2 = new Y.YDoc(2)
    const text2 = d2.getText('text')
    let actual;
    const sub = d2.onUpdateV2(e => actual = e);
    Y.applyUpdateV2(d2, expected)

    t.compare(text1.toString(), text2.toString())
    t.compare(actual, expected)

    // check unsubscribe
    sub.free()
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
    const sub = doc.onAfterTransaction(txn => {
        let beforeState = txn.beforeState
        let afterState = txn.afterState
        let deleteSet = txn.deleteSet
        event = {beforeState, afterState, deleteSet}
    });

    text.insert(0, 'hello world')

    console.log(event)
    t.compare(event.beforeState, new Map());
    let state = new Map()
    state.set(1n, 11)
    t.compare(event.afterState, state);
    t.compare(event.deleteSet, new Map());

    event = null
    text.delete(2, 7)

    t.compare(event.beforeState, state);
    t.compare(event.afterState, state);
    state = new Map()
    state.set(1n, [[2, 7]])
    t.compare(event.deleteSet, state);

    sub.free()
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
        doc.onSubdocs(subdocs => {
            let added = Array.from(subdocs.added).map(x => x.guid).sort()
            let removed = Array.from(subdocs.removed).map(x => x.guid).sort()
            let loaded = Array.from(subdocs.loaded).map(x => x.guid).sort()
            event = [added, removed, loaded]
        })
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
        doc2.onSubdocs(subdocs => {
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