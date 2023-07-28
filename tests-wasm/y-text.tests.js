import { exchangeUpdates } from './testHelper.js' // eslint-disable-line

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

    const d2 = new  Y.YDoc({clientID:2})
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

    const d2 = new  Y.YDoc({clientID:2})
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
    let observer = x.observe(e => {
        target = e.target
        delta = e.delta
    })

    // insert initial data to an empty YText
    x.insert(0, 'abcd')
    t.compare(target.toJson(), x.toJson())
    t.compare(delta, [{ insert: 'abcd' }])
    target = null
    delta = null

    // remove 2 chars from the middle
    x.delete(1, 2)
    t.compare(target.toJson(), x.toJson())
    t.compare(delta, [{ retain: 1 }, { delete: 2 }])
    target = null
    delta = null

    // insert new item in the middle
    x.insert(1, 'e', { bold: true })
    t.compare(target.toJson(), x.toJson())
    t.compare(delta, [{ retain: 1 }, { insert: 'e', attributes: { bold: true } }])
    target = null
    delta = null

    // remove formatting
    x.format(1, 1, { bold: null })
    t.compare(target.toJson(), x.toJson())
    t.compare(delta, [{ retain: 1 }, { retain: 2, attributes: { bold: null } }])
    target = null
    delta = null

    // free the observer and make sure that callback is no longer called
    observer.free()
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
    let observer = text.observe(e => {
        delta = e.delta
        origin = e.origin
    })

    d1.transact(txn => {
        text.insert(0, 'ab', { bold: true }, txn)
        text.insertEmbed(1, { image: 'imageSrc.png' }, { width: 100 }, txn)
    }, 'TEST_ORIGIN')
    t.compare(delta, [
        { insert: 'a', attributes: { bold: true } },
        { insert: { image: 'imageSrc.png' }, attributes: { width: 100 } },
        { insert: 'b', attributes: { bold: true } }
    ])
    t.compare(origin, 'TEST_ORIGIN')
    observer.free()
}