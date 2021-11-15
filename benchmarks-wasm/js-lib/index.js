import { runBenchmarksB1 } from '../js-lib/b1.js'
import { runBenchmarkB2 } from '../js-lib/b2.js'
import { runBenchmarkB3 } from '../js-lib/b3.js'
import { runBenchmarkB4 } from '../js-lib/b4.js'
import { CrdtFactory } from './utils.js' // eslint-disable-line

export * from './b1.js'
export * from './b2.js'
export * from './b3.js'
export * from './b4.js'
export * from './b4-editing-trace.js'
export * from './utils.js'

/**
 * @param {CrdtFactory} crdtFactory
 * @param {function(string):boolean} testFilter
 */
export const runBenchmarks = async (crdtFactory, testFilter) => {
  await runBenchmarksB1(crdtFactory, testFilter)
  await runBenchmarkB2(crdtFactory, testFilter)
  await runBenchmarkB3(crdtFactory, testFilter)
  await runBenchmarkB4(crdtFactory, testFilter)
}
