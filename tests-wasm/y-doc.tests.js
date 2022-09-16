import { exchangeUpdates } from './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {t.TestCase} tc
 */
export const testOnUpdate = tc => {
    const d1 = new Y.YDoc(1)
    const text1 = d1.getText('text')
    d1.transact(txn => text1.insert(txn, 0, 'hello'))
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

    d1.transact(txn => text1.insert(txn, 5, 'world'))
    expected = Y.encodeStateAsUpdate(d1)
    Y.applyUpdate(d2, expected)

    t.compare(text1.toString(), text2.toString())
    t.compare(actual, null) // subscription was release, we should get no more updates
}

/**
 * @param {t.TestCase} tccls
 */
export const testOnUpdateV2 = tc => {
    const d1 = new Y.YDoc(1)
    const text1 = d1.getText('text')
    d1.transact(txn => text1.insert(txn, 0, 'hello'))
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

    d1.transact(txn => text1.insert(txn, 5, 'world'))
    expected = Y.encodeStateAsUpdateV2(d1)
    Y.applyUpdateV2(d2, expected)

    t.compare(text1.toString(), text2.toString())
    t.compare(actual, null) // subscription was release, we should get no more updates
}

/**
 * @param {t.TestCase} tc
 */
export const testOnAfterTransaction = tc => {
    const doc = new Y.YDoc(1)
    const text = doc.getText('text')
    let event;
    const sub = doc.onAfterTransaction(e => event = e);

    doc.transact(txn => text.insert(txn, 0, 'hello world'))

    t.compare(event.beforeState, new Map());
    let state = new Map()
    state.set(1n, 11)
    t.compare(event.afterState, state);
    t.compare(event.deleteSet, new Map());

    event = null
    doc.transact(txn => text.delete(txn, 2, 7))

    t.compare(event.beforeState, state);
    t.compare(event.afterState, state);
    state = new Map()
    state.set(1n, [[2,7]])
    t.compare(event.deleteSet, state);

    sub.free()
    event = null
    doc.transact(txn => text.insert(txn, 4, ' the door'))

    t.compare(event, null)
}

/**
 * @param {t.TestCase} tc
 */
export const testSnapshots = tc => {
    const doc = new Y.YDoc(1)
    const text = doc.getText('text')
    doc.transact(txn => text.insert(txn, 0, 'hello'))
    const prev = Y.snapshot(doc)
    doc.transact(txn => text.insert(txn, 5, ' world'))
    const next = Y.snapshot(doc)

    const delta = doc.transact(txn => text.toDelta(txn, next, prev))
    t.compare(delta, [
        { insert: 'hello' },
        { insert: ' world', attributes: { ychange: { type: 'added' } } }
    ])
}

/**
 * @param {t.TestCase} tc
 */
export const testSnapshotState = tc => {
    const d1 = new Y.YDoc(1, false)
    const txt1 = d1.getText('text')
    d1.transact(txn => txt1.insert(txn, 0, 'hello'))
    const prev = Y.snapshot(d1)
    d1.transact(txn => txt1.insert(txn, 5, ' world'))
    const state = Y.encodeStateFromSnapshotV1(d1, prev)

    const d2 = new Y.YDoc(2)
    const txt2 = d2.getText('text')
    Y.applyUpdate(d2, state)

    t.compare(txt2.toString(), 'hello')
}