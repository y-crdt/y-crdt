#!/usr/bin/env node
import { setBenchmarkResult, writeBenchmarkResultsToFile } from '../js-lib/index.js'
import fs from 'fs'
import { join } from 'path'

const currDir = process.cwd()
const pkgLock = JSON.parse(fs.readFileSync(currDir + '/package-lock.json', 'utf8'))
const name = pkgLock.name

const bundleSize = fs.statSync(join(currDir, '/dist/bundle.js')).size
const gzBundleSize = fs.statSync(join(currDir, '/dist/bundle.js.gz')).size

const version = pkgLock.dependencies[name].version

setBenchmarkResult(name, 'Version', version)
setBenchmarkResult(name, 'Bundle size', `${bundleSize} bytes`)
setBenchmarkResult(name, 'Bundle size (gzipped)', `${gzBundleSize} bytes`)
writeBenchmarkResultsToFile('../results.json', () => false)
