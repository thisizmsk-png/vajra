// Parity (ADR-16 16.5, PR 6d1): the core's Rust port of the workflow's plan rules (app/core/src/desktop/plan.rs) and
// workflows/vajra-deliver.js give the same result on every case in tests/fixtures/plan/cases.json. This test runs the
// JS functions themselves on each case; the Rust test `plan::tests::parity_with_the_workflow` runs the port on the same
// file. The file also pins the sha256 of the JS rule block, so a change to the rules fails here until the fixtures are
// regenerated (and then the Rust test fails until the port follows).
// Run: node tests/workflows/vajra-plan-parity.test.mjs        Regenerate: the same with --write
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { scenario, REPO } from './harness.mjs'

const SRC = readFileSync(join(REPO, 'workflows', 'vajra-deliver.js'), 'utf8')
const FIXTURES = join(REPO, 'tests', 'fixtures', 'plan', 'cases.json')
const between = (a, b) => { const i = SRC.indexOf(a), j = SRC.indexOf(b, i); assert.ok(i >= 0 && j > i, `markers ${a} .. ${b}`); return SRC.slice(i, j) }
// The ported blocks, in this order. plan.rs hashes the same three slices.
const RULES = [between('const clean = ', 'function fnv('), between('const normPath', 'const globRe'), between('function validateUnderstanding(', '// Verifier results')]
const RULES_SHA = createHash('sha256').update(RULES.join('')).digest('hex')
const CHECKERS = ['verifier', 'reviewer', 'security_reviewer']

// `WRITERS` is free in validatePlan: here it is the run's writer seat roles, so "owner has a writer" is "owner_role
// has a seat" (16.5). A plan with no decisions, as the core passes them separately.
const load = (writers) => Function('WRITERS', RULES.join('\n') + 'return { validateUnderstanding, validatePlan, topo, waves }')(writers)

function run(c, d) {
  const seats = c.seats || d.seats
  const writers = Object.fromEntries(seats.filter((r) => !CHECKERS.includes(r)).map((r) => [r, r]))
  const js = load(writers)
  const cap = 'max_in_flight' in c ? c.max_in_flight : d.max_in_flight
  if (c.kind === 'criteria') return { error: js.validateUnderstanding({ acceptance_criteria: c.criteria }, null) }
  const tasks = c.tasks.map((t) => ({ id: t.id, title: t.title ?? 'title', owner: t.owner_role ?? 'backend', files: t.files, deps: t.deps ?? [],
    criteria: t.criteria ?? [], brief: t.brief ?? 'brief' }))
  if (c.kind === 'waves') return { order: js.topo(tasks), waves: js.waves(tasks, cap) }
  const error = js.validatePlan({ tasks, decisions: [] }, c.criteria || d.criteria)
  return error ? { error } : { error: null, order: js.topo(tasks), waves: js.waves(tasks, cap) }
}

const F = JSON.parse(readFileSync(FIXTURES, 'utf8'))
if (process.argv.includes('--write')) {
  F.js_rules_sha256 = RULES_SHA
  for (const c of F.cases) c.expect = run(c, F.defaults)
  // One line per case, so a rule change shows as the cases it changed.
  const { cases, ...head } = F
  const top = JSON.stringify(head, null, 2).replace(/\n}$/, '')
  writeFileSync(FIXTURES, `${top},\n  "cases": [\n${cases.map((c) => '    ' + JSON.stringify(c)).join(',\n')}\n  ]\n}\n`)
  console.log(`wrote ${F.cases.length} cases`)
} else {
  console.log('vajra-plan-parity (JS rules vs tests/fixtures/plan)')
  await scenario('the JS rule block is the one the fixtures were made from', async () => {
    assert.equal(F.js_rules_sha256, RULES_SHA,
      'the plan rules in workflows/vajra-deliver.js changed: run this test with --write, review cases.json, and port the change to app/core/src/desktop/plan.rs')
  })
  await scenario(`the JS rules give each fixture's result (${F.cases.length} cases)`, async () => {
    for (const c of F.cases) assert.deepEqual(run(c, F.defaults), c.expect, c.name)
  })
  await scenario('the fixtures cover every rule', async () => {
    const errors = F.cases.map((c) => c.expect.error).filter(Boolean)
    for (const want of ['1-12 entries', 'must look like ac-1', 'duplicate criterion', 'needs text and a check', 'plan must have 1-8 tasks', 'must look like t1',
      'duplicate task id', 'owner must be one of', 'needs a title and a brief', 'at least one path', 'unsafe path', 'unknown criterion', 'is not covered',
      'bad dependency', 'files must be disjoint', 'contain a cycle'])
      assert.ok(errors.some((e) => e.includes(want)), want)
  })
}
