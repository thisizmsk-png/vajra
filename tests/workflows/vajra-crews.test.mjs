// Offline scenarios for the crew workflows that wrap or mirror vajra-deliver:
//   vajra-feature-build, vajra-security-audit, vajra-architecture-review, vajra-incident-response.
// Run: node tests/workflows/vajra-crews.test.mjs
import assert from 'node:assert/strict'
import { join } from 'node:path'
import { loadWorkflow, responder, assertSafe, scenario, REPO } from './harness.mjs'

const HEAD = 'a'.repeat(40)
const branchIn = (p) => (p.match(/git checkout -B (\S+) /) || [])[1]
const go = (wf, base) => async (args, over) => { const r = await wf.run(args, responder(base, over)); assertSafe(r.calls); return r }
const labels = (r) => r.calls.map((c) => c.label)
const call = (r, l) => r.calls.find((c) => c.label === l)

// ---------------------------------------------------------------- feature-build (wraps vajra-deliver)
{
  const wf = loadWorkflow(join(REPO, 'workflows', 'vajra-feature-build.js'))
  const RUN = 'fb20261011'
  const ch = (id, p) => ({ status: 'done', branch: branchIn(p), base_commit: HEAD, commit: (id === 't1' ? 'b' : 'c') + '111111', files: id === 't1' ? ['src/a.ts'] : ['test/a.test.ts'], summary: 's', checks_run: ['t'], question: '', protected_actions_skipped: [] })
  const base = {
    understand: { restated: 'r', acceptance_criteria: [{ id: 'ac-1', criterion: 'works', check: 'npm test' }], in_scope: [], out_of_scope: [], assumptions: [], blocking_question: '', touches_security: false, deploy_requested: false },
    preflight: { head: HEAD, current_branch: 'main', dirty: false, base: '', campaign_ok: true, campaign_output: '' },
    'plan.1': { tasks: [{ id: 't1', title: 'impl', owner: 'bhima', files_owned: ['src/a.ts'], depends_on: [], criteria: ['ac-1'], brief: 'b' }, { id: 't2', title: 'tests', owner: 'sepoy-test-1', files_owned: ['test/a.test.ts'], depends_on: ['t1'], criteria: ['ac-1'], brief: 'b' }], protected_actions: [] },
    'task.t1': (l, o, p) => ch('t1', p),
    'task.t2': (l, o, p) => ch('t2', p),
    merge: (l, o, p) => ({ status: 'merged', branch: branchIn(p), commit: 'd1111111', merged: ['t1', 't2'], conflicts: [], notes: '' }),
    verify: { results: [{ criterion_id: 'ac-1', status: 'pass', command: 'npm test', exit_code: 0, evidence: 'ok' }], suite_command: 'npm test', suite_passes: true, regressions: [] },
    review: { verdict: 'approve', blocking_issues: [], risks: [], rationale: 'ok' },
    campaign: { ok: true, outputs: ['ok'] },
  }
  const run = go(wf, base)
  console.log('vajra-feature-build')
  await scenario('delegates to vajra-deliver with arjuna as planner and the build roster; returns its result', async () => {
    const r = await run({ feature: 'Add a CSV export endpoint', run_id: RUN })
    assert.equal(r.result.status, 'done')
    assert.equal(r.result.crew, 'feature-build')
    assert.match(r.result.node, /^deliver\//)
    assert.equal(call(r, 'plan.1').agentType, 'arjuna', 'arjuna plans in feature-build')
    assert.match(call(r, 'understand').prompt, /Build this feature end to end/)
    assert.match(call(r, 'understand').prompt, /feature-build/)
  })
  await scenario('string arg is the feature; short feature rejected', async () => {
    assert.equal((await run('Add a CSV export endpoint to the reports service')).result.status, 'done')
    const e = await run('short'); assert.equal(e.result.status, 'error'); assert.equal(e.calls.length, 0)
  })
  await scenario('bad run_id rejected before delegating', async () => {
    const r = await run({ feature: 'Add a CSV export endpoint', run_id: 'Bad Id' })
    assert.equal(r.result.status, 'error'); assert.equal(r.calls.length, 0)
  })
}

// ---------------------------------------------------------------- security-audit (read-only review)
{
  const wf = loadWorkflow(join(REPO, 'workflows', 'vajra-security-audit.js'))
  const finding = (title, sev, file) => ({ threat: 'T1', title, category: 'Injection', severity: sev, file, line: 10, evidence: 'e', recommended_fix: 'f' })
  const base = {
    threat_model: { assets: ['db'], entry_points: ['/api'], threats: [{ id: 'T1', title: 'injection', category: 'Injection', severity: 'high', location: 'src/db.ts', rationale: 'r' }] },
    'review.bhishma': { findings: [finding('unparameterized query', 'high', 'src/db.ts')] },
    'review.shakuni': { findings: [finding('unparameterized query', 'critical', 'src/db.ts'), finding('verbose errors', 'low', 'src/err.ts')] },
    confirm: { confirmed: true, severity: 'critical', accuracy: 'accurate', notes: 'real' },
    gate: { verdict: 'fail', summary: 'one high finding', remediation: [{ finding: 'F1', fix: 'parameterize', suggested_owner: 'bhima', priority: 'P0' }] },
  }
  const run = go(wf, base)
  console.log('vajra-security-audit')
  await scenario('read-only; findings merged, de-duped and ranked; confirmed high fails the gate', async () => {
    const r = await run({ scope: 'src/db.ts' })
    assert.equal(r.result.status, 'done')
    for (const c of r.calls) assert.equal(c.isolation, undefined, c.label + ' must be read-only')
    assert.equal(r.result.gate.verdict, 'fail')
    assert.equal(r.result.findings.length, 2, 'duplicate db.ts/Injection merged')
    const f1 = r.result.findings[0]
    assert.equal(f1.severity, 'critical', 'most severe wins on merge')
    assert.deepEqual(f1.found_by.sort(), ['bhishma', 'shakuni'])
    assert.ok(labels(r).includes('confirm.F1'))
    assert.equal(call(r, 'confirm.F1').agentType, 'vidura')
  })
  await scenario('gate forced to fail in code even if bhishma says pass, when a high is confirmed', async () => {
    const r = await run({ scope: 'src/db.ts' }, { gate: { verdict: 'pass', summary: 'lgtm', remediation: [] } })
    assert.equal(r.result.gate.verdict, 'fail')
  })
  await scenario('a finding downgraded to not_a_defect does not force a fail', async () => {
    const r = await run({ scope: 'src/db.ts' }, {
      'review.shakuni': { findings: [finding('unparameterized query', 'high', 'src/db.ts')] },
      confirm: { confirmed: false, severity: 'high', accuracy: 'not_a_defect', notes: 'false positive' },
      gate: { verdict: 'pass', summary: 'clear', remediation: [] },
    })
    assert.equal(r.result.gate.verdict, 'pass')
  })
  await scenario('no findings -> pass without a confirm or gate agent', async () => {
    const r = await run({ scope: 'src/empty.ts' }, { 'review.bhishma': { findings: [] }, 'review.shakuni': { findings: [] } })
    assert.equal(r.result.status, 'done'); assert.equal(r.result.gate.verdict, 'pass')
    assert.ok(!labels(r).some((l) => l.startsWith('confirm')))
  })
  await scenario('paths with traversal are dropped during the merge', async () => {
    const r = await run({ scope: 'src' }, { 'review.bhishma': { findings: [finding('x', 'high', '../../etc/passwd')] }, 'review.shakuni': { findings: [] } })
    assert.equal(r.result.findings.length, 0)
  })
  await scenario('both reviewers silent -> error', async () => {
    const r = await run({ scope: 'src' }, { 'review.bhishma': null, 'review.shakuni': null })
    assert.equal(r.result.status, 'error'); assert.equal(r.result.node, 'review')
  })
}

// ---------------------------------------------------------------- architecture-review (one ADR writer)
{
  const wf = loadWorkflow(join(REPO, 'workflows', 'vajra-architecture-review.js'))
  const RUN = 'ar20261011'
  const vote = (v, issues = []) => ({ verdict: v, blocking_issues: issues, rationale: 'r' })
  const adr = (p) => ({ status: 'done', branch: branchIn(p), base_commit: HEAD, commit: branchIn(p).includes('-r2') ? 'c222222' : 'b111111', path: `docs/adr/vajra-${RUN}.md`, summary: 's', question: '' })
  const base = {
    preflight: { head: HEAD },
    adr: (l, o, p) => adr(p),
    vote: vote('approve'),
    premortem: { failure_modes: [{ id: 'F1', mode: 'cost', impact: 'medium', likelihood: 'low' }] },
    ruling: { decision: 'accept', conditions: [], failure_mode_responses: [{ id: 'F1', response: 'budgeted' }], rationale: 'r' },
  }
  const run = go(wf, base)
  console.log('vajra-architecture-review')
  await scenario('only arjuna writes (worktree), reviewers read-only, yudhishthira rules, premortem answered', async () => {
    const r = await run({ subject: 'Should we move session storage to Redis?', run_id: RUN })
    assert.equal(r.result.status, 'done')
    assert.equal(call(r, 'adr.r1').agentType, 'arjuna')
    assert.equal(call(r, 'adr.r1').isolation, 'worktree')
    for (const l of ['vote.bhishma.r1', 'vote.draupadi.r1', 'vote.hanuman.r1', 'premortem', 'ruling.1']) assert.equal(call(r, l).isolation, undefined, l)
    assert.equal(call(r, 'ruling.1').agentType, 'yudhishthira')
    assert.match(call(r, 'adr.r1').prompt, /git checkout -B vajra\/ar20261011\/adr a{40}/)
  })
  await scenario('no majority -> revise the ADR up to max_rounds', async () => {
    const r = await run({ subject: 'Should we move session storage to Redis?', run_id: RUN, max_rounds: 2 }, {
      vote: (label) => vote(['approve', 'revise', 'reject'][['bhishma', 'draupadi', 'hanuman'].indexOf(label.split('.')[1])]),
    })
    assert.ok(labels(r).includes('adr.r2'))
    assert.match(call(r, 'adr.r2').prompt, /git checkout -B vajra\/ar20261011\/adr-r2 b111111/)
  })
  await scenario('without a majority approve, ruling cannot be "accept" (retried, then errors)', async () => {
    const noMaj = { vote: (label) => vote(['approve', 'revise', 'reject'][['bhishma', 'draupadi', 'hanuman'].indexOf(label.split('.')[1])]) }
    const r = await run({ subject: 'Should we move session storage to Redis?', run_id: RUN, max_rounds: 1 }, Object.assign({}, noMaj, { ruling: { decision: 'accept', conditions: [], failure_mode_responses: [{ id: 'F1', response: 'x' }], rationale: 'r' } }))
    assert.equal(r.result.status, 'error'); assert.match(r.result.error, /accept/)
  })
  await scenario('ruling that skips a failure mode is retried once', async () => {
    const r = await run({ subject: 'Should we move session storage to Redis?', run_id: RUN }, { 'ruling.1': { decision: 'accept', conditions: [], failure_mode_responses: [], rationale: 'r' } })
    assert.ok(labels(r).includes('ruling.2'))
  })
  await scenario('reject ruling -> rejected; defer -> need_input', async () => {
    const rej = await run({ subject: 'Should we move session storage to Redis?', run_id: RUN }, { vote: vote('reject'), ruling: { decision: 'reject', conditions: [], failure_mode_responses: [{ id: 'F1', response: 'x' }], rationale: 'r' } })
    assert.equal(rej.result.status, 'rejected')
    const def = await run({ subject: 'Should we move session storage to Redis?', run_id: RUN }, { ruling: { decision: 'defer', conditions: [], failure_mode_responses: [{ id: 'F1', response: 'x' }], rationale: 'r' } })
    assert.equal(def.result.status, 'need_input')
  })
  await scenario('invalid ADR change is not trusted', async () => {
    const r = await run({ subject: 'Should we move session storage to Redis?', run_id: RUN }, { 'adr.r1': { status: 'done', branch: 'main', base_commit: HEAD, commit: 'b111111', path: `docs/adr/vajra-${RUN}.md`, summary: 's', question: '' } })
    assert.equal(r.result.status, 'error'); assert.match(r.result.error, /invalid AdrChange/)
  })
}

// ---------------------------------------------------------------- incident-response (mitigation gate + fix)
{
  const wf = loadWorkflow(join(REPO, 'workflows', 'vajra-incident-response.js'))
  const RUN = 'ir20261011'
  const base = {
    preflight: { head: HEAD },
    triage: { summary: 's', severity: 'SEV2', impact: '5% of requests 500', symptoms: ['500s'], hypotheses: [{ id: 'H1', statement: 'null profile', check: 'curl' }], blocking_question: '' },
    'investigate.code': { hypothesis_results: [{ id: 'H1', verdict: 'confirmed', evidence: 'stack' }], root_cause: 'missing null guard', files: ['src/user.ts'], confidence: 'high' },
    'investigate.infra': { recent_changes: ['deploy 123'], config_findings: [], suspect_change: 'deploy 123', evidence: ['timing'] },
    mitigate: { options: [{ kind: 'rollback', steps: ['rollback 123'], undo: ['redeploy'], reversible: true, risk: 'low' }], recommended: 0, rationale: 'fast and reversible' },
    fix: (l, o, p) => ({ status: 'done', branch: branchIn(p), base_commit: HEAD, commit: 'b111111', files: ['src/user.ts', 'test/user.test.ts'], regression_test: 'test/user.test.ts', test_command: 'npm test', summary: 'guard', question: '' }),
    verify: { verdict: 'pass', fails_before: true, passes_after: true, suite_passes: true, evidence: ['before fail', 'after pass'] },
    postmortem: { timeline: ['t'], root_cause: 'missing null guard', contributing_factors: ['no test'], action_items: [{ item: 'add test', owner: 'bhima' }], blameless: true },
  }
  const run = go(wf, base)
  console.log('vajra-incident-response')
  await scenario('triage, parallel read-only investigate, reversible mitigation gate, verified fix', async () => {
    const r = await run({ incident: 'GET /users/{id} returns 500 when the profile row is missing' })
    assert.equal(r.result.status, 'need_input'); assert.equal(r.result.node, 'mitigation_gate')
    assert.equal(call(r, 'triage').agentType, 'ashwatthama')
    assert.equal(call(r, 'investigate.code').isolation, undefined)
    assert.equal(call(r, 'investigate.infra').isolation, undefined)
    assert.equal(call(r, 'fix.r1').isolation, 'worktree')
    assert.equal(call(r, 'verify.r1').isolation, 'worktree')
    assert.equal(r.result.artifacts.fix_status, 'verified')
    assert.match(r.result.question, /Approve and run the recommended mitigation/)
  })
  await scenario('mitigation plan is never executed; prompt forbids it', async () => {
    const r = await run({ incident: 'GET /users/{id} returns 500 when the profile row is missing' })
    assert.match(call(r, 'mitigate').prompt, /Do not execute anything/)
    assert.match(call(r, 'mitigate').prompt, /do not create, modify or delete any file/)
  })
  await scenario('blocking triage question -> need_input at triage', async () => {
    const r = await run({ incident: 'Something is wrong with the whole site right now please help' }, { triage: { ...base.triage, blocking_question: 'Which region is affected?' } })
    assert.equal(r.result.status, 'need_input'); assert.equal(r.result.node, 'triage')
  })
  await scenario('non-reversible recommended mitigation is flagged for a human choice', async () => {
    const r = await run({ incident: 'GET /users/{id} returns 500 when the profile row is missing' }, { mitigate: { options: [{ kind: 'other', steps: ['drop the table'], undo: [], reversible: false, risk: 'high' }], recommended: 0, rationale: 'r' } })
    assert.match(r.result.question, /not clearly reversible/)
  })
  await scenario('low-confidence root cause -> no fix attempted', async () => {
    const r = await run({ incident: 'Intermittent slowness across the API for the last hour or so' }, { 'investigate.code': { ...base['investigate.code'], confidence: 'low' } })
    assert.ok(!labels(r).some((l) => l.startsWith('fix')))
    assert.equal(r.result.artifacts.fix_status, 'skipped')
  })
  await scenario('fix never verifies -> fix_status unverified after max_rounds', async () => {
    const r = await run({ incident: 'GET /users/{id} returns 500 when the profile row is missing', max_rounds: 2 }, { verify: { ...base.verify, passes_after: false, verdict: 'fail' } })
    assert.equal(r.result.artifacts.fix_status, 'unverified')
    assert.ok(labels(r).includes('verify.r2') && !labels(r).includes('verify.r3'))
  })
  await scenario('both investigators silent -> error', async () => {
    const r = await run({ incident: 'GET /users/{id} returns 500 when the profile row is missing' }, { 'investigate.code': null, 'investigate.infra': null })
    assert.equal(r.result.status, 'error'); assert.equal(r.result.node, 'investigate')
  })
}
