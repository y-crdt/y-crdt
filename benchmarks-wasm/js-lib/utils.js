import * as prng from 'lib0/prng'
import * as metric from 'lib0/metric'
import * as math from 'lib0/math'
import * as error from 'lib0/error'
// @ts-ignore
import { performance as perf } from 'isomorphic.js'

export const N = 6000
export const disableAutomergeBenchmarks = true
export const disablePeersCrdtsBenchmarks = true
export const disableYjsBenchmarks = true
export const disableOTBenchmarks = false

export const benchmarkResults = {}

export const setBenchmarkResult = (libname, benchmarkid, result) => {
  console.info(libname, benchmarkid, result)
  const libResults = benchmarkResults[benchmarkid] || (benchmarkResults[benchmarkid] = {})
  libResults[libname] = result
}

export const writeBenchmarkResultsToFile = async (path, filter) => {
  if (typeof process !== 'undefined') {
    const fs = await import('fs')

    let json = {}
    try {
      json = JSON.parse(fs.readFileSync(path, 'utf8'))
    } catch (e) {}
    if (json[N] == null) {
      json[N] = {}
    }
    for (const key in benchmarkResults) {
      if (!filter(key)) {
        json[N][key] = benchmarkResults[key]
      }
    }
    fs.writeFileSync(path, JSON.stringify(json, null, 2))
  }
}

export const benchmarkTime = (libname, id, f) => {
  const start = perf.now()
  f()
  const time = perf.now() - start
  setBenchmarkResult(libname, id, `${time.toFixed(0)} ms`)
}

/**
 * @param {string} name
 * @param {function(string):boolean} filter
 * @param {function(string):(void|Promise<void>)} f
 */
export const runBenchmark = async (name, filter, f) => {
  if (filter(name)) {
    return
  }
  await f(name)
}

/**
 * A Pseudo Random Number Generator with a constant seed, so that repeating the runs will use the same random values.
 */
export const gen = prng.create(42)

export const cpy = o => JSON.parse(JSON.stringify(o))

export const getMemUsed = () => {
  if (typeof global !== 'undefined' && typeof process !== 'undefined') {
    if (global.gc) {
      global.gc()
    }
    return process.memoryUsage().heapUsed
  }
  return 0
}

export const logMemoryUsed = (libname, id, startHeapUsed) => {
  if (typeof global !== 'undefined' && typeof process !== 'undefined') {
    if (global.gc) {
      global.gc()
    }
    const diff = process.memoryUsage().heapUsed - startHeapUsed
    const p = metric.prefix(diff)
    setBenchmarkResult(libname, `${id} (memUsed)`, `${math.round(math.max(p.n * 10, 0)) / 10} ${p.prefix}B`)
  }
}

export const tryGc = () => {
  if (typeof global !== 'undefined' && typeof process !== 'undefined' && global.gc) {
    global.gc()
  }
}

/**
 * Insert a string into a deltaRga crdt
 *
 * @param {any} doc
 * @param {number} index
 * @param {Array<any>|string} content
 * @return {Array<ArrayBuffer>}
 */
export const deltaInsertHelper = (doc, index, content) => {
  const deltas = []
  for (let i = 0; i < content.length; i++) {
    deltas.push(doc.insertAt(index + i, content[i]))
  }
  return deltas
}

/**
 * Insert a string into a deltaRga crdt
 *
 * @param {any} doc
 * @param {number} index
 * @param {number} length
 * @return {Array<ArrayBuffer>}
 */
export const deltaDeleteHelper = (doc, index, length) => {
  const deltas = []
  for (let i = 0; i < length; i++) {
    deltas.push(doc.removeAt(index))
  }
  return deltas
}

export class CrdtFactory {
  /**
   * @param {function(Uint8Array|string):void} [updateHandler]
   * @return {AbstractCrdt}
   */
  create (updateHandler) {
    error.methodUnimplemented()
  }

  /**
   * @return {string}
   */
  getName () {
    error.methodUnimplemented()
  }
}

/**
 * Implement this Abstract CRDT to test it using the crdt-benchmarks library.
 *
 * We expect that the used CRDT library supports manipulation of text, arrays,
 * and of an key-value store (map). There are different tests for teach of these
 * features. The `insertArray` & `deleteArray` methods must operate on an
 * internal representation of an instanciated shared Array. Similarly, the
 * `setMap` and `deleteMap` methods must operate on an internal representation
 * of an instanciated shared Map. These operations must not be cached. They must
 * add an update message immediately to the updates array
 */
export class AbstractCrdt {
  /**
   * @param {function(Uint8Array|string):void} [updateHandler]
   */
  constructor (updateHandler) {
  }

  /**
   * @return {Uint8Array|string}
   */
  getEncodedState () {
    error.methodUnimplemented()
  }

  /**
   * @param {Uint8Array|string} update
   */
  applyUpdate (update) {
    error.methodUnimplemented()
  }

  /**
   * Insert several items into the internal shared array implementation.
   *
   * @param {number} index
   * @param {Array<any>} elements
   */
  insertArray (index, elements) {
    error.methodUnimplemented()
  }

  /**
   * Delete several items into the internal shared array implementation.
   *
   * @param {number} index
   * @param {number} length
   */
  deleteArray (index, length) {
    error.methodUnimplemented()
  }

  /**
   * @return {Array<any>}
   */
  getArray () {
    error.methodUnimplemented()
  }

  /**
   * Insert text into the internal shared text implementation.
   *
   * @param {number} index
   * @param {string} text
   */
  insertText (index, text) {
    error.methodUnimplemented()
  }

  /**
   * Delete text from the internal shared text implementation.
   *
   * @param {number} index
   * @param {number} len
   */
  deleteText (index, len) {
    error.methodUnimplemented()
  }

  /**
   * @return {string}
   */
  getText () {
    error.methodUnimplemented()
  }

  /**
   * @param {function (AbstractCrdt): void} f
   */
  transact (f) {
    error.methodUnimplemented()
  }

  /**
   * @param {string} key
   * @param {any} val
   */
  setMap (key, val) {
    error.methodUnimplemented()
  }

  /**
   * @return {Map<string,any> | Object<string, any>}
   */
  getMap () {
    error.methodUnimplemented()
  }
}
