import * as t from 'lib0/testing'
import * as prng from 'lib0/prng'
import * as encoding from 'lib0/encoding'
import * as decoding from 'lib0/decoding'
import * as object from 'lib0/object'
import * as Y from 'ywasm'

Y.YDoc.prototype.transact = function(callback) {
  let txn = this.beginTransaction();
  try {
      callback(txn);
  } finally {
      txn.commit();
  }
};

/**
 * @typedef {Map<number, number>} StateMap
 */


const syncProtocol = (function () {

    /**
     * Core Yjs defines two message types:
     * • YjsSyncStep1: Includes the State Set of the sending client. When received, the client should reply with YjsSyncStep2.
     * • YjsSyncStep2: Includes all missing structs and the complete delete set. When received, the client is assured that it
     *   received all information from the remote client.
     *
     * In a peer-to-peer network, you may want to introduce a SyncDone message type. Both parties should initiate the connection
     * with SyncStep1. When a client received SyncStep2, it should reply with SyncDone. When the local client received both
     * SyncStep2 and SyncDone, it is assured that it is synced to the remote client.
     *
     * In a client-server model, you want to handle this differently: The client should initiate the connection with SyncStep1.
     * When the server receives SyncStep1, it should reply with SyncStep2 immediately followed by SyncStep1. The client replies
     * with SyncStep2 when it receives SyncStep1. Optionally the server may send a SyncDone after it received SyncStep2, so the
     * client knows that the sync is finished.  There are two reasons for this more elaborated sync model: 1. This protocol can
     * easily be implemented on top of http and websockets. 2. The server shoul only reply to requests, and not initiate them.
     * Therefore it is necesarry that the client initiates the sync.
     *
     * Construction of a message:
     * [messageType : varUint, message definition..]
     *
     * Note: A message does not include information about the room name. This must to be handled by the upper layer protocol!
     *
     * stringify[messageType] stringifies a message definition (messageType is already read from the bufffer)
     */
    const messageYjsSyncStep1 = 0
    const messageYjsSyncStep2 = 1
    const messageYjsUpdate = 2

    /**
     * Create a sync step 1 message based on the state of the current shared document.
     *
     * @param {encoding.Encoder} encoder
     * @param {Y.YDoc} doc
     */
    const writeSyncStep1 = (encoder, doc) => {
        encoding.writeVarUint(encoder, messageYjsSyncStep1)
        const sv = Y.encodeStateVector(doc)
        encoding.writeVarUint8Array(encoder, sv)
    }

    /**
     * @param {encoding.Encoder} encoder
     * @param {Y.YDoc} doc
     * @param {Uint8Array} [encodedStateVector]
     */
    const writeSyncStep2 = (encoder, doc, encodedStateVector) => {
        encoding.writeVarUint(encoder, messageYjsSyncStep2)
        encoding.writeVarUint8Array(encoder, Y.encodeStateAsUpdate(doc, encodedStateVector))
    }

    /**
     * Read SyncStep1 message and reply with SyncStep2.
     *
     * @param {decoding.Decoder} decoder The reply to the received message
     * @param {encoding.Encoder} encoder The received message
     * @param {Y.YDoc} doc
     */
    const readSyncStep1 = (decoder, encoder, doc) =>
        writeSyncStep2(encoder, doc, decoding.readVarUint8Array(decoder))

    /**
     * Read and apply Structs and then DeleteStore to a y instance.
     *
     * @param {decoding.Decoder} decoder
     * @param {Y.YDoc} doc
     * @param {any} transactionOrigin
     */
    const readSyncStep2 = (decoder, doc) => {
        try {
            Y.applyUpdate(doc, decoding.readVarUint8Array(decoder))
        } catch (error) {
            // This catches errors that are thrown by event handlers
            console.error('Caught error while handling a Yjs update', error)
        }
    }

    /**
     * @param {encoding.Encoder} encoder
     * @param {Uint8Array} update
     */
    const writeUpdate = (encoder, update) => {
        encoding.writeVarUint(encoder, messageYjsUpdate)
        encoding.writeVarUint8Array(encoder, update)
    }

    /**
     * Read and apply Structs and then DeleteStore to a y instance.
     *
     * @param {decoding.Decoder} decoder
     * @param {Y.YDoc} doc
     * @param {any} transactionOrigin
     */
    const readUpdate = readSyncStep2

    /**
     * @param {decoding.Decoder} decoder A message received from another client
     * @param {encoding.Encoder} encoder The reply message. Will not be sent if empty.
     * @param {Y.YDoc} doc
     * @param {any} transactionOrigin
     */
    const readSyncMessage = (decoder, encoder, doc) => {
        const messageType = decoding.readVarUint(decoder)
        switch (messageType) {
            case messageYjsSyncStep1:
                readSyncStep1(decoder, encoder, doc)
                break
            case messageYjsSyncStep2:
                readSyncStep2(decoder, doc)
                break
            case messageYjsUpdate:
                readUpdate(decoder, doc)
                break
            default:
                throw new Error('Unknown message type')
        }
        return messageType
    }

    return {
        messageYjsSyncStep1: messageYjsSyncStep1,
        messageYjsSyncStep2: messageYjsSyncStep2,
        messageYjsUpdate: messageYjsUpdate,
        writeSyncStep1: writeSyncStep1,
        writeSyncStep2: writeSyncStep2,
        readSyncStep1: readSyncStep1,
        readSyncStep2: readSyncStep2,
        writeUpdate: writeUpdate,
        readUpdate: readUpdate,
        readSyncMessage: readSyncMessage,
    };
})();

if (typeof window !== 'undefined') {
    // @ts-ignore
    window.Y = Y // eslint-disable-line
}

/**
 * @param {TestYInstance} y // publish message created by `y` to all other online clients
 * @param {Uint8Array} m
 */
const broadcastMessage = (y, m) => {
    if (y.tc.onlineConns.has(y)) {
        y.tc.onlineConns.forEach(remoteYInstance => {
            if (remoteYInstance !== y) {
                remoteYInstance._receive(m, y)
            }
        })
    }
}

export let useV2 = false

export const encV1 = {
    encodeStateAsUpdate: Y.encodeStateAsUpdate,
    mergeUpdates: Y.mergeUpdates,
    applyUpdate: Y.applyUpdate,
    logUpdate: Y.logUpdate,
    updateEventName: 'update',
    diffUpdate: Y.diffUpdate
}

export const encV2 = {
    encodeStateAsUpdate: Y.encodeStateAsUpdateV2,
    mergeUpdates: Y.mergeUpdatesV2,
    applyUpdate: Y.applyUpdateV2,
    logUpdate: Y.logUpdateV2,
    updateEventName: 'updateV2',
    diffUpdate: Y.diffUpdateV2
}

export let enc = encV1

const useV1Encoding = () => {
    useV2 = false
    enc = encV1
}

const useV2Encoding = () => {
    console.error('sync protocol doesnt support v2 protocol yet, fallback to v1 encoding') // @Todo
    useV2 = false
    enc = encV1
}

export class TestYInstance extends Y.YDoc {
    /**
     * @param {TestConnector} testConnector
     * @param {number} clientID
     */
    constructor (testConnector, clientID) {
        super()
        this.userID = clientID // overwriting clientID
        /**
         * @type {TestConnector}
         */
        this.tc = testConnector
        /**
         * @type {Map<TestYInstance, Array<Uint8Array>>}
         */
        this.receiving = new Map()
        testConnector.allConns.add(this)
        /**
         * The list of received updates.
         * We are going to merge them later using Y.mergeUpdates and check if the resulting document is correct.
         * @type {Array<Uint8Array>}
         */
        this.updates = []
        // set up observe on local model
        this.on(enc.updateEventName, /** @param {Uint8Array} update @param {any} origin */ (update, origin) => {
            if (origin !== testConnector) {
                const encoder = encoding.createEncoder()
                syncProtocol.writeUpdate(encoder, update)
                broadcastMessage(this, encoding.toUint8Array(encoder))
                this.updates.push(update)
            }
        })
        this.connect()
    }

    /**
     * Disconnect from TestConnector.
     */
    disconnect () {
        this.receiving = new Map()
        this.tc.onlineConns.delete(this)
    }

    /**
     * Append yourself to the list of known Y instances in testconnector.
     * Also initiate sync with all clients.
     */
    connect () {
        if (!this.tc.onlineConns.has(this)) {
            this.tc.onlineConns.add(this)
            const encoder = encoding.createEncoder()
            syncProtocol.writeSyncStep1(encoder, this)
            // publish SyncStep1
            broadcastMessage(this, encoding.toUint8Array(encoder))
            this.tc.onlineConns.forEach(remoteYInstance => {
                if (remoteYInstance !== this) {
                    // remote instance sends instance to this instance
                    const encoder = encoding.createEncoder()
                    syncProtocol.writeSyncStep1(encoder, remoteYInstance)
                    this._receive(encoding.toUint8Array(encoder), remoteYInstance)
                }
            })
        }
    }

    /**
     * Receive a message from another client. This message is only appended to the list of receiving messages.
     * TestConnector decides when this client actually reads this message.
     *
     * @param {Uint8Array} message
     * @param {TestYInstance} remoteClient
     */
    _receive (message, remoteClient) {
        let messages = this.receiving.get(remoteClient)
        if (messages === undefined) {
            messages = []
            this.receiving.set(remoteClient, messages)
        }
        messages.push(message)
    }
}

/**
 * Keeps track of TestYInstances.
 *
 * The TestYInstances add/remove themselves from the list of connections maiained in this object.
 * I think it makes sense. Deal with it.
 */
export class TestConnector {
    /**
     * @param {prng.PRNG} gen
     */
    constructor (gen) {
        /**
         * @type {Set<TestYInstance>}
         */
        this.allConns = new Set()
        /**
         * @type {Set<TestYInstance>}
         */
        this.onlineConns = new Set()
        /**
         * @type {prng.PRNG}
         */
        this.prng = gen
    }

    /**
     * Create a new Y instance and add it to the list of connections
     * @param {number} clientID
     */
    createY (clientID) {
        return new TestYInstance(this, clientID)
    }

    /**
     * Choose random connection and flush a random message from a random sender.
     *
     * If this function was unable to flush a message, because there are no more messages to flush, it returns false. true otherwise.
     * @return {boolean}
     */
    flushRandomMessage () {
        const gen = this.prng
        const conns = Array.from(this.onlineConns).filter(conn => conn.receiving.size > 0)
        if (conns.length > 0) {
            const receiver = prng.oneOf(gen, conns)
            const [sender, messages] = prng.oneOf(gen, Array.from(receiver.receiving))
            const m = messages.shift()
            if (messages.length === 0) {
                receiver.receiving.delete(sender)
            }
            if (m === undefined) {
                return this.flushRandomMessage()
            }
            const encoder = encoding.createEncoder()
            // console.log('receive (' + sender.userID + '->' + receiver.userID + '):\n', syncProtocol.stringifySyncMessage(decoding.createDecoder(m), receiver))
            // do not publish data created when this function is executed (could be ss2 or update message)
            syncProtocol.readSyncMessage(decoding.createDecoder(m), encoder, receiver, receiver.tc)
            if (encoding.length(encoder) > 0) {
                // send reply message
                sender._receive(encoding.toUint8Array(encoder), receiver)
            }
            {
                // If update message, add the received message to the list of received messages
                const decoder = decoding.createDecoder(m)
                const messageType = decoding.readVarUint(decoder)
                switch (messageType) {
                    case syncProtocol.messageYjsUpdate:
                    case syncProtocol.messageYjsSyncStep2:
                        receiver.updates.push(decoding.readVarUint8Array(decoder))
                        break
                }
            }
            return true
        }
        return false
    }

    /**
     * @return {boolean} True iff this function actually flushed something
     */
    flushAllMessages () {
        let didSomething = false
        while (this.flushRandomMessage()) {
            didSomething = true
        }
        return didSomething
    }

    reconnectAll () {
        this.allConns.forEach(conn => conn.connect())
    }

    disconnectAll () {
        this.allConns.forEach(conn => conn.disconnect())
    }

    syncAll () {
        this.reconnectAll()
        this.flushAllMessages()
    }

    /**
     * @return {boolean} Whether it was possible to disconnect a randon connection.
     */
    disconnectRandom () {
        if (this.onlineConns.size === 0) {
            return false
        }
        prng.oneOf(this.prng, Array.from(this.onlineConns)).disconnect()
        return true
    }

    /**
     * @return {boolean} Whether it was possible to reconnect a random connection.
     */
    reconnectRandom () {
        /**
         * @type {Array<TestYInstance>}
         */
        const reconnectable = []
        this.allConns.forEach(conn => {
            if (!this.onlineConns.has(conn)) {
                reconnectable.push(conn)
            }
        })
        if (reconnectable.length === 0) {
            return false
        }
        prng.oneOf(this.prng, reconnectable).connect()
        return true
    }
}

/**
 * @template T
 * @param {t.TestCase} tc
 * @param {{users?:number}} conf
 * @param {InitTestObjectCallback<T>} [initTestObject]
 * @return {{testObjects:Array<any>,testConnector:TestConnector,users:Array<TestYInstance>,array0:Y.YArray<any>,array1:Y.YArray<any>,array2:Y.YArray<any>,map0:Y.YMap<any>,map1:Y.YMap<any>,map2:Y.YMap<any>,map3:Y.YMap<any>,text0:Y.YText,text1:Y.YText,text2:Y.YText,xml0:Y.YXmlElement,xml1:Y.YXmlElement,xml2:Y.YXmlElement}}
 */
export const init = (tc, { users = 5 } = {}, initTestObject) => {
    /**
     * @type {Object<string,any>}
     */
    const result = {
        users: []
    }
    const gen = tc.prng
    // choose an encoding approach at random
    if (prng.bool(gen)) {
        useV2Encoding()
    } else {
        useV1Encoding()
    }

    const testConnector = new TestConnector(gen)
    result.testConnector = testConnector
    for (let i = 0; i < users; i++) {
        const y = testConnector.createY(i)
        y.clientID = i
        result.users.push(y)
        result['array' + i] = y.getArray('array')
        result['map' + i] = y.getMap('map')
        result['xml' + i] = y.get('xml', Y.YXmlElement)
        result['text' + i] = y.getText('text')
    }
    testConnector.syncAll()
    result.testObjects = result.users.map(initTestObject || (() => null))
    useV1Encoding()
    return /** @type {any} */ (result)
}

/**
 * 1. reconnect and flush all
 * 2. user 0 gc
 * 3. get type content
 * 4. disconnect & reconnect all (so gc is propagated)
 * 5. compare os, ds, ss
 *
 * @param {Array<TestYInstance>} users
 */
export const compare = users => {
    users.forEach(u => u.connect())
    while (users[0].tc.flushAllMessages()) {}
    // For each document, merge all received document updates with Y.mergeUpdates and create a new document which will be added to the list of "users"
    // This ensures that mergeUpdates works correctly
    const mergedDocs = users.map(user => {
        const ydoc = new Y.YDoc()
        enc.applyUpdate(ydoc, enc.mergeUpdates(user.updates))
        return ydoc
    })
    users.push(.../** @type {any} */(mergedDocs))
    const userArrayValues = users.map(u => u.getArray('array').toJSON())
    const userMapValues = users.map(u => u.getMap('map').toJSON())
    const userXmlValues = users.map(u => u.getXmlElement('xml').toString())
    const userTextValues = users.map(u => u.getText('text').toDelta())
    for (const u of users) {
        t.assert(u.store.pendingDs === null)
        t.assert(u.store.pendingStructs === null)
    }
    // Test Array iterator
    t.compare(users[0].getArray('array').toArray(), Array.from(users[0].getArray('array')))
    // Test Map iterator
    const ymapkeys = Array.from(users[0].getMap('map').keys())
    t.assert(ymapkeys.length === Object.keys(userMapValues[0]).length)
    ymapkeys.forEach(key => t.assert(object.hasProperty(userMapValues[0], key)))
    /**
     * @type {Object<string,any>}
     */
    const mapRes = {}
    for (const [k, v] of users[0].getMap('map')) {
        mapRes[k] = v instanceof Y.AbstractType ? v.toJSON() : v
    }
    t.compare(userMapValues[0], mapRes)
    // Compare all users
    for (let i = 0; i < users.length - 1; i++) {
        t.compare(userArrayValues[i].length, users[i].getArray('array').length)
        t.compare(userArrayValues[i], userArrayValues[i + 1])
        t.compare(userMapValues[i], userMapValues[i + 1])
        t.compare(userXmlValues[i], userXmlValues[i + 1])
        t.compare(userTextValues[i].map(/** @param {any} a */ a => typeof a.insert === 'string' ? a.insert : ' ').join('').length, users[i].getText('text').length)
        t.compare(userTextValues[i], userTextValues[i + 1])
        t.compare(Y.getStateVector(users[i].store), Y.getStateVector(users[i + 1].store))
        compareDS(Y.createDeleteSetFromStructStore(users[i].store), Y.createDeleteSetFromStructStore(users[i + 1].store))
        compareStructStores(users[i].store, users[i + 1].store)
    }
    users.map(u => u.destroy())
}

/**
 * @param {Y.Item?} a
 * @param {Y.Item?} b
 * @return {boolean}
 */
export const compareItemIDs = (a, b) => a === b || (a !== null && b != null && Y.compareIDs(a.id, b.id))

/**
 * @param {Y.StructStore} ss1
 * @param {Y.StructStore} ss2
 */
export const compareStructStores = (ss1, ss2) => {
    t.assert(ss1.clients.size === ss2.clients.size)
    for (const [client, structs1] of ss1.clients) {
        const structs2 = /** @type {Array<Y.AbstractStruct>} */ (ss2.clients.get(client))
        t.assert(structs2 !== undefined && structs1.length === structs2.length)
        for (let i = 0; i < structs1.length; i++) {
            const s1 = structs1[i]
            const s2 = structs2[i]
            // checks for abstract struct
            if (
                s1.constructor !== s2.constructor ||
                !Y.compareIDs(s1.id, s2.id) ||
                s1.deleted !== s2.deleted ||
                // @ts-ignore
                s1.length !== s2.length
            ) {
                t.fail('Structs dont match')
            }
            if (s1 instanceof Y.Item) {
                if (
                    !(s2 instanceof Y.Item) ||
                    !((s1.left === null && s2.left === null) || (s1.left !== null && s2.left !== null && Y.compareIDs(s1.left.lastId, s2.left.lastId))) ||
                    !compareItemIDs(s1.right, s2.right) ||
                    !Y.compareIDs(s1.origin, s2.origin) ||
                    !Y.compareIDs(s1.rightOrigin, s2.rightOrigin) ||
                    s1.parentSub !== s2.parentSub
                ) {
                    return t.fail('Items dont match')
                }
                // make sure that items are connected correctly
                t.assert(s1.left === null || s1.left.right === s1)
                t.assert(s1.right === null || s1.right.left === s1)
                t.assert(s2.left === null || s2.left.right === s2)
                t.assert(s2.right === null || s2.right.left === s2)
            }
        }
    }
}

/**
 * @param {Y.DeleteSet} ds1
 * @param {Y.DeleteSet} ds2
 */
export const compareDS = (ds1, ds2) => {
    t.assert(ds1.clients.size === ds2.clients.size)
    ds1.clients.forEach((deleteItems1, client) => {
        const deleteItems2 = /** @type {Array<Y.DeleteItem>} */ (ds2.clients.get(client))
        t.assert(deleteItems2 !== undefined && deleteItems1.length === deleteItems2.length)
        for (let i = 0; i < deleteItems1.length; i++) {
            const di1 = deleteItems1[i]
            const di2 = deleteItems2[i]
            if (di1.clock !== di2.clock || di1.len !== di2.len) {
                t.fail('DeleteSets dont match')
            }
        }
    })
}

/**
 * @template T
 * @callback InitTestObjectCallback
 * @param {TestYInstance} y
 * @return {T}
 */

/**
 * @template T
 * @param {t.TestCase} tc
 * @param {Array<function(Y.YDoc,prng.PRNG,T):void>} mods
 * @param {number} iterations
 * @param {InitTestObjectCallback<T>} [initTestObject]
 */
export const applyRandomTests = (tc, mods, iterations, initTestObject) => {
    const gen = tc.prng
    const result = init(tc, { users: 5 }, initTestObject)
    const { testConnector, users } = result
    for (let i = 0; i < iterations; i++) {
        if (prng.int32(gen, 0, 100) <= 2) {
            // 2% chance to disconnect/reconnect a random user
            if (prng.bool(gen)) {
                testConnector.disconnectRandom()
            } else {
                testConnector.reconnectRandom()
            }
        } else if (prng.int32(gen, 0, 100) <= 1) {
            // 1% chance to flush all
            testConnector.flushAllMessages()
        } else if (prng.int32(gen, 0, 100) <= 50) {
            // 50% chance to flush a random message
            testConnector.flushRandomMessage()
        }
        const user = prng.int32(gen, 0, users.length - 1)
        const test = prng.oneOf(gen, mods)
        test(users[user], gen, result.testObjects[user])
    }
    compare(users)
    return result
}
