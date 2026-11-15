// Parity (ADR-16 16.5a, PR 6g): the core's Rust port of the workflow's verify and review rules
// (app/core/src/desktop/verify.rs) and workflows/vajra-deliver.js give the same result on every case in
// tests/fixtures/verify/cases.json. This test runs the JS itself on each case; `verify::tests::parity_with_the_workflow`
// runs the port on the same file. The file pins the sha256 of the ported JS blocks (the core's `GATE_VERSION`), so a
// rule change fails here until the fixtures are regenerated, and then the Rust test fails until the port follows.
// Two blocks sit inline in the deliver loop; they run here as written, with one stated substitution (the gate's
// `break` becomes a return).
// Run: node tests/workflows/vajra-verify-parity.test.mjs        Regenerate: the same with --write
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { scenario, REPO } from './harness.mjs'

const SRC = readFileSync(join(REPO, 'workflows', 'vajra-deliver.js'), 'utf8')
const FIXTURES = join(REPO, 'tests', 'fixtures', 'verify', 'cases.json')
const between = (a, b) => { const i = SRC.indexOf(a), j = SRC.indexOf(b, i); assert.ok(i >= 0 && j > i, `markers ${a} .. ${b}`); return SRC.slice(i, j) }
// The ported blocks, in this order. verify.rs hashes the same slices (GATE_VERSION).
const PORTED = {
  sec: between('const SEC_RE', 'const CAMPAIGN_SH'),
  rules: between('const normText', '// Reviewer risks'),
  suite: between('const suitePasses', 'const flatten'),
  redo: between('    const fb = {', '    const v2 ='),
  reject: between('  const rejected = reviews.find', '  if (rejected)'),
  gate: between('  const byC = resultsByCriterion(verification.results, CRITERIA)', '  log(`round ${round}: ${failing.length}'),
  flag: between('const SECURITY = ', '\n'),
}
const GATE_SHA = createHash('sha256').update(Object.values(PORTED).join('')).digest('hex')
// oneLine and topo come from the plan rules (already pinned by vajra-plan-parity).
const BASE = between('const clean = ', 'function fnv(') + between('function topo(', 'function waves(')
const BREAK = 'delivered = true; break'
assert.equal(PORTED.gate.split(BREAK).length, 2, 'the gate block has one break')

const js = Function(BASE + PORTED.sec + PORTED.rules + PORTED.suite + `
  const redo = (v, CRITERIA) => { const problems = verificationProblems(v, CRITERIA)\n${PORTED.redo}\n return fb }
  const rejectedBy = (reviews) => {\n${PORTED.reject}\n return !!rejected }
  const gate = (verification, CRITERIA, TASKS, reviews) => {
    const problems = verificationProblems(verification, CRITERIA), round = 1, verified = []
    const byId = Object.fromEntries(TASKS.map((t) => [t.id, t])), ORDER = topo(TASKS)
    let feedbackFor = {}
    ${PORTED.gate.replace(BREAK, 'return { passed: true, failing: [], suite_ok: suiteOk, feedback: {} }')}
    return { passed: false, failing: failing.map((c) => c.id), suite_ok: suiteOk, feedback: feedbackFor }
  }
  const security = (SEC_RE, understanding, ASK, CRITERIA) => {\n${PORTED.flag}\n return SECURITY }
  return { resultsByCriterion, commandProblem, setupOf, verificationProblems, redo, rejectedBy, gate, security, SEC_RE }`)()

function run(c, d) {
  const criteria = c.criteria || d.criteria
  switch (c.kind) {
    case 'results_by_criterion': {
      const out = js.resultsByCriterion(c.results, criteria)
      return Object.fromEntries(Object.entries(out).map(([id, r]) => [id, c.results.indexOf(r)]))
    }
    case 'command_problem': return { problem: js.commandProblem(c.cmd, c.setup || [], c.kind_of || 'check') }
    case 'setup_of': return { setup: js.setupOf({ setup_cmds: c.setup_cmds }) }
    case 'verification_problems': return js.verificationProblems(c.verification, criteria)
    case 'redo_feedback': return js.redo(c.verification, criteria)
    case 'gate': {
      const reviews = c.reviews || []
      return { ...js.gate(c.verification, criteria, c.tasks || d.tasks, reviews), rejected: js.rejectedBy(reviews) }
    }
    case 'security': return { flagged: js.security(js.SEC_RE, { touches_security: c.touches_security }, c.ask, c.criteria_text.map((t) => ({ criterion: t }))) }
    default: throw new Error(`unknown kind ${c.kind}`)
  }
}

const F = JSON.parse(readFileSync(FIXTURES, 'utf8'))
if (process.argv.includes('--write')) {
  F.js_gate_sha256 = GATE_SHA
  for (const c of F.cases) c.expect = run(c, F.defaults)
  const { cases, ...head } = F
  const top = JSON.stringify(head, null, 2).replace(/\n}$/, '')
  writeFileSync(FIXTURES, `${top},\n  "cases": [\n${cases.map((c) => '    ' + JSON.stringify(c)).join(',\n')}\n  ]\n}\n`)
  console.log(`wrote ${F.cases.length} cases`)
} else {
  console.log('vajra-verify-parity (JS verify and review rules vs tests/fixtures/verify)')
  await scenario('the ported JS blocks are the ones the fixtures were made from', async () => {
    assert.equal(F.js_gate_sha256, GATE_SHA,
      'the verify or review rules in workflows/vajra-deliver.js changed: run this test with --write, review cases.json, port the change to app/core/src/desktop/verify.rs and update GATE_VERSION')
  })
  await scenario(`the JS rules give each fixture's result (${F.cases.length} cases)`, async () => {
    for (const c of F.cases) assert.deepEqual(run(c, F.defaults), c.expect, c.name)
  })
  await scenario('the fixtures reach every rule', async () => {
    const problems = F.cases.filter((c) => c.kind === 'command_problem').map((c) => c.expect.problem).filter(Boolean)
    for (const want of ['no command', 'note in parentheses', 'elides', 'outside the repository', 'masks the exit code', 'bare python'])
      assert.ok(problems.some((p) => p.includes(want)), want)
    const fb = F.cases.filter((c) => c.kind === 'gate').flatMap((c) => Object.values(c.expect.feedback).flatMap(Object.keys))
    for (const want of ['failing_criteria', 'suite_command_problem', 'blocking_issues', 'suite_regressions']) assert.ok(fb.includes(want), want)
    assert.ok(F.cases.some((c) => c.kind === 'gate' && c.expect.passed) && F.cases.some((c) => c.kind === 'gate' && c.expect.rejected))
  })
}
