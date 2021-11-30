import { YjsFactory } from './factory.js'
import { runBenchmarks, writeBenchmarkResultsToFile } from '../../js-lib/index.js'

const logMemOnly = process.argv[2] === 'mem-only'

;(async () => {
  await runBenchmarks(new YjsFactory(), testName => false && !testName.startsWith('[B4'))
  writeBenchmarkResultsToFile('../results.json', testId => logMemOnly && testId.search('(memUsed)') < 0)
})()
