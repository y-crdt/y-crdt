#!/usr/bin/env node

import { edits, finalText } from '../js-lib/b4-editing-trace.js'

const counts = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]

const repeats = 1000

console.log('text length', finalText.length)

// apply the same dataset {repeats} times
for (let rep = 0; rep < repeats; rep++) {
  // adjust add to start editing at a random position in the existing document
  const add = finalText.length * rep // Math.round(finalText.length * rep * Math.random())

  // iterate through the dataset
  for (let i = 0; i < edits.length; i++) {
    counts[Number((edits[i][0] + add).toString()[0])]++
  }
}

// index counts
console.log('counts', counts)

// compute the distribution
for (let i = 0; i < counts.length; i++) {
  counts[i] /= edits.length * repeats
}

console.log('distribution', counts)
