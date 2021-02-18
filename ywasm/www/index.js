import * as wasm from 'ywasm'

let start = performance.now()
const doc = new wasm.Doc()
const ytext = doc.getType('type')
const tr = doc.transact()
for (let i = 0; i < 1000000; i++) {
  ytext.insert(tr, 0, (i % 10).toString())
}

console.log('time to apply prepends: ', performance.now() - start)
console.log('text length: ', ytext.toString().length)
const encodedState = doc.encodeStateAsUpdate()
console.log('update message size: ', encodedState.length)

const doc2 = new wasm.Doc()
const ytext2 = doc.getType('type')

start = performance.now()
doc2.applyUpdate(encodedState)

console.log('time to apply update message: ', performance.now() - start)
console.log('text length: ', ytext2.toString().length)
