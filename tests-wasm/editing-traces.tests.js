import * as Y from 'ywasm'
import * as t from 'lib0/testing'
import * as fs from 'fs'
import * as zlib from 'zlib'

/**
 * @param {t.TestCase} tc
 * @param {String} filename
 */
const run = (tc, filename) => {
    const { startContent, endContent, txns } = JSON.parse(
        filename.endsWith('.gz')
            ? zlib.gunzipSync(fs.readFileSync(filename))
            : fs.readFileSync(filename, 'utf-8')
    )
    const doc = new Y.YDoc()
    const text = doc.getText('text')
    if (startContent && startContent !== '') {
        text.push(startContent)
    }
    const start = performance.now()
    for (const {patches} of txns) {
        let txn = doc.writeTransaction()
        for (const [pos, del, chunk] of patches) {
            if (del !== 0) {
                text.delete(pos, del, txn)
            }
            if (chunk && chunk !== '') {
                text.insert(pos, chunk, null, txn)
            }
        }
        txn.commit()
        txn.free()
    }
    const end = performance.now()
    console.log('execution time: ', (end-start), 'milliseconds')

    const content = text.toString()
    t.compareStrings(content, endContent)
}

/**
 * @param {t.TestCase} tc
 */
export const testEditingTraceAutomerge = tc => {
    run(tc, '../assets/editing-traces/sequential_traces/automerge-paper.json.gz')
}

/**
 * @param {t.TestCase} tc
 */
export const testEditingTraceFriendsForever = tc => {
    run(tc, '../assets/editing-traces/sequential_traces/friendsforever_flat.json.gz')
}

/**
 * @param {t.TestCase} tc
 */
export const testEditingTraceRustCode = tc => {
    run(tc, '../assets/editing-traces/sequential_traces/rustcode.json.gz')
}

/**
 * @param {t.TestCase} tc
 */
export const testEditingTraceSephBlog = tc => {
    run(tc, '../assets/editing-traces/sequential_traces/seph-blog1.json.gz')
}

/**
 * @param {t.TestCase} tc
 */
export const testEditingTraceSvelteComponent = tc => {
    run(tc, '../assets/editing-traces/sequential_traces/sveltecomponent.json.gz')
}