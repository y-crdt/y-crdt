import { setBenchmarkResult, gen, N, benchmarkTime, logMemoryUsed, getMemUsed, runBenchmark } from './utils.js'
import * as prng from 'lib0/prng'
import * as math from 'lib0/math'
import * as t from 'lib0/testing'
import { CrdtFactory, AbstractCrdt } from './index.js' // eslint-disable-line

/**
 * @param {CrdtFactory} crdtFactory
 * @param {function(string):boolean} filter
 */
export const runBenchmarksB1 = async (crdtFactory, filter) => {
  /**
   * Helper function to run a B1 benchmarks.
   *
   * @template T
   * @param {string} id name of the benchmark e.g. "[B1.1] Description"
   * @param {Array<T>} inputData
   * @param {function(AbstractCrdt, T, number):void} changeFunction Is called on every element in inputData
   * @param {function(AbstractCrdt, AbstractCrdt):void} check Check if the benchmark result is correct (all clients end up with the expected result)
   */
  const benchmarkTemplate = (id, inputData, changeFunction, check) => {
    let encodedState = null
    {
      const doc1Updates = []
      const doc1 = crdtFactory.create(update => { doc1Updates.push(update) })
      const doc2 = crdtFactory.create()
      benchmarkTime(crdtFactory.getName(), `${id} (time)`, () => {
        for (let i = 0; i < inputData.length; i++) {
          changeFunction(doc1, inputData[i], i)
        }
      })
      doc1Updates.forEach(update => {
        doc2.applyUpdate(update)
      })
      check(doc1, doc2)
      const updateSize = doc1Updates.reduce((a, b) => a + b.length, 0)
      setBenchmarkResult(crdtFactory.getName(), `${id} (avgUpdateSize)`, `${math.round(updateSize / inputData.length)} bytes`)
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

  await runBenchmark('[B1.1] Append N characters', filter, benchmarkName => {
    const string = prng.word(gen, N, N)
    benchmarkTemplate(
      benchmarkName,
      string.split(''),
      (doc, s, i) => { doc.insertText(i, s) },
      (doc1, doc2) => {
        t.assert(doc1.getText() === doc2.getText())
        t.assert(doc1.getText() === string)
      }
    )
  })

  await runBenchmark('[B1.2] Insert string of length N', filter, benchmarkName => {
    const string = prng.word(gen, N, N)
    // B1.1: Insert text from left to right
    benchmarkTemplate(
      benchmarkName,
      [string],
      (doc, s, i) => { doc.insertText(i, s) },
      (doc1, doc2) => {
        t.assert(doc1.getText() === doc2.getText())
        t.assert(doc1.getText() === string)
      }
    )
  })

  await runBenchmark('[B1.3] Prepend N characters', filter, benchmarkName => {
    const string = prng.word(gen, N, N)
    const reversedString = string.split('').reverse().join('')
    benchmarkTemplate(
      benchmarkName,
      reversedString.split(''),
      (doc, s, i) => { doc.insertText(0, s) },
      (doc1, doc2) => {
        t.assert(doc1.getText() === doc2.getText())
        t.assert(doc1.getText() === string)
      }
    )
  })

  await runBenchmark('[B1.4] Insert N characters at random positions', filter, benchmarkName => {
    // calculate random input
    let string = ''
    const input = []
    for (let i = 0; i < N; i++) {
      const index = prng.int32(gen, 0, string.length)
      const insert = prng.word(gen, 1, 1)
      string = string.slice(0, index) + insert + string.slice(index)
      input.push({ index, insert })
    }
    benchmarkTemplate(
      benchmarkName,
      input,
      (doc, op, i) => { doc.insertText(op.index, op.insert) },
      (doc1, doc2) => {
        t.assert(doc1.getText() === doc2.getText())
        t.assert(doc1.getText() === string)
      }
    )
  })

  await runBenchmark('[B1.5] Insert N words at random positions', filter, benchmarkName => {
    // This test simulates a real-word editing scenario.
    // Users often write a word, and then switch to a different next position.
    // calculate random input
    let string = ''
    const input = []
    for (let i = 0; i < N; i++) {
      const index = prng.int32(gen, 0, string.length)
      const insert = prng.word(gen, 2, 10)
      string = string.slice(0, index) + insert + string.slice(index)
      input.push({ index, insert })
    }
    benchmarkTemplate(
      benchmarkName,
      input,
      (doc, op, i) => { doc.insertText(op.index, op.insert) },
      (doc1, doc2) => {
        t.assert(doc1.getText() === doc2.getText())
        t.assert(doc1.getText() === string)
      }
    )
  })

  await runBenchmark('[B1.6] Insert string, then delete it', filter, benchmarkName => {
    const string = prng.word(gen, N, N)
    // B1.1: Insert text from left to right
    benchmarkTemplate(
      benchmarkName,
      [string],
      (doc, s, i) => {
        doc.insertText(i, s)
        doc.deleteText(i, s.length)
      },
      (doc1, doc2) => {
        t.assert(doc1.getText() === doc2.getText())
        t.assert(doc1.getText() === '')
      }
    )
  })

  await runBenchmark('[B1.7] Insert/Delete strings at random positions', filter, benchmarkName => {
    // calculate random input
    let string = ''
    const input = []
    for (let i = 0; i < N; i++) {
      const index = prng.uint32(gen, 0, string.length)
      if (string.length === index || prng.bool(gen)) {
        const insert = prng.word(gen, 2, 10)
        string = string.slice(0, index) + insert + string.slice(index)
        input.push({ index, insert })
      } else {
        const deleteCount = prng.uint32(gen, 1, math.min(9, string.length - index))
        string = string.slice(0, index) + string.slice(index + deleteCount)
        input.push({ index, deleteCount })
      }
    }
    benchmarkTemplate(
      benchmarkName,
      input,
      (doc, op, i) => {
        if (op.insert !== undefined) {
          doc.insertText(op.index, op.insert)
        } else {
          doc.deleteText(op.index, op.deleteCount)
        }
      },
      (doc1, doc2) => {
        t.assert(doc1.getText() === doc2.getText())
        t.assert(doc1.getText() === string)
      }
    )
  })

  // benchmarks with numbers begin here

  await runBenchmark('[B1.8] Append N numbers', filter, benchmarkName => {
    const numbers = Array.from({ length: N }).map(() => prng.uint32(gen, 0, 0x7fffffff))
    benchmarkTemplate(
      benchmarkName,
      numbers,
      (doc, s, i) => { doc.insertArray(i, [s]) },
      (doc1, doc2) => {
        t.compare(doc1.getArray(), doc2.getArray())
        t.compare(doc1.getArray(), numbers)
      }
    )
  })

  await runBenchmark('[B1.9] Insert Array of N numbers', filter, benchmarkName => {
    const numbers = Array.from({ length: N }).map(() => prng.uint32(gen, 0, 0x7fffffff))
    benchmarkTemplate(
      benchmarkName,
      [numbers],
      (doc, s, i) => { doc.insertArray(i, s) },
      (doc1, doc2) => {
        t.compare(doc1.getArray(), doc2.getArray())
        t.compare(doc1.getArray(), numbers)
      }
    )
  })

  await runBenchmark('[B1.10] Prepend N numbers', filter, benchmarkName => {
    const numbers = Array.from({ length: N }).map(() => prng.uint32(gen, 0, 0x7fffffff))
    const numbersReversed = numbers.slice().reverse()

    benchmarkTemplate(
      benchmarkName,
      numbers,
      (doc, s, i) => { doc.insertArray(0, [s]) },
      (doc1, doc2) => {
        t.compare(doc1.getArray(), doc2.getArray())
        t.compare(doc1.getArray(), numbersReversed)
      }
    )
  })

  await runBenchmark('[B1.11] Insert N numbers at random positions', filter, benchmarkName => {
    // calculate random input
    const numbers = []
    const input = []
    for (let i = 0; i < N; i++) {
      const index = prng.uint32(gen, 0, numbers.length)
      const insert = prng.uint32(gen, 0, 0x7fffffff)
      numbers.splice(index, 0, insert)
      input.push({ index, insert })
    }

    benchmarkTemplate(
      benchmarkName,
      input,
      (doc, op, i) => { doc.insertArray(op.index, [op.insert]) },
      (doc1, doc2) => {
        t.compare(doc1.getArray(), doc2.getArray())
        t.compare(doc1.getArray(), numbers)
      }
    )
  })
}
