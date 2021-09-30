
import * as map from './y-map.tests.js'
import * as array from './y-array.tests.js'
import * as text from './y-text.tests.js'
import * as xml from './y-xml.tests.js'

import { runTests } from 'lib0/testing'
import { isBrowser, isNode } from 'lib0/environment'
import * as log from 'lib0/logging'

if (isBrowser) {
    log.createVConsole(document.body)
}
runTests({
    array, map, text, xml
}).then(success => {
    /* istanbul ignore next */
    if (isNode) {
        process.exit(success ? 0 : 1)
    }
})
