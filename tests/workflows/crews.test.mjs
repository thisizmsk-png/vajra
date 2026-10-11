// Offline scenarios for the crew workflows: sabha, architecture-review, incident-response, feature-build.
// Run: node tests/workflows/crews.test.mjs
import assert from 'node:assert/strict'
import { join } from 'node:path'
import { readdirSync } from 'node:fs'
import { loadWorkflow, responder, assertSafe, scenario, REPO } from './harness.mjs'

const W = (n) => loadWorkflow(join(REPO, 'workflows', n + '.js'))
const labels = (r) => r.calls.map((c) => c.label)
const call = (r, l) => r.calls.find((c) => c.label === l)
const HEAD = 'a'.repeat(40)
const branchIn = (p) => p.match(/git checkout -B (\S+) /)[1]

console.log('workflow files')
await scenario('every workflows/*.js loads under the runtime rules', async () => {
  const files = readdirSync(join(REPO, 'workflows')).filter((f) => f.endsWith('.js'))
  assert.ok(files.length >= 5)
  for (const f of files) assert.ok(f.startsWith('vajra-'), f), W(f.replace(/\.js$/, ''))
})

// ---------------- sabha
console.log('vajra-sabha')
const sabha = W('vajra-sabha')