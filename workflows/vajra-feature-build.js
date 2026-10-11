export const meta = {
  name: 'vajra-feature-build',
  description: 'Vajra feature-build crew: arjuna designs and splits the work, bhima/nakula/sepoys build in worktrees, vidura verifies every criterion, arjuna and bhishma review; returns one local branch',
  whenToUse: 'A full-stack feature with a clear goal ("/vajra fleet feature-build ..."). Args: the feature as text, or JSON {"feature": "...", "run_id": "20261011-0930", "max_rounds": 3}',
  phases: [
    { title: 'Brief', detail: 'validate the feature request' },
  ],
}

// The feature-build crew is the vajra-deliver pipeline with arjuna (principal SDE) as the planner and the
// writer roster narrowed to the build crew. vajra-deliver supplies the typed nodes, the bounded fix loop,
// independent verification and the no-protected-actions rule; it must be installed next to this script.

function parseArgs(a) {
  if (a === undefined || a === null) return {}
  if (typeof a !== 'string') return a
  try {
    const j = JSON.parse(a)
    return j && typeof j === 'object' && !Array.isArray(j) ? j : { feature: a }
  } catch (e) {
    return { feature: a }
  }
}
const A = parseArgs(args)
const FEATURE = String(A.feature || A.ask || '').replace(/[\u0000-\u0008\u000b-\u001f\u007f]/g, ' ').trim().slice(0, 20000)
if (FEATURE.length < 10) {
  return { status: 'error', node: 'input', error: 'a feature description of 10+ characters is required', artifacts: {} }
}
const SLUG_RE = /^[a-z0-9][a-z0-9-]{2,40}$/
if (A.run_id !== undefined && !(typeof A.run_id === 'string' && SLUG_RE.test(A.run_id))) {
  return { status: 'error', node: 'input', error: 'run_id must be a slug like 20261011-0930', artifacts: {} }
}

phase('Brief')
log('feature-build: arjuna plans; bhima, nakula and the sepoys build; vidura verifies')
const childArgs = {
  ask: 'Build this feature end to end, with tests: ' + FEATURE,
  planner: 'arjuna',
  writers: ['bhima', 'nakula', 'sepoy-backend-1', 'sepoy-frontend-1', 'sepoy-test-1'],
  hints: { crews: ['feature-build'] },
}
if (A.run_id) childArgs.run_id = A.run_id
if (Number.isInteger(A.max_rounds)) childArgs.max_rounds = A.max_rounds
let r
try {
  r = await workflow('vajra-deliver', childArgs)
} catch (e) {
  return { status: 'error', node: 'deliver', error: 'vajra-deliver workflow unavailable: ' + String(e && e.message ? e.message : e), artifacts: {} }
}
if (!r || typeof r !== 'object' || !['done', 'need_input', 'rejected', 'error'].includes(r.status)) {
  return { status: 'error', node: 'deliver', error: 'vajra-deliver returned no valid result', artifacts: {} }
}
return Object.assign({}, r, { crew: 'feature-build', node: 'deliver/' + r.node, artifacts: r.artifacts || {} })
