import {exchangeUpdates} from './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {t.TestCase} tc
 */
export const testInserts = tc => {
    const d1 = new Y.YDoc()
    var x = d1.getText('test')

    x.push("hello!")
    x.insert(5, " world")

    const expected = "hello world!"

    var value = x.toString()
    t.compareStrings(value, expected)

    const d2 = new Y.YDoc({clientID: 2})
    x = d2.getText('test')

    exchangeUpdates([d1, d2])

    value = x.toString()
    t.compareStrings(value, expected)
}

/**
 * @param {t.TestCase} tc
 */
export const testDeletes = tc => {
    const d1 = new Y.YDoc()
    var x = d1.getText('test')

    x.push("hello world!")
    t.compare(x.length(), 12)
    x.delete(5, 6)
    t.compare(x.length(), 6)
    x.insert(5, " Yrs")
    t.compare(x.length(), 10)

    const expected = "hello Yrs!"

    var value = x.toString()
    t.compareStrings(value, expected)

    const d2 = new Y.YDoc({clientID: 2})
    x = d2.getText('test')

    exchangeUpdates([d1, d2])

    value = x.toString()
    t.compareStrings(value, expected)
}

/**
 * @param {t.TestCase} tc
 */
export const testObserver = tc => {
    const d1 = new Y.YDoc()
    /**
     * @param {Y.YText} tc
     */
    const x = d1.getText('test')
    let target = null
    let delta = null
    let callback = e => {
        target = e.target
        delta = e.delta
    }
    x.observe(callback)

    // insert initial data to an empty YText
    x.insert(0, 'abcd')
    t.compare(target.toJson(), x.toJson())
    t.compare(delta, [{insert: 'abcd'}])
    target = null
    delta = null

    // remove 2 chars from the middle
    x.delete(1, 2)
    t.compare(target.toJson(), x.toJson())
    t.compare(delta, [{retain: 1}, {delete: 2}])
    target = null
    delta = null

    // insert new item in the middle
    x.insert(1, 'e', {bold: true})
    t.compare(target.toJson(), x.toJson())
    t.compare(delta, [{retain: 1}, {insert: 'e', attributes: {bold: true}}])
    target = null
    delta = null

    // remove formatting
    x.format(1, 1, {bold: null})
    t.compare(target.toJson(), x.toJson())
    t.compare(delta, [{retain: 1}, {retain: 2, attributes: {bold: null}}])
    target = null
    delta = null

    // free the observer and make sure that callback is no longer called
    t.assert(x.unobserve(callback), 'unobserve failed')
    x.insert(1, 'fgh')
    t.compare(target, null)
    t.compare(delta, null)
}

/**
 * @param {t.TestCase} tc
 */
export const testToDeltaEmbedAttributes = tc => {
    const d1 = new Y.YDoc()
    const text = d1.getText('test')

    let delta = null
    let origin = null
    text.observe(e => {
        delta = e.delta
        origin = e.origin
    })

    d1.transact(txn => {
        text.insert(0, 'ab', {bold: true}, txn)
        text.insertEmbed(1, {image: 'imageSrc.png'}, {width: 100}, txn)
    }, 'TEST_ORIGIN')
    t.compare(delta, [
        {insert: 'a', attributes: {bold: true}},
        {insert: {image: 'imageSrc.png'}, attributes: {width: 100}},
        {insert: 'b', attributes: {bold: true}}
    ])
    t.compare(origin, 'TEST_ORIGIN')
}


/**
 * @param {t.TestCase} tc
 */
export const testMultilineFormat = tc => {
    const ydoc = new Y.YDoc()
    const testText = ydoc.getText('test')
    testText.insert(0, 'Test\nMulti-line\nFormatting')
    testText.applyDelta([
        {retain: 4, attributes: {bold: true}},
        {retain: 1}, // newline character
        {retain: 10, attributes: {bold: true}},
        {retain: 1}, // newline character
        {retain: 10, attributes: {bold: true}}
    ])
    t.compare(testText.toDelta(), [
        {insert: 'Test', attributes: {bold: true}},
        {insert: '\n'},
        {insert: 'Multi-line', attributes: {bold: true}},
        {insert: '\n'},
        {insert: 'Formatting', attributes: {bold: true}}
    ])
}

/**
 * @param {t.TestCase} tc
 */
export const testNotMergeEmptyLinesFormat = tc => {
    const ydoc = new Y.YDoc()
    const testText = ydoc.getText('test')
    testText.applyDelta([
        {insert: 'Text'},
        {insert: '\n', attributes: {title: true}},
        {insert: '\nText'},
        {insert: '\n', attributes: {title: true}}
    ])
    t.compare(testText.toDelta(), [
        {insert: 'Text'},
        {insert: '\n', attributes: {title: true}},
        {insert: '\nText'},
        {insert: '\n', attributes: {title: true}}
    ])
}

/**
 * @param {t.TestCase} tc
 */
export const testGetDeltaWithEmbeds = tc => {
    const ydoc = new Y.YDoc()
    const text = ydoc.getText('test')
    text.applyDelta([{
        insert: {linebreak: 's'}
    }])
    t.compare(text.toDelta(), [{
        insert: {linebreak: 's'}
    }])
}

/**
 * @param {t.TestCase} tc
 */
export const testTypesAsEmbed = tc => {
    const doc0 = new Y.YDoc({clientID: 1})
    const text0 = doc0.getText('test')
    const doc1 = new Y.YDoc({clientID: 2})
    const text1 = doc1.getText('test')
    text0.applyDelta([{
        insert: new Y.YMap([['key', 'val']])
    }])
    t.compare(text0.toDelta()[0].insert.toJson(), {key: 'val'})
    let firedEvent = false
    text1.observe(event => {
        const d = event.delta
        t.assert(d.length === 1)
        t.compare(d.map(x => (x.insert).toJson()), [{key: 'val'}])
        firedEvent = true
    })
    exchangeUpdates([doc0, doc1])
    const delta = text1.toDelta()
    t.assert(delta.length === 1)
    t.compare(delta[0].insert.toJson(), {key: 'val'})
    t.assert(firedEvent, 'fired the event observer containing a Type-Embed')
}

/**
 * @param {t.TestCase} tc
 */
export const testSnapshot = tc => {
    const doc0 = new Y.YDoc({clientID: 1, gc: false})
    const text0 = doc0.getText('test')
    text0.applyDelta([{
        insert: 'abcd'
    }])
    const snapshot1 = Y.snapshot(doc0)
    text0.applyDelta([{
        retain: 1
    }, {
        insert: 'x'
    }, {
        delete: 1
    }])
    const snapshot2 = Y.snapshot(doc0)
    text0.applyDelta([{
        retain: 2
    }, {
        delete: 3
    }, {
        insert: 'x'
    }, {
        delete: 1
    }])
    const state1 = text0.toDelta(snapshot1)
    t.compare(state1, [{insert: 'abcd'}])
    const state2 = text0.toDelta(snapshot2)
    t.compare(state2, [{insert: 'axcd'}])
    const state2Diff = text0.toDelta(snapshot2, snapshot1)
    // @ts-ignore Remove userid info
    state2Diff.forEach(v => {
        if (v.attributes && v.attributes.ychange) {
            delete v.attributes.ychange.user
        }
    })
    t.compare(state2Diff, [{insert: 'a'}, {insert: 'x', attributes: {ychange: {type: 'added'}}}, {
        insert: 'b',
        attributes: {ychange: {type: 'removed'}}
    }, {insert: 'cd'}])
}

/**
 * @param {t.TestCase} tc
 */
export const testSnapshotDeleteAfter = tc => {
    const doc0 = new Y.YDoc({clientID: 1, gc: false})
    const text0 = doc0.getText('test')
    text0.applyDelta([{
        insert: 'abcd'
    }])
    const snapshot1 = Y.snapshot(doc0)
    text0.applyDelta([{
        retain: 4
    }, {
        insert: 'e'
    }])
    const state1 = text0.toDelta(snapshot1)
    t.compare(state1, [{insert: 'abcd'}])
}
