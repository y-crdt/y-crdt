import { setBenchmarkResult, benchmarkTime, N, logMemoryUsed, getMemUsed, runBenchmark } from './utils.js'
import * as t from 'lib0/testing'
import * as math from 'lib0/math'
import { createMutex } from 'lib0/mutex'
import { CrdtFactory, AbstractCrdt } from './index.js' // eslint-disable-line

const sqrtN = math.floor(math.sqrt(N)) * 20
console.log('sqrtN =', sqrtN)

/**
 * @param {CrdtFactory} crdtFactory
 * @param {function(string):boolean} filter
 */
export const runBenchmarkB3 = async (crdtFactory, filter) => {
  /**
   * @param {string} id
   * @param {function(AbstractCrdt, number):void} changeDoc
   * @param {function(Array<AbstractCrdt>):void} check
   */
  const benchmarkTemplate = (id, changeDoc, check) => {
    let encodedState = null
    {
      const docs = []
      const updates = []
      const mux = createMutex()
      for (let i = 0; i < sqrtN; i++) {
        // push all created updates to the updates array
        docs.push(crdtFactory.create(update => mux(() => updates.push(update))))
      }
      for (let i = 0; i < docs.length; i++) {
        changeDoc(docs[i], i)
      }
      t.assert(updates.length >= sqrtN)
      // sync client 0 for reference
      mux(() => {
        docs[0].transact(() => {
          for (let i = 0; i < updates.length; i++) {
            docs[0].applyUpdate(updates[i])
          }
        })
      })
      benchmarkTime(crdtFactory.getName(), `${id} (time)`, () => {
        mux(() => {
          docs[1].transact(() => {
            for (let i = 0; i < updates.length; i++) {
              docs[1].applyUpdate(updates[i])
            }
          })
        })
      })
      check(docs.slice(0, 2))
      setBenchmarkResult(crdtFactory.getName(), `${id} (updateSize)`, `${updates.reduce((len, update) => len + update.length, 0)} bytes`)
      benchmarkTime(crdtFactory.getName(), `${id} (encodeTime)`, () => {
        encodedState = docs[0].getEncodedState()
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

  await runBenchmark('[B3.1] 20√N clients concurrently set number in Map', filter, benchmarkName => {
    benchmarkTemplate(
      benchmarkName,
      (doc, i) => doc.setMap('v', i),
      docs => {
        const map = docs[0].getMap()
        docs.forEach(doc => {
          t.compare(map, doc.getMap())
        })
      }
    )
  })

  await runBenchmark('[B3.2] 20√N clients concurrently set Object in Map', filter, benchmarkName => {
    // each client sets a user data object { name: id, address: 'here' }
    benchmarkTemplate(
      benchmarkName,
      (doc, i) => {
        doc.setMap('v', {
          name: i.toString(),
          address: 'here'
        })
      },
      docs => {
        const map = docs[0].getMap()
        docs.forEach(doc => {
          t.compare(map, doc.getMap())
        })
      }
    )
  })

  await runBenchmark('[B3.3] 20√N clients concurrently set String in Map', filter, benchmarkName => {
    benchmarkTemplate(
      benchmarkName,
      (doc, i) => {
        doc.setMap('v', i.toString().repeat(sqrtN))
      },
      docs => {
        const map = docs[0].getMap()
        docs.forEach(doc => {
          t.compare(map, doc.getMap())
        })
      }
    )
  })

  await runBenchmark('[B3.4] 20√N clients concurrently insert text in Array', filter, benchmarkName => {
    benchmarkTemplate(
      benchmarkName,
      (doc, i) => {
        doc.insertArray(0, [i.toString()])
      },
      docs => {
        const arr = docs[0].getArray()
        docs.forEach(doc => {
          t.compare(arr, doc.getArray())
        })
      }
    )
  })
}
