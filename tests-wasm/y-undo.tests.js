import { exchangeUpdates } from './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as t from 'lib0/testing'

/**
 * @param {t.TestCase} tc
 */
export const testUndoText = tc => {
    const d0 = new Y.YDoc({clientID:1})
    const d1 = new Y.YDoc({clientID:2})
    const text0 = d0.getText("test")
    const text1 = d1.getText("test")
    const undoManager = new Y.YUndoManager(d0, text0)

    // items that are added & deleted in the same transaction won't be undo
    text0.insert(0, 'test')
    text0.delete(0, 4)
    undoManager.undo()
    t.assert(text0.toString() === '')

    // follow redone items
    text0.insert(0, 'a')
    undoManager.stopCapturing()
    text0.delete(0, 1)
    undoManager.stopCapturing()
    undoManager.undo()
    t.assert(text0.toString() === 'a')
    undoManager.undo()
    t.assert(text0.toString() === '')

    text0.insert(0, 'abc')
    text1.insert(0, 'xyz')
    exchangeUpdates([d0,d1])
    undoManager.undo()
    t.assert(text0.toString() === 'xyz')
    undoManager.redo()
    t.assert(text0.toString() === 'abcxyz')
    exchangeUpdates([d0,d1])
    text1.delete(0, 1)
    exchangeUpdates([d0,d1])
    undoManager.undo()
    t.assert(text0.toString() === 'xyz')
    undoManager.redo()
    t.assert(text0.toString() === 'bcxyz')
    // test marks
    text0.format(1, 3, { bold: true })
    t.compare(text0.toDelta(), [{ insert: 'b' }, { insert: 'cxy', attributes: { bold: true } }, { insert: 'z' }])
    undoManager.undo()
    t.compare(text0.toDelta(), [{ insert: 'bcxyz' }])
    undoManager.redo()
    t.compare(text0.toDelta(), [{ insert: 'b' }, { insert: 'cxy', attributes: { bold: true } }, { insert: 'z' }])
}

/**
 * Test case to fix #241
 * @param {t.TestCase} tc
 */
export const testDoubleUndo = tc => {
    const doc = new Y.YDoc({clientID:1})
    const text = doc.getText("test")
    text.insert(0, '1221')

    const manager = new Y.YUndoManager(doc, text)

    text.insert(2, '3')
    text.insert(3, '3')

    manager.undo()
    manager.undo()

    text.insert(2, '3')

    t.compareStrings(text.toString(), '12321')
}

/**
 * @param {t.TestCase} tc
 */
export const testUndoMap = tc => {
    const d0 = new Y.YDoc({clientID:1})
    const d1 = new Y.YDoc({clientID:2})
    const map0 = d0.getMap("test")
    const map1 = d1.getMap("test")
    map0.set('a', 0)
    const undoManager = new Y.YUndoManager(d0, map0)
    map0.set('a', 1)
    undoManager.undo()
    t.assert(map0.get('a') === 0)
    undoManager.redo()
    t.assert(map0.get('a') === 1)
    // testing sub-types and if it can restore a whole type
    const subType = new Y.YMap()
    map0.set('a', subType)
    subType.set('x', 42)
    t.compare(map0.toJson(), /** @type {any} */ ({ a: { x: 42 } }))
    undoManager.undo()
    t.assert(map0.get('a') === 1)
    undoManager.redo()
    t.compare(map0.toJson(), /** @type {any} */ ({ a: { x: 42 } }))
    exchangeUpdates([d0,d1])
    // if content is overwritten by another user, undo operations should be skipped
    map1.set('a', 44)
    exchangeUpdates([d0,d1])
    undoManager.undo()
    t.assert(map0.get('a') === 44)
    undoManager.redo()
    t.assert(map0.get('a') === 44)

    // test setting value multiple times
    map0.set('b', 'initial')
    undoManager.stopCapturing()
    map0.set('b', 'val1')
    map0.set('b', 'val2')
    undoManager.stopCapturing()
    undoManager.undo()
    t.assert(map0.get('b') === 'initial')
}

/**
 * @param {t.TestCase} tc
 */
export const testUndoArray = tc => {
    const d0 = new Y.YDoc({clientID:1})
    const d1 = new Y.YDoc({clientID:2})
    const array0 = d0.getArray("test")
    const array1 = d1.getArray("test")
    const undoManager = new Y.YUndoManager(d0, array0)
    array0.insert(0, [1, 2, 3])
    array1.insert(0, [4, 5, 6])
    exchangeUpdates([d0,d1])
    t.compare(array0.toJson(), [1, 2, 3, 4, 5, 6])
    undoManager.undo()
    t.compare(array0.toJson(), [4, 5, 6])
    undoManager.redo()
    t.compare(array0.toJson(), [1, 2, 3, 4, 5, 6])
    exchangeUpdates([d0,d1])
    array1.delete(0, 1) // user1 deletes [1]
    exchangeUpdates([d0,d1])
    undoManager.undo()
    t.compare(array0.toJson(), [4, 5, 6])
    undoManager.redo()
    t.compare(array0.toJson(), [2, 3, 4, 5, 6])
    array0.delete(0, 5)
    // test nested structure
    const ymap = new Y.YMap()
    array0.insert(0, [ymap])
    t.compare(array0.toJson(), [{}])
    undoManager.stopCapturing()
    ymap.set('a', 1)
    t.compare(array0.toJson(), [{ a: 1 }])
    undoManager.undo()
    t.compare(array0.toJson(), [{}])
    undoManager.undo()
    t.compare(array0.toJson(), [2, 3, 4, 5, 6])
    undoManager.redo()
    t.compare(array0.toJson(), [{}])
    undoManager.redo()
    t.compare(array0.toJson(), [{ a: 1 }])
    exchangeUpdates([d0,d1])
    array1.get(0).set('b', 2)
    exchangeUpdates([d0,d1])
    t.compare(array0.toJson(), [{ a: 1, b: 2 }])
    undoManager.undo()
    t.compare(array0.toJson(), [{ b: 2 }])
    undoManager.undo()
    t.compare(array0.toJson(), [2, 3, 4, 5, 6])
    undoManager.redo()
    t.compare(array0.toJson(), [{ b: 2 }])
    undoManager.redo()
    t.compare(array0.toJson(), [{ a: 1, b: 2 }])
}

/**
 * @param {t.TestCase} tc
 */
export const testUndoXml = tc => {
    const d0 = new Y.YDoc({clientID:1})
    const xml0 = d0.getXmlElement("undefined")
    const undoManager = new Y.YUndoManager(d0, xml0)
    const child = xml0.insertXmlElement(0, 'p')
    const textchild = child.insertXmlText(0)
    textchild.insert(0, 'content')
    t.assert(xml0.toString() === '<undefined><p>content</p></undefined>')
    // format textchild and revert that change
    undoManager.stopCapturing()
    textchild.format(3, 4, { bold: {} })
    t.assert(xml0.toString() === '<undefined><p>con<bold>tent</bold></p></undefined>')
    undoManager.undo()
    t.assert(xml0.toString() === '<undefined><p>content</p></undefined>')
    undoManager.redo()
    t.assert(xml0.toString() === '<undefined><p>con<bold>tent</bold></p></undefined>')
    xml0.delete(0, 1)
    t.assert(xml0.toString() === '<undefined></undefined>')
    undoManager.undo()
    t.assert(xml0.toString() === '<undefined><p>con<bold>tent</bold></p></undefined>')
}