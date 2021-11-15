
import { AbstractCrdt, CrdtFactory } from '../../js-lib/index.js' // eslint-disable-line
import * as Y from 'ywasm'

export const name = 'ywasm'

/**
 * @implements {CrdtFactory}
 */
export class YjsFactory {
  /**
   * @param {function(Uint8Array):void} [updateHandler]
   */
  create (updateHandler) {
    return new YwasmCRDT(updateHandler)
  }

  getName () {
    return name
  }
}

/**
 * @implements {AbstractCrdt}
 */
export class YwasmCRDT {

  /**
   * @param {function(Uint8Array):void} [updateHandler]
   */
  constructor (updateHandler) {
    this.ydoc = new Y.YDoc()
    this.transaction = null;
    this.updateHandler = updateHandler;
    this.yarray = this.ydoc.getArray('array')
    this.ymap = this.ydoc.getMap('map')
    this.ytext = this.ydoc.getText('text')
  }

  /**
   * @return {Uint8Array|string}
   */
  getEncodedState () {
    return Y.encodeStateAsUpdate(this.ydoc)
  }

  /**
   * @param {Uint8Array} update
   */
  applyUpdate (update) {
    Y.applyUpdate(this.ydoc, update)
  }

  /**
   * Insert several items into the internal shared array implementation.
   *
   * @param {number} index
   * @param {Array<any>} elems
   */
  insertArray (index, elems) {
    this.transact(tr => this.yarray.insert(tr, index, elems))
  }

  /**
   * Delete several items into the internal shared array implementation.
   *
   * @param {number} index
   * @param {number} len
   */
  deleteArray (index, len) {
    this.transact(tr => this.yarray.delete(tr, index, len))
  }

  /**
   * @return {Array<any>}
   */
  getArray () {
    return this.transact(tr => this.yarray.toJson(tr))
  }

  /**
   * Insert text into the internal shared text implementation.
   *
   * @param {number} index
   * @param {string} text
   */
  insertText (index, text) {
    this.transact(tr => this.ytext.insert(tr, index, text))
  }

  /**
   * Delete text from the internal shared text implementation.
   *
   * @param {number} index
   * @param {number} len
   */
  deleteText (index, len) {
    this.transact(tr => this.ytext.delete(tr, index, len))
  }

  /**
   * @return {string}
   */
  getText () {
    return this.transact(tr => this.ytext.toString(tr))
  }

  /**
   * @template T
   * @param {function(any):T} f
   * @return {T}
   */
  transact (f) {
    let result;
    if (this.transaction) {
      result = f(this.transaction);
    } else {
      this.transaction = this.ydoc.beginTransaction();
      try {

        result = f(this.transaction)

        // call update handler
        if (this.updateHandler) {
          this.updateHandler(this.transaction.encodeUpdate());
        }
      } finally {
        this.transaction.free()
        this.transaction = null;
      }
    }

    return result
  }

  /**
   * @param {string} key
   * @param {any} val
   */
  setMap (key, val) {
    this.transact(tr => this.ymap.set(tr, key, val))
  }

  /**
   * @return {Map<string,any> | Object<string, any>}
   */
  getMap () {
    return this.transact(tr => this.ymap.toJson(tr))
  }
}
