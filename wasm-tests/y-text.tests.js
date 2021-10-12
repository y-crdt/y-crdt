import { exchangeUpdates } from './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {t.TestCase} tc
 */
export const testInserts = tc => {
    const d1 = new Y.YDoc()
    var x = d1.getText('test')

    d1.transact(txn => x.push(txn, "hello!"))
    d1.transact(txn => x.insert(txn, 5, " world"))

    const expected = "hello world!"

    var value = d1.transact(txn => x.toString(txn))
    t.compareStrings(value, expected)

    const d2 = new Y.YDoc(2)
    x = d2.getText('test')

    exchangeUpdates([d1, d2])

    value = d2.transact(txn => x.toString(txn))
    t.compareStrings(value, expected)
}

/**
 * @param {t.TestCase} tc
 */
export const testDeletes = tc => {
    const d1 = new Y.YDoc()
    var x = d1.getText('test')

    d1.transact(txn => x.push(txn, "hello world!"))
    t.compare(x.length, 12)
    d1.transact(txn => x.delete(txn, 5, 6))
    t.compare(x.length, 6)
    d1.transact(txn => x.insert(txn, 5, " Yrs"))
    t.compare(x.length, 10)

    const expected = "hello Yrs!"

    var value = d1.transact(txn => x.toString(txn))
    t.compareStrings(value, expected)

    const d2 = new Y.YDoc(2)
    x = d2.getText('test')

    exchangeUpdates([d1, d2])

    value = d2.transact(txn => x.toString(txn))
    t.compareStrings(value, expected)
}