import { setBenchmarkResult, benchmarkTime, logMemoryUsed, getMemUsed, tryGc, runBenchmark } from './utils.js'
import * as math from 'lib0/math'
import * as t from 'lib0/testing'
import { CrdtFactory, AbstractCrdt } from './index.js' // eslint-disable-line
// @ts-ignore
import { edits, finalText } from './b4-editing-trace.js'

/**
 * @param {CrdtFactory} crdtFactory
 * @param {function(string):boolean} filter
 */
export const runBenchmarkB4 = async (crdtFactory, filter) => {
  /**
   * @template T
   * @param {string} id
   * @param {Array<T>} inputData
   * @param {function(AbstractCrdt, T, number):void} changeFunction
   * @param {function(AbstractCrdt):void} check
   */
  const benchmarkTemplate = (id, inputData, changeFunction, check) => {
    let encodedState = /** @type {any} */ (null)
    ;(() => {
      // We scope the creation of doc1 so we can gc it before we parse it again.
      const doc1 = crdtFactory.create()
      benchmarkTime(crdtFactory.getName(), `${id} (time)`, () => {
        for (let i = 0; i < inputData.length; i++) {
          changeFunction(doc1, inputData[i], i)
          // we forcefully overwrite updates because we want to reduce potentially significant memory overhead
        }
      })
      check(doc1)
      benchmarkTime(crdtFactory.getName(), `${id} (encodeTime)`, () => {
        encodedState = doc1.getEncodedState()
      })
    })()
    const documentSize = encodedState.byteLength
    setBenchmarkResult(crdtFactory.getName(), `${id} (docSize)`, `${documentSize} bytes`)
    ;(() => {
      const startHeapUsed = getMemUsed()
      // @ts-ignore we only store doc so it is not garbage collected
      let doc = null // eslint-disable-line
      benchmarkTime(crdtFactory.getName(), `${id} (parseTime)`, () => {
        doc = crdtFactory.create()
        doc.applyUpdate(encodedState)
      })
      logMemoryUsed(crdtFactory.getName(), id, startHeapUsed)
    })()
  }

  await runBenchmark('[B4] Apply real-world editing dataset', filter, async benchmarkName => {
    await benchmarkTemplate(
      benchmarkName,
      edits,
      (doc, edit) => {
        doc.deleteText(edit[0], edit[1])
        if (edit[2]) {
          doc.insertText(edit[0], edit[2])
        }
      },
      doc1 => {
        t.compareStrings(doc1.getText(), finalText)
      }
    )
  })

  await runBenchmark('[B4x100] Apply real-world editing dataset 100 times', filter, benchmarkName => {
    const multiplicator = 100
    let encodedState = /** @type {any} */ (null)

    ;(() => {
      const doc = crdtFactory.create()

      benchmarkTime(crdtFactory.getName(), `${benchmarkName} (time)`, () => {
        for (let iterations = 0; iterations < multiplicator; iterations++) {
          if (iterations > 0 && iterations % 5 === 0) {
            console.log(`Finished ${iterations}%`)
          }
          for (let i = 0; i < edits.length; i++) {
            const edit = edits[i]
            if (edit[1] > 0) {
              doc.deleteText(edit[0], edit[1])
            }
            if (edit[2]) {
              doc.insertText(edit[0], edit[2])
            }
          }
        }
      })
      /**
       * @type {any}
       */
      benchmarkTime(crdtFactory.getName(), `${benchmarkName} (encodeTime)`, () => {
        encodedState = doc.getEncodedState()
      })
    })()

    ;(() => {
      const documentSize = encodedState.byteLength
      setBenchmarkResult(crdtFactory.getName(), `${benchmarkName} (docSize)`, `${documentSize} bytes`)
      tryGc()
    })()

    ;(() => {
      const startHeapUsed = getMemUsed()
      // @ts-ignore we only store doc so it is not garbage collected
      let doc = null // eslint-disable-line
      benchmarkTime(crdtFactory.getName(), `${benchmarkName} (parseTime)`, () => {
        doc = crdtFactory.create()
        doc.applyUpdate(encodedState)
      })
      logMemoryUsed(crdtFactory.getName(), benchmarkName, startHeapUsed)
    })()
  })
}
