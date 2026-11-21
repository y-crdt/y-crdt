/**
 * Interoperability tests between ywasm (Yrs compiled to WebAssembly) and the reference
 * JavaScript implementation - Yjs.
 *
 * Every test produces changes on one side (or on both sides concurrently), exchanges binary
 * updates between ywasm and Yjs documents using both lib0 v1 and v2 encodings and verifies
 * that both implementations converge to the same state.
 */
import './testHelper.js' // eslint-disable-line

import * as Y from 'ywasm'
import * as Yjs from 'yjs'
import * as t from 'lib0/testing'
import * as prng from 'lib0/prng'
import * as decoding from 'lib0/decoding'

const ENCODINGS = [
    {name: 'v1', v2: false},
    {name: 'v2', v2: true},
]

/**
 * @param {Y.YDoc|Yjs.Doc} doc
 * @return {boolean}
 */
const isYjs = doc => doc instanceof Yjs.Doc

/**
 * @param {Y.YDoc|Yjs.Doc} doc
 * @return {Uint8Array}
 */
const encodeStateVector = doc => isYjs(doc) ? Yjs.encodeStateVector(doc) : Y.encodeStateVector(doc)

/**
 * @param {Y.YDoc|Yjs.Doc} doc
 * @param {Uint8Array|undefined} stateVector
 * @param {boolean} v2
 * @return {Uint8Array}
 */
const encodeStateAsUpdate = (doc, stateVector, v2) => {
    if (isYjs(doc)) {
        return v2 ? Yjs.encodeStateAsUpdateV2(doc, stateVector) : Yjs.encodeStateAsUpdate(doc, stateVector)
    } else {
        return v2 ? Y.encodeStateAsUpdateV2(doc, stateVector) : Y.encodeStateAsUpdate(doc, stateVector)
    }
}

/**
 * @param {Y.YDoc|Yjs.Doc} doc
 * @param {Uint8Array} update
 * @param {boolean} v2
 */
const applyUpdate = (doc, update, v2) => {
    if (isYjs(doc)) {
        v2 ? Yjs.applyUpdateV2(doc, update, 'interop') : Yjs.applyUpdate(doc, update, 'interop')
    } else {
        v2 ? Y.applyUpdateV2(doc, update, 'interop') : Y.applyUpdate(doc, update, 'interop')
    }
}

/**
 * Exchanges missing updates between all pairs of documents, using a state vector of a receiver.
 *
 * @param {Array<Y.YDoc|Yjs.Doc>} docs
 * @param {boolean} v2
 */
const sync = (docs, v2) => {
    for (const d1 of docs) {
        for (const d2 of docs) {
            if (d1 !== d2) {
                const sv = encodeStateVector(d1)
                const update = encodeStateAsUpdate(d2, sv, v2)
                applyUpdate(d1, update, v2)
            }
        }
    }
}

/**
 * Sends a full document state from `from` to `to`.
 *
 * @param {Y.YDoc|Yjs.Doc} from
 * @param {Y.YDoc|Yjs.Doc} to
 * @param {boolean} v2
 */
const send = (from, to, v2) => applyUpdate(to, encodeStateAsUpdate(from, undefined, v2), v2)

/**
 * @param {number} clientID
 * @return {Yjs.Doc}
 */
const newYjsDoc = clientID => {
    const doc = new Yjs.Doc()
    doc.clientID = clientID
    return doc
}

/**
 * @param {number} clientID
 * @return {Y.YDoc}
 */
const newYwasmDoc = clientID => new Y.YDoc({clientID})

/**
 * Runs a given test body once for every supported update encoding.
 *
 * @param {function(boolean, string):void} f
 */
const forEachEncoding = f => {
    for (const {name, v2} of ENCODINGS) {
        f(v2, name)
    }
}

// --------------------------------------------------------------------------------------------
// YText
// --------------------------------------------------------------------------------------------

/**
 * @param {t.TestCase} tc
 */
export const testTextYjsToYwasm = tc => {
    forEachEncoding((v2, enc) => {
        const yjs = newYjsDoc(1)
        const ytext = yjs.getText('text')
        ytext.insert(0, 'hello world!')
        ytext.delete(5, 6)
        ytext.insert(5, ' Yrs')

        const ywasm = newYwasmDoc(2)
        const wtext = ywasm.getText('text')
        send(yjs, ywasm, v2)

        t.compareStrings(wtext.toString(), 'hello Yrs!', enc)
        t.compare(wtext.length(), ytext.length, enc)
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testTextYwasmToYjs = tc => {
    forEachEncoding((v2, enc) => {
        const ywasm = newYwasmDoc(1)
        const wtext = ywasm.getText('text')
        wtext.insert(0, 'hello world!')
        wtext.delete(5, 6)
        wtext.insert(5, ' Yjs')

        const yjs = newYjsDoc(2)
        const ytext = yjs.getText('text')
        send(ywasm, yjs, v2)

        t.compareStrings(ytext.toString(), 'hello Yjs!', enc)
        t.compare(ytext.length, wtext.length(), enc)
    })
}

/**
 * Text using characters outside of Basic Multilingual Plane (surrogate pairs in UTF-16)
 * and multi-byte UTF-8 characters. Offsets on both sides are expected to be counted in UTF-16
 * code units.
 *
 * @param {t.TestCase} tc
 */
export const testTextUnicode = tc => {
    forEachEncoding((v2, enc) => {
        const yjs = newYjsDoc(1)
        const ywasm = newYwasmDoc(2)
        const ytext = yjs.getText('text')
        const wtext = ywasm.getText('text')

        ytext.insert(0, 'こんにちは😀世界')
        sync([yjs, ywasm], v2)
        t.compareStrings(wtext.toString(), ytext.toString(), enc)
        t.compare(wtext.length(), ytext.length, enc)

        // insert right after the emoji (which takes 2 UTF-16 code units)
        wtext.insert(7, '🎉')
        wtext.delete(0, 1)
        sync([yjs, ywasm], v2)
        t.compareStrings(ytext.toString(), 'んにちは😀🎉世界', enc)

        ytext.delete(4, 4) // remove both emojis
        ytext.insert(4, 'ü')
        sync([yjs, ywasm], v2)
        t.compareStrings(wtext.toString(), 'んにちはü世界', enc)
        t.compare(wtext.length(), ytext.length, enc)
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testTextFormatting = tc => {
    forEachEncoding((v2, enc) => {
        // Yjs -> ywasm
        {
            const yjs = newYjsDoc(1)
            const ytext = yjs.getText('text')
            ytext.insert(0, 'abc', {bold: true})
            ytext.insert(3, 'def')
            ytext.insert(6, 'ghi', {italic: true, color: '#ff0000'})
            ytext.format(1, 4, {underline: true})
            ytext.format(7, 1, {italic: null})

            const ywasm = newYwasmDoc(2)
            send(yjs, ywasm, v2)
            t.compare(ywasm.getText('text').toDelta(), ytext.toDelta(), enc)
        }
        // ywasm -> Yjs
        {
            const ywasm = newYwasmDoc(1)
            const wtext = ywasm.getText('text')
            wtext.insert(0, 'abc', {bold: true})
            wtext.insert(3, 'def')
            wtext.insert(6, 'ghi', {italic: true, color: '#ff0000'})
            wtext.format(1, 4, {underline: true})
            wtext.format(7, 1, {italic: null})

            const yjs = newYjsDoc(2)
            send(ywasm, yjs, v2)
            t.compare(yjs.getText('text').toDelta(), wtext.toDelta(), enc)
        }
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testTextEmbeds = tc => {
    forEachEncoding((v2, enc) => {
        const yjs = newYjsDoc(1)
        const ywasm = newYwasmDoc(2)
        const ytext = yjs.getText('text')
        const wtext = ywasm.getText('text')

        ytext.insert(0, 'ab')
        ytext.insertEmbed(1, {image: 'https://example.com/a.png'}, {width: 100})
        sync([yjs, ywasm], v2)
        t.compare(wtext.toDelta(), ytext.toDelta(), enc)

        wtext.insertEmbed(3, {formula: 'e=mc^2'})
        sync([yjs, ywasm], v2)
        t.compare(ytext.toDelta(), [
            {insert: 'a'},
            {insert: {image: 'https://example.com/a.png'}, attributes: {width: 100}},
            {insert: 'b'},
            {insert: {formula: 'e=mc^2'}},
        ], enc)
        t.compare(wtext.toDelta(), ytext.toDelta(), enc)
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testTextApplyDelta = tc => {
    forEachEncoding((v2, enc) => {
        const yjs = newYjsDoc(1)
        const ywasm = newYwasmDoc(2)
        const ytext = yjs.getText('text')
        const wtext = ywasm.getText('text')

        ytext.applyDelta([{insert: 'Hello'}, {insert: ' world', attributes: {bold: true}}])
        sync([yjs, ywasm], v2)
        wtext.applyDelta([{retain: 6}, {retain: 5, attributes: {bold: null, italic: true}}, {insert: '!'}])
        sync([yjs, ywasm], v2)

        const expected = [
            {insert: 'Hello'},
            {insert: ' ', attributes: {bold: true}},
            {insert: 'world', attributes: {italic: true}},
            {insert: '!'},
        ]
        t.compare(ytext.toDelta(), expected, enc)
        t.compare(wtext.toDelta(), expected, enc)
    })
}

// --------------------------------------------------------------------------------------------
// YMap
// --------------------------------------------------------------------------------------------

const PRIMITIVES = {
    string: 'value',
    emptyString: '',
    integer: 42,
    negative: -7,
    bigInteger: 2 ** 40,
    float: 3.1415,
    boolTrue: true,
    boolFalse: false,
    nil: null,
    array: [1, 'two', false, null, [3]],
    object: {nested: {deep: 'value'}, list: [1, 2, 3]},
}

/**
 * @param {t.TestCase} tc
 */
export const testMapPrimitivesYjsToYwasm = tc => {
    forEachEncoding((v2, enc) => {
        const yjs = newYjsDoc(1)
        const ymap = yjs.getMap('map')
        for (const [key, value] of Object.entries(PRIMITIVES)) {
            ymap.set(key, value)
        }
        ymap.set('binary', new Uint8Array([1, 2, 3, 255]))
        ymap.set('removed', 'x')
        ymap.delete('removed')

        const ywasm = newYwasmDoc(2)
        send(yjs, ywasm, v2)
        const wmap = ywasm.getMap('map')

        t.compare(wmap.length(), ymap.size, enc)
        for (const [key, value] of Object.entries(PRIMITIVES)) {
            t.compare(wmap.get(key), value, `${enc}: ${key}`)
        }
        t.compare(wmap.get('binary'), new Uint8Array([1, 2, 3, 255]), enc)
        t.compare(wmap.get('removed'), undefined, enc)
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testMapPrimitivesYwasmToYjs = tc => {
    forEachEncoding((v2, enc) => {
        const ywasm = newYwasmDoc(1)
        const wmap = ywasm.getMap('map')
        for (const [key, value] of Object.entries(PRIMITIVES)) {
            wmap.set(key, value)
        }
        wmap.set('binary', new Uint8Array([1, 2, 3, 255]))
        wmap.set('removed', 'x')
        wmap.delete('removed')

        const yjs = newYjsDoc(2)
        send(ywasm, yjs, v2)
        const ymap = yjs.getMap('map')

        t.compare(ymap.size, wmap.length(), enc)
        for (const [key, value] of Object.entries(PRIMITIVES)) {
            t.compare(ymap.get(key), value, `${enc}: ${key}`)
        }
        t.compare(ymap.get('binary'), new Uint8Array([1, 2, 3, 255]), enc)
        t.compare(ymap.has('removed'), false, enc)
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testMapNestedTypes = tc => {
    forEachEncoding((v2, enc) => {
        // Yjs -> ywasm
        {
            const yjs = newYjsDoc(1)
            const ymap = yjs.getMap('map')
            const inner = new Yjs.Map()
            ymap.set('map', inner)
            inner.set('key', 'value')
            const arr = new Yjs.Array()
            ymap.set('array', arr)
            arr.insert(0, [1, 2, 3])
            const text = new Yjs.Text()
            ymap.set('text', text)
            text.insert(0, 'hello')

            const ywasm = newYwasmDoc(2)
            send(yjs, ywasm, v2)
            const wmap = ywasm.getMap('map')
            t.compare(wmap.get('map').toJson(), {key: 'value'}, enc)
            t.compare(wmap.get('array').toJson(), [1, 2, 3], enc)
            t.compareStrings(wmap.get('text').toString(), 'hello', enc)
        }
        // ywasm -> Yjs
        {
            const ywasm = newYwasmDoc(1)
            const wmap = ywasm.getMap('map')
            wmap.set('map', new Y.YMap({key: 'value'}))
            wmap.set('array', new Y.YArray([1, 2, 3]))
            wmap.set('text', new Y.YText('hello'))
            wmap.get('array').push([4])

            const yjs = newYjsDoc(2)
            send(ywasm, yjs, v2)
            const ymap = yjs.getMap('map')
            t.assert(ymap.get('map') instanceof Yjs.Map, enc)
            t.assert(ymap.get('array') instanceof Yjs.Array, enc)
            t.assert(ymap.get('text') instanceof Yjs.Text, enc)
            t.compare(ymap.toJSON(), {map: {key: 'value'}, array: [1, 2, 3, 4], text: 'hello'}, enc)
        }
    })
}

/**
 * Concurrent writes to the same key must be resolved in the same way by both implementations.
 *
 * @param {t.TestCase} tc
 */
export const testMapConcurrentConflict = tc => {
    forEachEncoding((v2, enc) => {
        for (const [yjsID, ywasmID] of [[1, 2], [2, 1]]) {
            const yjs = newYjsDoc(yjsID)
            const ywasm = newYwasmDoc(ywasmID)
            yjs.getMap('map').set('key', 'yjs')
            yjs.getMap('map').set('yjs-only', 1)
            ywasm.getMap('map').set('key', 'ywasm')
            ywasm.getMap('map').set('ywasm-only', 2)

            sync([yjs, ywasm], v2)

            const yjsJson = yjs.getMap('map').toJSON()
            const ywasmJson = ywasm.getMap('map').toJson()
            t.compare(ywasmJson, yjsJson, enc)
            t.compare(Object.keys(yjsJson).length, 3, enc)
        }
    })
}

// --------------------------------------------------------------------------------------------
// YArray
// --------------------------------------------------------------------------------------------

/**
 * @param {t.TestCase} tc
 */
export const testArray = tc => {
    forEachEncoding((v2, enc) => {
        const yjs = newYjsDoc(1)
        const ywasm = newYwasmDoc(2)
        const yarr = yjs.getArray('array')
        const warr = ywasm.getArray('array')

        yarr.insert(0, [1, 'two', {three: 3}, [4], null, true])
        sync([yjs, ywasm], v2)
        t.compare(warr.toJson(), yarr.toJSON(), enc)

        warr.delete(1, 2)
        warr.insert(1, ['inserted by ywasm'])
        warr.push([7, 8, 9])
        sync([yjs, ywasm], v2)
        t.compare(yarr.toJSON(), [1, 'inserted by ywasm', [4], null, true, 7, 8, 9], enc)

        yarr.delete(0, 1)
        yarr.push([new Uint8Array([10, 20])])
        sync([yjs, ywasm], v2)
        // binary content is compared separately, as ywasm `toJson` maps Uint8Array into plain arrays
        t.compare(warr.length(), yarr.length, enc)
        t.compare(warr.toJson().slice(0, -1), yarr.toJSON().slice(0, -1), enc)
        t.compare(warr.get(warr.length() - 1), new Uint8Array([10, 20]), enc)
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testArrayNestedTypes = tc => {
    forEachEncoding((v2, enc) => {
        const yjs = newYjsDoc(1)
        const ywasm = newYwasmDoc(2)
        const yarr = yjs.getArray('array')
        const warr = ywasm.getArray('array')

        const ymap = new Yjs.Map()
        yarr.insert(0, [ymap])
        ymap.set('a', 1)
        sync([yjs, ywasm], v2)

        warr.get(0).set('b', 2)
        warr.push([new Y.YArray(['x', 'y'])])
        sync([yjs, ywasm], v2)

        t.compare(yarr.toJSON(), [{a: 1, b: 2}, ['x', 'y']], enc)
        yarr.get(1).insert(1, ['z'])
        sync([yjs, ywasm], v2)
        t.compare(warr.toJson(), [{a: 1, b: 2}, ['x', 'z', 'y']], enc)
    })
}

// NOTE: YArray.move is not covered here. Move operation (ContentMove) is a Yrs extension that
// is not supported by Yjs v13, so updates containing moved elements cannot be read by Yjs.

// --------------------------------------------------------------------------------------------
// XML types
// --------------------------------------------------------------------------------------------

/**
 * @param {t.TestCase} tc
 */
export const testXmlYjsToYwasm = tc => {
    forEachEncoding((v2, enc) => {
        const yjs = newYjsDoc(1)
        const frag = yjs.getXmlFragment('xml')
        const p = new Yjs.XmlElement('p')
        frag.insert(0, [p])
        p.setAttribute('class', 'paragraph')
        const text = new Yjs.XmlText()
        p.insert(0, [text])
        text.insert(0, 'hello ')
        text.insert(6, 'world', {bold: true})
        const div = new Yjs.XmlElement('div')
        frag.insert(1, [div])
        div.insert(0, [new Yjs.XmlElement('br')])

        const ywasm = newYwasmDoc(2)
        send(yjs, ywasm, v2)
        const wfrag = ywasm.getXmlFragment('xml')

        t.compare(wfrag.length(), 2, enc)
        const wp = wfrag.firstChild()
        t.compareStrings(wp.name(), 'p', enc)
        t.compare(wp.getAttribute('class'), 'paragraph', enc)
        t.compare(wp.firstChild().toDelta(), text.toDelta(), enc)
        t.compareStrings(wfrag.toString(), frag.toString(), enc)
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testXmlYwasmToYjs = tc => {
    forEachEncoding((v2, enc) => {
        const ywasm = newYwasmDoc(1)
        const wfrag = ywasm.getXmlFragment('xml')
        wfrag.push(new Y.YXmlElement('p', {class: 'paragraph'}))
        const wp = wfrag.firstChild()
        wp.push(new Y.YXmlText('hello '))
        wp.firstChild().insert(6, 'world', {bold: true})
        wfrag.push(new Y.YXmlElement('div', {}, [new Y.YXmlElement('br')]))

        const yjs = newYjsDoc(2)
        send(ywasm, yjs, v2)
        const frag = yjs.getXmlFragment('xml')

        t.compare(frag.length, 2, enc)
        const p = frag.get(0)
        t.assert(p instanceof Yjs.XmlElement, enc)
        t.compareStrings(p.nodeName, 'p', enc)
        t.compare(p.getAttribute('class'), 'paragraph', enc)
        t.compare(p.get(0).toDelta(), wp.firstChild().toDelta(), enc)
        t.compareStrings(frag.toString(), wfrag.toString(), enc)
    })
}

// --------------------------------------------------------------------------------------------
// Document-level features
// --------------------------------------------------------------------------------------------

/**
 * Incremental updates emitted by one implementation via 'update' events
 * must be applicable by the other one.
 *
 * @param {t.TestCase} tc
 */
export const testIncrementalUpdates = tc => {
    const yjs = newYjsDoc(1)
    const ywasm = newYwasmDoc(2)
    yjs.on('update', (update, origin) => {
        if (origin !== 'interop') {
            Y.applyUpdate(ywasm, update, 'interop')
        }
    })
    ywasm.on('update', (update, origin) => {
        if (origin !== 'interop') {
            Yjs.applyUpdate(yjs, update, 'interop')
        }
    })

    const ytext = yjs.getText('text')
    const wtext = ywasm.getText('text')
    ytext.insert(0, 'abc')
    wtext.insert(3, 'def')
    ytext.delete(1, 1)
    wtext.format(0, 2, {bold: true})
    ywasm.getMap('map').set('k', 'v')
    yjs.getArray('array').insert(0, [1, 2])

    t.compareStrings(ytext.toString(), 'acdef')
    t.compare(wtext.toDelta(), ytext.toDelta())
    t.compare(yjs.getMap('map').toJSON(), {k: 'v'})
    t.compare(ywasm.getArray('array').toJson(), [1, 2])
}

/**
 * State vectors and partial (diff) updates produced by one implementation
 * must be understood by the other one.
 *
 * @param {t.TestCase} tc
 */
export const testStateVectorDiff = tc => {
    forEachEncoding((v2, enc) => {
        const yjs = newYjsDoc(1)
        const ywasm = newYwasmDoc(2)
        yjs.getText('text').insert(0, 'abc')
        sync([yjs, ywasm], v2)

        // ywasm produces more changes, Yjs requests only the missing ones
        ywasm.getText('text').insert(3, 'def')
        const sv = Yjs.encodeStateVector(yjs)
        const diff = encodeStateAsUpdate(ywasm, sv, v2)
        const full = encodeStateAsUpdate(ywasm, undefined, v2)
        t.assert(diff.length < full.length, enc)
        applyUpdate(yjs, diff, v2)
        t.compareStrings(yjs.getText('text').toString(), 'abcdef', enc)

        // state vectors should be equal
        t.compare(
            Yjs.decodeStateVector(Y.encodeStateVector(ywasm)),
            Yjs.decodeStateVector(Yjs.encodeStateVector(yjs)),
            enc
        )
    })
}

/**
 * Updates applied out-of-order (with missing dependencies) must be stashed as pending
 * and integrated once missing updates arrive.
 *
 * @param {t.TestCase} tc
 */
export const testOutOfOrderUpdates = tc => {
    forEachEncoding((v2, enc) => {
        const yjs = newYjsDoc(1)
        const updates = []
        yjs.on(v2 ? 'updateV2' : 'update', update => updates.push(update))
        const ytext = yjs.getText('text')
        ytext.insert(0, 'a')
        ytext.insert(1, 'b')
        ytext.insert(2, 'c')
        ytext.delete(0, 1)

        const ywasm = newYwasmDoc(2)
        for (const update of updates.slice().reverse()) {
            applyUpdate(ywasm, update, v2)
        }
        t.compareStrings(ywasm.getText('text').toString(), 'bc', enc)
    })
}

/**
 * Updates produced by ywasm must be processable by Yjs update utilities
 * (merge, diff, format conversion) and vice versa.
 *
 * @param {t.TestCase} tc
 */
export const testUpdateUtilities = tc => {
    const ywasm = newYwasmDoc(1)
    const wtext = ywasm.getText('text')
    const updates = []
    ywasm.on('update', update => updates.push(update))
    wtext.insert(0, 'hello')
    wtext.insert(5, ' world')
    wtext.delete(0, 1)

    // merge updates produced by ywasm using Yjs
    const merged = Yjs.mergeUpdates(updates)
    const yjs = newYjsDoc(2)
    Yjs.applyUpdate(yjs, merged)
    t.compareStrings(yjs.getText('text').toString(), 'ello world')

    // convert ywasm v1 update into v2 using Yjs and apply it back to ywasm
    const v2 = Yjs.convertUpdateFormatV1ToV2(Y.encodeStateAsUpdate(ywasm))
    const ywasm2 = newYwasmDoc(3)
    Y.applyUpdateV2(ywasm2, v2)
    t.compareStrings(ywasm2.getText('text').toString(), 'ello world')

    // convert ywasm v2 update into v1 using Yjs
    const v1 = Yjs.convertUpdateFormatV2ToV1(Y.encodeStateAsUpdateV2(ywasm))
    const yjs2 = newYjsDoc(4)
    Yjs.applyUpdate(yjs2, v1)
    t.compareStrings(yjs2.getText('text').toString(), 'ello world')

    // Yjs diffUpdate on ywasm update
    yjs.getText('text').insert(0, 'H')
    const diff = Yjs.diffUpdate(Yjs.encodeStateAsUpdate(yjs), Y.encodeStateVector(ywasm))
    Y.applyUpdate(ywasm, diff)
    t.compareStrings(wtext.toString(), 'Hello world')
}

/**
 * Garbage collected (deleted) content must be correctly encoded and decoded by both sides.
 *
 * @param {t.TestCase} tc
 */
export const testDeletedContent = tc => {
    forEachEncoding((v2, enc) => {
        const yjs = newYjsDoc(1)
        const yarr = yjs.getArray('array')
        const inner = new Yjs.Map()
        yarr.insert(0, ['a', inner, 'b'])
        inner.set('x', new Yjs.Text('nested'))
        yarr.delete(1, 1)
        yjs.getText('text').insert(0, 'to be removed')
        yjs.getText('text').delete(0, 6)

        const ywasm = newYwasmDoc(2)
        send(yjs, ywasm, v2)
        t.compare(ywasm.getArray('array').toJson(), ['a', 'b'], enc)
        t.compareStrings(ywasm.getText('text').toString(), 'removed', enc)

        // round trip back to a fresh Yjs document
        const yjs2 = newYjsDoc(3)
        send(ywasm, yjs2, v2)
        t.compare(yjs2.getArray('array').toJSON(), ['a', 'b'], enc)
        t.compareStrings(yjs2.getText('text').toString(), 'removed', enc)
        t.compare(
            Yjs.decodeStateVector(Yjs.encodeStateVector(yjs2)),
            Yjs.decodeStateVector(Yjs.encodeStateVector(yjs)),
            enc
        )
    })
}

/**
 * @param {t.TestCase} tc
 */
export const testSubdocuments = tc => {
    forEachEncoding((v2, enc) => {
        const yjs = newYjsDoc(1)
        const subdoc = new Yjs.Doc({guid: 'subdoc-guid'})
        yjs.getMap('map').set('subdoc', subdoc)

        const ywasm = newYwasmDoc(2)
        send(yjs, ywasm, v2)
        const wsubdoc = ywasm.getMap('map').get('subdoc')
        t.assert(wsubdoc instanceof Y.YDoc, enc)
        t.compareStrings(wsubdoc.guid, 'subdoc-guid', enc)

        ywasm.getMap('map').set('subdoc2', new Y.YDoc({guid: 'another-guid'}))
        const yjs2 = newYjsDoc(3)
        send(ywasm, yjs2, v2)
        const ysubdoc2 = yjs2.getMap('map').get('subdoc2')
        t.assert(ysubdoc2 instanceof Yjs.Doc, enc)
        t.compareStrings(ysubdoc2.guid, 'another-guid', enc)
        t.compareStrings(yjs2.getMap('map').get('subdoc').guid, 'subdoc-guid', enc)
    })
}

/**
 * @return {{yjs: Yjs.Doc, ywasm: Y.YDoc}}
 */
const snapshotFixture = () => {
    const yjs = new Yjs.Doc({gc: false})
    yjs.clientID = 1
    yjs.getText('text').insert(0, 'hello')
    yjs.getText('text').delete(0, 1)
    yjs.getText('text').delete(2, 1)

    const ywasm = new Y.YDoc({clientID: 2, gc: false})
    send(yjs, ywasm, false)
    return {yjs, ywasm}
}

/**
 * Snapshots encoded by one side must be decodable by the other.
 *
 * @param {t.TestCase} tc
 */
export const testSnapshotsV1 = tc => {
    const {yjs, ywasm} = snapshotFixture()
    const ysnap = Yjs.snapshot(yjs)
    const wsnap = Y.snapshot(ywasm)

    t.compare(Y.encodeSnapshotV1(wsnap), Yjs.encodeSnapshot(ysnap))
    t.assert(Y.equalSnapshots(Y.decodeSnapshotV1(Yjs.encodeSnapshot(ysnap)), wsnap))
    t.assert(Yjs.equalSnapshots(Yjs.decodeSnapshot(Y.encodeSnapshotV1(wsnap)), ysnap))
}

/**
 * Yrs encodes v2 snapshots using a full update encoder, which in Yjs corresponds to
 * `encodeSnapshotV2(snapshot, new UpdateEncoderV2())`.
 *
 * @param {t.TestCase} tc
 */
export const testSnapshotsV2UpdateEncoder = tc => {
    const {yjs, ywasm} = snapshotFixture()
    const ysnap = Yjs.snapshot(yjs)
    const wsnap = Y.snapshot(ywasm)

    const yjsEncoded = Yjs.encodeSnapshotV2(ysnap, new Yjs.UpdateEncoderV2())
    const ywasmEncoded = Y.encodeSnapshotV2(wsnap)
    t.compare(ywasmEncoded, yjsEncoded)
    t.assert(Y.equalSnapshots(Y.decodeSnapshotV2(yjsEncoded), wsnap))
    t.assert(Yjs.equalSnapshots(
        Yjs.decodeSnapshotV2(ywasmEncoded, new Yjs.UpdateDecoderV2(decoding.createDecoder(ywasmEncoded))),
        ysnap
    ))
}

/**
 * KNOWN INCOMPATIBILITY: by default Yjs `encodeSnapshotV2`/`decodeSnapshotV2` use a lightweight
 * `DSEncoderV2`/`DSDecoderV2`, which produce a different binary format than the one used by Yrs.
 * This test is skipped until both implementations agree on a default v2 snapshot encoding.
 *
 * @param {t.TestCase} tc
 */
export const testSnapshotsV2Default = tc => {
    t.skip()
    const {yjs, ywasm} = snapshotFixture()
    const ysnap = Yjs.snapshot(yjs)
    const wsnap = Y.snapshot(ywasm)

    t.compare(Y.encodeSnapshotV2(wsnap), Yjs.encodeSnapshotV2(ysnap))
    t.assert(Y.equalSnapshots(Y.decodeSnapshotV2(Yjs.encodeSnapshotV2(ysnap)), wsnap))
    t.assert(Yjs.equalSnapshots(Yjs.decodeSnapshotV2(Y.encodeSnapshotV2(wsnap)), ysnap))
}

// --------------------------------------------------------------------------------------------
// Concurrent editing / convergence
// --------------------------------------------------------------------------------------------

/**
 * @param {Yjs.Doc} yjs
 * @param {Y.YDoc} ywasm
 * @param {string} enc
 */
const assertConverged = (yjs, ywasm, enc) => {
    t.compareStrings(ywasm.getText('text').toString(), yjs.getText('text').toString(), enc)
    t.compare(ywasm.getText('text').toDelta(), yjs.getText('text').toDelta(), enc)
    t.compare(ywasm.getArray('array').toJson(), yjs.getArray('array').toJSON(), enc)
    t.compare(ywasm.getMap('map').toJson(), yjs.getMap('map').toJSON(), enc)
    t.compare(
        Yjs.decodeStateVector(Y.encodeStateVector(ywasm)),
        Yjs.decodeStateVector(Yjs.encodeStateVector(yjs)),
        enc
    )
}

/**
 * @param {t.TestCase} tc
 */
export const testConcurrentText = tc => {
    forEachEncoding((v2, enc) => {
        for (const [yjsID, ywasmID] of [[1, 2], [2, 1]]) {
            const yjs = newYjsDoc(yjsID)
            const ywasm = newYwasmDoc(ywasmID)
            yjs.getText('text').insert(0, 'abcdef')
            sync([yjs, ywasm], v2)

            // concurrent inserts at the same position and overlapping deletes
            yjs.getText('text').insert(3, 'XXX')
            yjs.getText('text').delete(0, 2)
            ywasm.getText('text').insert(3, 'YYY')
            ywasm.getText('text').delete(1, 3)
            ywasm.getText('text').format(0, 2, {bold: true})
            sync([yjs, ywasm], v2)

            assertConverged(yjs, ywasm, enc)
        }
    })
}

/**
 * Two Yjs documents and two ywasm documents editing concurrently.
 *
 * @param {t.TestCase} tc
 */
export const testConcurrentManyPeers = tc => {
    forEachEncoding((v2, enc) => {
        const docs = [newYjsDoc(1), newYwasmDoc(2), newYjsDoc(3), newYwasmDoc(4)]
        docs.forEach((doc, i) => {
            if (isYjs(doc)) {
                doc.getText('text').insert(0, `yjs${i}`)
                doc.getArray('array').insert(0, [i])
                doc.getMap('map').set('key', i)
            } else {
                doc.getText('text').insert(0, `ywasm${i}`)
                doc.getArray('array').insert(0, [i])
                doc.getMap('map').set('key', i)
            }
        })
        sync(docs, v2)
        const [yjs1, ywasm2, yjs3, ywasm4] = docs
        assertConverged(yjs1, ywasm2, enc)
        assertConverged(yjs3, ywasm4, enc)
        assertConverged(yjs1, ywasm4, enc)
    })
}

/**
 * @param {prng.PRNG} gen
 * @param {Yjs.Doc} doc
 */
const randomYjsOp = (gen, doc) => {
    const text = doc.getText('text')
    const array = doc.getArray('array')
    const map = doc.getMap('map')
    switch (prng.int32(gen, 0, 6)) {
        case 0: {
            const pos = prng.int32(gen, 0, text.length)
            text.insert(pos, prng.word(gen, 1, 5))
            break
        }
        case 1: {
            if (text.length > 0) {
                const pos = prng.int32(gen, 0, text.length - 1)
                const len = prng.int32(gen, 1, Math.min(3, text.length - pos))
                text.delete(pos, len)
            }
            break
        }
        case 2: {
            if (text.length > 0) {
                const pos = prng.int32(gen, 0, text.length - 1)
                const len = prng.int32(gen, 1, text.length - pos)
                text.format(pos, len, {bold: prng.bool(gen) ? true : null})
            }
            break
        }
        case 3: {
            const pos = prng.int32(gen, 0, array.length)
            array.insert(pos, [prng.int32(gen, 0, 1000)])
            break
        }
        case 4: {
            if (array.length > 0) {
                array.delete(prng.int32(gen, 0, array.length - 1), 1)
            }
            break
        }
        case 5: {
            map.set(prng.oneOf(gen, ['a', 'b', 'c']), prng.word(gen, 1, 3))
            break
        }
        default: {
            map.delete(prng.oneOf(gen, ['a', 'b', 'c']))
        }
    }
}

/**
 * @param {prng.PRNG} gen
 * @param {Y.YDoc} doc
 */
const randomYwasmOp = (gen, doc) => {
    const text = doc.getText('text')
    const array = doc.getArray('array')
    const map = doc.getMap('map')
    switch (prng.int32(gen, 0, 6)) {
        case 0: {
            const pos = prng.int32(gen, 0, text.length())
            text.insert(pos, prng.word(gen, 1, 5))
            break
        }
        case 1: {
            const length = text.length()
            if (length > 0) {
                const pos = prng.int32(gen, 0, length - 1)
                const len = prng.int32(gen, 1, Math.min(3, length - pos))
                text.delete(pos, len)
            }
            break
        }
        case 2: {
            const length = text.length()
            if (length > 0) {
                const pos = prng.int32(gen, 0, length - 1)
                const len = prng.int32(gen, 1, length - pos)
                text.format(pos, len, {bold: prng.bool(gen) ? true : null})
            }
            break
        }
        case 3: {
            const pos = prng.int32(gen, 0, array.length())
            array.insert(pos, [prng.int32(gen, 0, 1000)])
            break
        }
        case 4: {
            const length = array.length()
            if (length > 0) {
                array.delete(prng.int32(gen, 0, length - 1), 1)
            }
            break
        }
        case 5: {
            map.set(prng.oneOf(gen, ['a', 'b', 'c']), prng.word(gen, 1, 3))
            break
        }
        default: {
            map.delete(prng.oneOf(gen, ['a', 'b', 'c']))
        }
    }
}

/**
 * Randomized concurrent editing between Yjs and ywasm peers. After every round all peers
 * exchange updates and must converge to an identical state.
 *
 * @param {t.TestCase} tc
 */
export const testRandomizedConvergence = tc => {
    forEachEncoding((v2, enc) => {
        const gen = tc.prng
        const yjsDocs = [newYjsDoc(1), newYjsDoc(3)]
        const ywasmDocs = [newYwasmDoc(2), newYwasmDoc(4)]
        for (let round = 0; round < 30; round++) {
            for (const doc of yjsDocs) {
                const ops = prng.int32(gen, 1, 5)
                for (let i = 0; i < ops; i++) randomYjsOp(gen, doc)
            }
            for (const doc of ywasmDocs) {
                const ops = prng.int32(gen, 1, 5)
                for (let i = 0; i < ops; i++) randomYwasmOp(gen, doc)
            }
            // occasionally sync only part of the peers, to produce more complex concurrent histories
            if (prng.bool(gen)) {
                sync([yjsDocs[0], ywasmDocs[0]], v2)
            } else {
                sync([...yjsDocs, ...ywasmDocs], v2)
                for (const yjs of yjsDocs) {
                    for (const ywasm of ywasmDocs) {
                        assertConverged(yjs, ywasm, `${enc} round ${round}`)
                    }
                }
            }
        }
        sync([...yjsDocs, ...ywasmDocs], v2)
        for (const yjs of yjsDocs) {
            for (const ywasm of ywasmDocs) {
                assertConverged(yjs, ywasm, enc)
            }
        }
    })
}
