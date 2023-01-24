import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {Y.YDoc} ydoc
 * @param {Y.YText} ytext
 */
const checkStickyIndex = (ydoc, ytext) => {
    // test if all positions are encoded and restored correctly
    for (let i = 0; i < ytext.length; i++) {
        // for all types of associations..
        for (let assoc = -1; assoc < 2; assoc++) {
            const rpos = Y.createStickyIndexFromType(ytext, i, assoc)
            const encodedRpos = Y.encodeStickyIndex(rpos)
            const decodedRpos = Y.decodeStickyIndex(encodedRpos)
            const absPos = (Y.createOffsetFromStickyIndex(decodedRpos, ydoc))
            t.assert(absPos.index === i)
            t.assert(absPos.assoc === assoc)
        }
    }
}

/**
 * @param {t.TestCase} tc
 */
export const testStickyIndexCase1 = tc => {
    const ydoc = new Y.YDoc()
    const ytext = ydoc.getText('test')
    ytext.insert(0, '1')
    ytext.insert(0, 'abc')
    ytext.insert(0, 'z')
    ytext.insert(0, 'y')
    ytext.insert(0, 'x')
    checkStickyIndex(ydoc, ytext)
}

/**
 * @param {t.TestCase} tc
 */
export const testStickyIndexCase2 = tc => {
    const ydoc = new Y.YDoc()
    const ytext = ydoc.getText('test')
    ytext.insert(0, 'abc')
    checkStickyIndex(ydoc, ytext)
}

/**
 * @param {t.TestCase} tc
 */
export const testStickyIndexCase3 = tc => {
    const ydoc = new Y.YDoc()
    const ytext = ydoc.getText('test')
    ytext.insert(0, 'abc')
    ytext.insert(0, '1')
    ytext.insert(0, 'xyz')
    checkStickyIndex(ydoc, ytext)
}

/**
 * @param {t.TestCase} tc
 */
export const testStickyIndexCase4 = tc => {
    const ydoc = new Y.YDoc()
    const ytext = ydoc.getText('test')
    ytext.insert(0, '1')
    checkStickyIndex(ydoc, ytext)
}

/**
 * @param {t.TestCase} tc
 */
export const testStickyIndexCase5 = tc => {
    const ydoc = new Y.YDoc()
    const ytext = ydoc.getText('test')
    ytext.insert(0, '2')
    ytext.insert(0, '1')
    checkStickyIndex(ydoc, ytext)
}

/**
 * @param {t.TestCase} tc
 */
export const testStickyIndexCase6 = tc => {
    const ydoc = new Y.YDoc()
    const ytext = ydoc.getText('test')
    checkStickyIndex(ydoc, ytext)
}

/**
 * @param {t.TestCase} tc
 */
export const testStickyIndexAssociationDifference = tc => {
    const ydoc = new Y.YDoc()
    const ytext = ydoc.getText('test')
    ytext.insert(0, '2')
    ytext.insert(0, '1')
    const rposRight = Y.createStickyIndexFromType(ytext, 1, 0)
    const rposLeft = Y.createStickyIndexFromType(ytext, 1, -1)
    ytext.insert(1, 'x')
    const posRight = Y.createOffsetFromStickyIndex(rposRight, ydoc)
    const posLeft = Y.createOffsetFromStickyIndex(rposLeft, ydoc)
    t.assert(posRight != null && posRight.index === 2)
    t.assert(posLeft != null && posLeft.index === 1)
}
