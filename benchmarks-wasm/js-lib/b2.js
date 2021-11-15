import { setBenchmarkResult, gen, N, benchmarkTime, runBenchmark, logMemoryUsed, getMemUsed } from './utils.js'
import * as prng from 'lib0/prng'
import * as math from 'lib0/math'
import { createMutex } from 'lib0/mutex'
import * as t from 'lib0/testing'
import { CrdtFactory, AbstractCrdt } from './index.js' // eslint-disable-line

const initText = prng.word(gen, 100, 100)

/**
 * @param {CrdtFactory} crdtFactory
 * @param {function(string):boolean} filter
 */
export const runBenchmarkB2 = async (crdtFactory, filter) => {
  /**
   * @param {string} id
   * @param {function(AbstractCrdt):void} changeDoc1
   * @param {function(AbstractCrdt):void} changeDoc2
   * @param {function(AbstractCrdt, AbstractCrdt):void} check
   */
  const benchmarkTemplate = (id, changeDoc1, changeDoc2, check) => {
    let encodedState = null
    {
      let updatesSize = 0
      const mux = createMutex()
      const doc1 = crdtFactory.create(update => mux(() => { updatesSize += update.length; doc2.applyUpdate(update) }))
      const doc2 = crdtFactory.create(update => mux(() => { updatesSize += update.length; doc1.applyUpdate(update) }))
      doc1.insertText(0, initText)
      benchmarkTime(crdtFactory.getName(), `${id} (time)`, () => {
        doc1.transact(() => {
          changeDoc1(doc1)
        })
        doc2.transact(() => {
          changeDoc2(doc2)
        })
      })
      check(doc1, doc2)
      const avgUpdateSize = math.round(updatesSize / 2)
      setBenchmarkResult(crdtFactory.getName(), `${id} (updateSize)`, `${avgUpdateSize} bytes`)
      benchmarkTime(crdtFactory.getName(), `${id} (encodeTime)`, () => {
        encodedState = doc1.getEncodedState()
      })
      // @ts-ignore
      const documentSize = encodedState.length
      setBenchmarkResult(crdtFactory.getName(), `${id} (docSize)`, `${documentSize} bytes`)
    }
    benchmarkTime(crdtFactory.getName(), `${id} (parseTime)`, () => {
      const startHeapUsed = getMemUsed()
      const doc = crdtFactory.create()
      doc.applyUpdate(encodedState)
      logMemoryUsed(crdtFactory.getName(), id, startHeapUsed)
    })
  }

  await runBenchmark('[B2.1] Concurrently insert string of length N at index 0', filter, benchmarkName => {
    const string1 = prng.word(gen, N, N)
    const string2 = prng.word(gen, N, N)
    benchmarkTemplate(
      benchmarkName,
      doc1 => { doc1.insertText(0, string1) },
      doc2 => { doc2.insertText(0, string2) },
      (doc1, doc2) => {
        t.assert(doc1.getText() === doc2.getText())
        t.assert(doc1.getText().length === N * 2 + 100)
      }
    )
  })

  await runBenchmark('[B2.2] Concurrently insert N characters at random positions', filter, benchmarkName => {
    const genInput = () => {
      let str = initText
      const input = []
      for (let i = 0; i < N; i++) {
        const index = prng.uint32(gen, 0, str.length)
        const insert = prng.word(gen, 1, 1)
        str = str.slice(0, index) + insert + str.slice(index)
        input.push({ index, insert })
      }
      return input
    }
    const input1 = genInput()
    const input2 = genInput()
    benchmarkTemplate(
      benchmarkName,
      doc1 => {
        input1.forEach(({ index, insert }) => { doc1.insertText(index, insert) })
      },
      doc2 => {
        input2.forEach(({ index, insert }) => { doc2.insertText(index, insert) })
      },
      (doc1, doc2) => {
        t.assert(doc1.getText() === doc2.getText())
        t.assert(doc1.getText().length === N * 2 + 100)
      }
    )
  })

  await runBenchmark('[B2.3] Concurrently insert N words at random positions', filter, benchmarkName => {
    const genInput = () => {
      let str = initText
      const input = []
      for (let i = 0; i < N; i++) {
        const index = prng.uint32(gen, 0, str.length)
        const insert = prng.word(gen, 3, 9)
        str = str.slice(0, index) + insert + str.slice(index)
        input.push({ index, insert })
      }
      return input
    }
    const input1 = genInput()
    const input2 = genInput()
    benchmarkTemplate(
      benchmarkName,
      doc1 => {
        input1.forEach(({ index, insert }) => { doc1.insertText(index, insert) })
      },
      doc2 => {
        input2.forEach(({ index, insert }) => { doc2.insertText(index, insert) })
      },
      (doc1, doc2) => {
        t.assert(doc1.getText() === doc2.getText())
      }
    )
  })

  await runBenchmark('[B2.4] Concurrently insert & delete', filter, benchmarkName => {
    const genInput = () => {
      let str = initText
      const input = []
      for (let i = 0; i < N; i++) {
        const index = prng.uint32(gen, 0, str.length)
        const insert = prng.word(gen, 3, 9)
        str = str.slice(0, index) + insert + str.slice(index)
        input.push({ index, insert })
        if (str.length === index || prng.bool(gen)) {
          const insert = prng.word(gen, 2, 10)
          str = str.slice(0, index) + insert + str.slice(index)
          input.push({ index, insert })
        } else {
          const deleteCount = prng.uint32(gen, 1, math.min(9, str.length - index))
          str = str.slice(0, index) + str.slice(index + deleteCount)
          input.push({ index, deleteCount })
        }
      }
      return input
    }
    const input1 = genInput()
    const input2 = genInput()
    benchmarkTemplate(
      benchmarkName,
      doc1 => {
        input1.forEach(({ index, insert, deleteCount }) => {
          if (insert !== undefined) {
            doc1.insertText(index, insert)
          } else {
            doc1.deleteText(index, deleteCount || 0)
          }
        })
      },
      doc2 => {
        input2.forEach(({ index, insert, deleteCount }) => {
          if (insert !== undefined) {
            doc2.insertText(index, insert)
          } else {
            doc2.deleteText(index, deleteCount || 0)
          }
        })
      },
      (doc1, doc2) => {
        t.assert(doc1.getText() === doc2.getText())
      }
    )
  })
}
