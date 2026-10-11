// Parity: the wave scheduler in workflows/vajra-deliver.js must match `scripts/vajra_tasks.py waves` exactly
// (the workflow cannot read tasks.json, so it recomputes the waves). Run: node tests/workflows/vajra-waves.test.mjs
import assert from 'node:assert/strict'
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { loadWorkflow, responder, scenario, REPO } from './harness.mjs'

const SRC = readFileSync(join(REPO, 'workflows', 'vajra-deliver.js'), 'utf8')
const PY = join(REPO, 'scripts', 'vajra_tasks.py')
const between = (a, b) => { const i = SRC.indexOf(a), j = SRC.indexOf(b, i); assert.ok(i >= 0 && j > i, `markers ${a} .. ${b}`); return SRC.slice(i, j) }
const js = Function(between('const normPath', 'const globRe') + between('function topo(', '// Verifier results') + 'return { waves, overlaps }')()

let python = true
try { execFileSync('python3', ['-c', 'pass']) } catch (e) { python = false }

// Deterministic pseudo-random graphs (LCG), with overlapping files, globs and directory prefixes.
function* graphs(n) {
  let s = 12345
  const rnd = (k) => { s = (Math.imul(s, 1103515245) + 12345) >>> 0; return s % k }
  const pool = ['src/', 'src/a.ts', 'src/b/', 'src/b/c.ts', 'src/*.ts', 'test/', 'test/a.test.ts', 'docs/**', 'docs/x.md', 'README.md', 'lib/x', 'lib/xy', '.']
  for (let g = 0; g < n; g++) {
    const size = 1 + rnd(8), tasks = []
    for (let i = 0; i < size; i++) {
      const deps = []
      for (let j = 0; j < i; j++) if (rnd(4) === 0) deps.push('t' + j)
      const files = []
      for (let k = 0; k < 1 + rnd(2); k++) { const f = pool[rnd(pool.length - (g % 5 ? 1 : 0))]; if (!files.includes(f)) files.push(f) }
      tasks.push({ id: 't' + i, deps, files })
    }
    // Shuffle insertion order so topological order is not just list order.
    for (let i = tasks.length - 1; i > 0; i--) { const j = rnd(i + 1); [tasks[i], tasks[j]] = [tasks[j], tasks[i]] }
    yield tasks
  }
}

console.log('vajra-waves (JS vs vajra_tasks.py)')
await scenario('overlaps(): directory prefixes, globs and the repo root', async () => {
  const yes = [['src/', 'src/a.ts'], ['src/*.ts', 'src/a.ts'], ['docs/**', 'docs/x.md'], ['.', 'anything'], ['src/b', 'src/b/c.ts']]
  const no = [['lib/x', 'lib/xy'], ['src/a.ts', 'src/b.ts'], ['docs/*.md', 'src/a.md'], ['test/', 'tests/']]
  for (const [a, b] of yes) assert.ok(js.overlaps(a, b) && js.overlaps(b, a), `${a} ~ ${b}`)
  for (const [a, b] of no) assert.ok(!js.overlaps(a, b), `${a} !~ ${b}`)
})
await scenario('JS waves() equals vajra_tasks.py compute_waves() on 300 generated graphs, every maxInFlight', async () => {
  if (!python) { console.log('    (skipped: python3 missing)'); return }
  const cases = [...graphs(300)]
  const caps = [null, 1, 2, 3, 4]
  const out = execFileSync('python3', ['-I', '-c',
    'import importlib.util, json, sys\n' +
    'spec = importlib.util.spec_from_file_location("vt", sys.argv[1]); m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)\n' +
    'cases = json.load(sys.stdin)\n' +
    'print(json.dumps([[m.compute_waves(c, cap) for cap in [None, 1, 2, 3, 4]] for c in cases]))', PY],
  { input: JSON.stringify(cases) }).toString()
  const py = JSON.parse(out)
  cases.forEach((c, i) => caps.forEach((cap, k) => assert.deepEqual(js.waves(c, cap), py[i][k], `graph ${i} cap ${cap}: ${JSON.stringify(c)}`)))
  // Overlap really separates tasks: some generated graph has two dependency-free tasks in different waves.
  const levels = (c) => { const by = Object.fromEntries(c.map((t) => [t.id, t])), memo = {}
    const lv = (id) => memo[id] !== undefined ? memo[id] : (memo[id] = Math.max(0, ...by[id].deps.map((d) => lv(d) + 1)))
    return Math.max(...c.map((t) => lv(t.id))) + 1 }
  assert.ok(cases.filter((c) => js.waves(c).length > levels(c)).length >= 10, 'file overlaps add waves beyond the dependency levels')
})
await scenario('the CLI agrees with the workflow on a real delivery plan', async () => {
  if (!python) { console.log('    (skipped: python3 missing)'); return }
  const HOME = mkdtempSync(join(tmpdir(), 'vajra-waves-'))
  try {
    mkdirSync(join(HOME, '.claude', 'vajra', 'campaigns', 'deliver-x'), { recursive: true })
    const env = { ...process.env, HOME }
    const T = (...a) => execFileSync('python3', [PY, ...a, '--campaign', 'deliver-x'], { env }).toString()
    const tasks = [
      { id: 't1', owner: 'bhima', files: ['src/a/'], deps: [] },
      { id: 't2', owner: 'nakula', files: ['src/b/'], deps: [] },
      { id: 't3', owner: 'abhimanyu', files: ['README.md'], deps: [] },
      { id: 't4', owner: 'sepoy-test-1', files: ['test/'], deps: ['t1', 't2'] },
      { id: 't5', owner: 'bhima', files: ['docs/'], deps: ['t4'] },
    ]
    for (const t of tasks) T('add', t.id, '--title', t.id, '--owner', t.owner, '--files', t.files.join(','), ...(t.deps.length ? ['--deps', t.deps.join(',')] : []))
    for (const cap of [4, 2, 1]) {
      const cli = JSON.parse(T('waves', '--max-in-flight', String(cap), '--json'))
      assert.deepEqual(cli, js.waves(tasks, cap), `cap ${cap}`)
      // And the deliver workflow logs the same batches for this plan.
      const wf = loadWorkflow(join(REPO, 'workflows', 'vajra-deliver.js'))
      const plan = { tasks: tasks.map((t) => ({ id: t.id, title: t.id, owner: t.owner, files: t.files, deps: t.deps, criteria: ['ac-1'], brief: 'b' })), decisions: [], protected_actions: [] }
      const r = await wf.run({ ask: 'Build five things in waves please', run_id: 'w20261011', maxInFlight: cap }, responder({
        'kb.search': { ok: true, lines: [] },
        understand: { restated: 'r', acceptance_criteria: [{ id: 'ac-1', criterion: 'works', check: 'npm test' }], in_scope: [], out_of_scope: [], assumptions: [], blocking_question: '', touches_security: false, deploy_requested: false },
        preflight: { head: 'a'.repeat(40), current_branch: 'main', dirty: false, base: '', campaign_ok: true, campaign_output: '' },
        plan, tasks: { ok: true, outputs: [] },
        task: () => new Error('stop after the schedule is logged'),
        campaign: { ok: true, outputs: [] },
      }))
      assert.ok(r.logs.includes(`waves (maxInFlight ${cap}): ${JSON.stringify(cli)}`), r.logs.join('\n'))
    }
  } finally { rmSync(HOME, { recursive: true, force: true }) }
})
