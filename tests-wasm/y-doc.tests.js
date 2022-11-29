import { exchangeUpdates } from './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {t.TestCase} tc
 */
export const testOnUpdate = tc => {
    const d1 = new Y.YDoc({clientID: 1})
    const text1 = d1.getText('text')
    text1.insert(0, 'hello')
    let expected = Y.encodeStateAsUpdate(d1)

    const d2 = new Y.YDoc(2)
    const text2 = d2.getText('text')
    let actual;
    const sub = d2.onUpdate(e => actual = e);
    Y.applyUpdate(d2, expected)

    t.compare(text1.toString(), text2.toString())
    t.compare(actual, expected)

    // check unsubscribe
    sub.free()
    actual = null

    text1.insert(5, 'world')
    expected = Y.encodeStateAsUpdate(d1)
    Y.applyUpdate(d2, expected)

    t.compare(text1.toString(), text2.toString())
    t.compare(actual, null) // subscription was release, we should get no more updates
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
    const sub = doc.onAfterTransaction(e => event = e);

    text.insert(0, 'hello world')

    t.compare(event.beforeState, new Map());
    let state = new Map()
    state.set(1n, 11)
    t.compare(event.afterState, state);
    t.compare(event.deleteSet, new Map());

    event = null
    text.delete( 2, 7)

    t.compare(event.beforeState, state);
    t.compare(event.afterState, state);
    state = new Map()
    state.set(1n, [[2,7]])
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
        { insert: 'hello' },
        { insert: ' world', attributes: { ychange: { type: 'added' } } }
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

    const d2 = new Y.YDoc(2)
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
            let added = Array.from(subdocs.added).map(x => x.guid)
            let removed = Array.from(subdocs.removed).map(x => x.guid)
            let loaded = Array.from(subdocs.loaded).map(x => x.guid)
            event = [added, removed, loaded]
        })
        const subdocs = doc.getMap('mysubdocs')
        const docA = new Y.YDoc({ guid: 'a' })
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

        subdocs.set('b', new Y.YDoc({ guid: 'a', shouldLoad: false }))
        t.compare(event, [['a'], [], []])
        subdocs.get('b').load()
        t.compare(event, [[], [], ['a']])

        const docC = new Y.YDoc({ guid: 'c' })
        docC.load()
        subdocs.set('c', docC)
        t.compare(event, [['c'], [], ['c']])

        t.compare(Array.from(doc.getSubdocGuids()), ['a', 'c'])
    }

    const doc2 = new Y.Doc()
    {
        t.compare(Array.from(doc2.getSubdocs()), [])
        /**
         * @type {Array<any>|null}
         */
        let event = /** @type {any} */ (null)
        doc2.onSubdocs(subdocs => {
            let added = Array.from(subdocs.added).map(x => x.guid)
            let removed = Array.from(subdocs.removed).map(x => x.guid)
            let loaded = Array.from(subdocs.loaded).map(x => x.guid)
            event = [added, removed, loaded]
        })
        Y.applyUpdate(doc2, Y.encodeStateAsUpdate(doc))
        t.compare(event, [['a', 'a', 'c'], [], []])

        doc2.getMap('mysubdocs').get('a').load()
        t.compare(event, [[], [], ['a']])

        t.compare(Array.from(doc2.getSubdocGuids()), ['a', 'c'])

        doc2.getMap('mysubdocs').delete('a')
        t.compare(event, [[], ['a'], []])
        t.compare(Array.from(doc2.getSubdocGuids()), ['a', 'c'])
    }
}

/**
 * @param {t.TestCase} tc
 */
export const testSubdocLoadEdgeCases = tc => {
    const ydoc = new Y.YDoc()
    const yarray = ydoc.getArray('test')
    const subdoc1 = new Y.YDoc()
    /**
     * @type {any}
     */
    let lastEvent = null
    ydoc.onSubdocs(event => {
        lastEvent = event
    })
    yarray.insert(0, [subdoc1])
    t.assert(subdoc1.shouldLoad)
    t.assert(subdoc1.autoLoad === false)
    t.assert(lastEvent !== null && lastEvent.loaded.has(subdoc1))
    t.assert(lastEvent !== null && lastEvent.added.has(subdoc1))
    // destroy and check whether lastEvent adds it again to added (it shouldn't)
    subdoc1.destroy()
    const subdoc2 = yarray.get(0)
    t.assert(subdoc1 !== subdoc2)
    t.assert(lastEvent !== null && lastEvent.added.has(subdoc2))
    t.assert(lastEvent !== null && !lastEvent.loaded.has(subdoc2))
    // load
    subdoc2.load()
    t.assert(lastEvent !== null && !lastEvent.added.has(subdoc2))
    t.assert(lastEvent !== null && lastEvent.loaded.has(subdoc2))
    // apply from remote
    const ydoc2 = new Y.YDoc()
    ydoc2.onSubdocs(event => {
        lastEvent = event
    })
    Y.applyUpdate(ydoc2, Y.encodeStateAsUpdate(ydoc))
    const subdoc3 = ydoc2.getArray('test').get(0)
    t.assert(subdoc3.shouldLoad === false)
    t.assert(subdoc3.autoLoad === false)
    t.assert(lastEvent !== null && lastEvent.added.has(subdoc3))
    t.assert(lastEvent !== null && !lastEvent.loaded.has(subdoc3))
    // load
    subdoc3.load()
    t.assert(subdoc3.shouldLoad)
    t.assert(lastEvent !== null && !lastEvent.added.has(subdoc3))
    t.assert(lastEvent !== null && lastEvent.loaded.has(subdoc3))
}

/**
 * @param {t.TestCase} tc
 */
export const testSubdocLoadEdgeCasesAutoload = tc => {
    const ydoc = new Y.YDoc()
    const yarray = ydoc.getArray('test')
    const subdoc1 = new Y.YDoc({ autoLoad: true })
    /**
     * @type {any}
     */
    let lastEvent = null
    ydoc.onSubdocs(event => {
        lastEvent = event
    })
    yarray.insert(0, [subdoc1])
    t.assert(subdoc1.shouldLoad)
    t.assert(subdoc1.autoLoad)
    t.assert(lastEvent !== null && lastEvent.loaded.has(subdoc1))
    t.assert(lastEvent !== null && lastEvent.added.has(subdoc1))
    // destroy and check whether lastEvent adds it again to added (it shouldn't)
    subdoc1.destroy()
    const subdoc2 = yarray.get(0)
    t.assert(subdoc1 !== subdoc2)
    t.assert(lastEvent !== null && lastEvent.added.has(subdoc2))
    t.assert(lastEvent !== null && !lastEvent.loaded.has(subdoc2))
    // load
    subdoc2.load()
    t.assert(lastEvent !== null && !lastEvent.added.has(subdoc2))
    t.assert(lastEvent !== null && lastEvent.loaded.has(subdoc2))
    // apply from remote
    const ydoc2 = new Y.YDoc()
    ydoc2.onSubdocs(event => {
        lastEvent = event
    })
    Y.applyUpdate(ydoc2, Y.encodeStateAsUpdate(ydoc))
    const subdoc3 = ydoc2.getArray('test').get(0)
    t.assert(subdoc1.shouldLoad)
    t.assert(subdoc1.autoLoad)
    t.assert(lastEvent !== null && lastEvent.added.has(subdoc3))
    t.assert(lastEvent !== null && lastEvent.loaded.has(subdoc3))
}