// Offline stub harness for Vajra workflow scripts (workflows/vajra-*.js).
// Runs a script body with stubbed agent/parallel/pipeline/phase/log/args/budget/workflow globals, the way the
// Claude Code workflow runtime does, and checks the runtime's rules: pure-literal meta first, phases declared,
// no clock/randomness/imports, valid schemas, unique labels (resume stability), known agent types,
// responses that match the schema.
import { readFileSync, readdirSync } from 'node:fs'
import { dirname, join, basename } from 'node:path'
import { fileURLToPath } from 'node:url'
import assert from 'node:assert/strict'

const HERE = dirname(fileURLToPath(import.meta.url))
export const REPO = join(HERE, '..', '..')
export const ROSTER = new Set(readdirSync(join(REPO, 'bundled-agents')).filter((f) => f.endsWith('.md')).map((f) => basename(f, '.md')))
// Agents with no Edit/Write (read-only by charter). Writers must never be one of these.
export const READ_ONLY = new Set(['bhishma', 'duryodhana', 'yudhishthira', 'drona', 'dhrishtadyumna', 'draupadi'])

// Balanced-brace scan that skips string literals, so braces inside strings don't confuse it.
function literalEnd(src, open) {
  let depth = 0, q = null
  for (let i = open; i < src.length; i++) {
    const ch = src[i]
    if (q) { if (ch === '\\') i++; else if (ch === q) q = null; continue }
    if (ch === "'" || ch === '"' || ch === '`') q = ch
    else if (ch === '{') depth++
    else if (ch === '}') { depth--; if (depth === 0) return i }
  }
  throw new Error('unbalanced meta literal')
}

export function validate(schema, value, path = '$') {
  const errs = []
  const t = schema.type
  if (t === 'object') {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return [`${path}: expected object`]
    for (const k of schema.required || []) if (!(k in value)) errs.push(`${path}.${k}: missing`)
    for (const [k, v] of Object.entries(value)) {
      if (schema.properties && k in schema.properties) errs.push(...validate(schema.properties[k], v, `${path}.${k}`))
      else if (schema.additionalProperties === false) errs.push(`${path}.${k}: not allowed`)
    }
  } else if (t === 'array') {
    if (!Array.isArray(value)) return [`${path}: expected array`]
    value.forEach((v, i) => errs.push(...validate(schema.items || {}, v, `${path}[${i}]`)))
  } else if (t === 'string') {
    if (typeof value !== 'string') return [`${path}: expected string`]
  } else if (t === 'boolean') {
    if (typeof value !== 'boolean') return [`${path}: expected boolean`]
  } else if (t === 'integer') {
    if (!Number.isInteger(value)) return [`${path}: expected integer`]
  } else if (t === 'number') {
    if (typeof value !== 'number') return [`${path}: expected number`]
  }
  if (schema.enum && !schema.enum.includes(value)) errs.push(`${path}: ${JSON.stringify(value)} not in enum`)
  return errs
}

function checkSchemaShape(s, path = '$') {
  assert.ok(s && typeof s === 'object', `${path}: schema missing`)
  const allowed = new Set(['type', 'properties', 'required', 'items', 'enum', 'additionalProperties'])
  for (const k of Object.keys(s)) assert.ok(allowed.has(k), `${path}: non-portable schema keyword ${k}`)
  if (s.type === 'object') {
    assert.ok(s.properties, `${path}: object without properties`)
    for (const k of s.required || []) assert.ok(k in s.properties, `${path}: required ${k} not in properties`)
    for (const [k, v] of Object.entries(s.properties)) checkSchemaShape(v, `${path}.${k}`)
  }
  if (s.type === 'array') checkSchemaShape(s.items, `${path}[]`)
}

export function loadWorkflow(file) {
  const src = readFileSync(file, 'utf8')
  assert.ok(src.startsWith('export const meta = {'), `${file}: must start with "export const meta = {"`)
  const open = src.indexOf('{')
  const end = literalEnd(src, open)
  const metaSrc = src.slice(open, end + 1)
  // Pure literal: outside of plain string literals, no calls, spreads, templates or bare identifiers as values.
  assert.ok(!metaSrc.includes('`'), 'meta must not use template literals')
  const bare = metaSrc.replace(/'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*"/g, '""')
  assert.ok(!/\.\.\.|\(/.test(bare), 'meta must be a pure literal (no spread or calls)')
  for (const m of bare.matchAll(/:\s*([A-Za-z_$][\w$]*)/g)) assert.ok(['true', 'false', 'null'].includes(m[1]), `meta value ${m[1]} is not a literal`)
  const meta = Function('"use strict"; return (' + metaSrc + ')')()
  assert.ok(meta.name && meta.description, 'meta needs name and description')
  assert.equal(meta.name, basename(file, '.js'), 'meta.name must match the file name')
  const body = src.slice(end + 1)
  for (const bad of [/Date\.now\s*\(/, /Math\.random\s*\(/, /new Date\s*\(\s*\)/, /\bimport\s*\(/, /\brequire\s*\(/, /\bprocess\./]) {
    assert.ok(!bad.test(body), `${file}: forbidden construct ${bad}`)
  }
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor
  const fn = new AsyncFunction('agent', 'parallel', 'pipeline', 'phase', 'log', 'args', 'budget', 'workflow', 'Math', body)
  const phaseTitles = new Set((meta.phases || []).map((p) => p.title))

  // shared: state shared with a parent run when this script runs as a nested workflow() step.
  async function run(argsVal, responder, shared) {
    const ctx = shared || { calls: [], logs: [], phases: [], labels: new Set(), harnessErrors: [], depth: 0 }
    const { calls, logs, phases, labels, harnessErrors } = ctx
    const agent = async (prompt, opts = {}) => {
      assert.equal(typeof prompt, 'string', 'prompt must be a string')
      assert.ok(opts.label, 'every agent() call needs a stable label')
      assert.ok(!labels.has(opts.label), `duplicate label ${opts.label} (breaks resume)`)
      labels.add(opts.label)
      assert.ok(ROSTER.has(opts.agentType), `${opts.label}: unknown agentType ${opts.agentType}`)
      assert.notEqual(opts.agentType, 'krishna', 'krishna never runs as a subagent')
      if (opts.phase) assert.ok(phaseTitles.has(opts.phase), `${opts.label}: phase "${opts.phase}" not in meta.phases`)
      if (opts.isolation !== undefined) assert.equal(opts.isolation, 'worktree')
      if (opts.isolation === 'worktree') assert.ok(!READ_ONLY.has(opts.agentType), `${opts.label}: read-only agent given a worktree`)
      if (opts.schema) checkSchemaShape(opts.schema)
      const call = { label: opts.label, agentType: opts.agentType, isolation: opts.isolation, phase: opts.phase, prompt }
      calls.push(call)
      let out = await responder(opts.label, opts, prompt, calls)
      if (typeof out === 'function') out = out()
      if (out instanceof Error) throw out
      // Fixture bugs must fail the test even when the script catches the error.
      if (out === undefined) { harnessErrors.push(`no fixture for ${opts.label}`); throw new Error(`HARNESS: no fixture for ${opts.label}`) }
      if (out !== null && opts.schema) {
        const errs = validate(opts.schema, out)
        if (errs.length) { harnessErrors.push(`${opts.label}: ${errs.slice(0, 3).join('; ')}`); throw new Error(`HARNESS: fixture for ${opts.label} violates schema`) }
      }
      call.out = out
      return out
    }
    const parallel = async (thunks) => Promise.all(thunks.map(async (t) => { try { return await t() } catch (e) { return null } }))
    const pipeline = async (items, ...stages) => Promise.all(items.map(async (item, i) => {
      let v = item
      try { for (const s of stages) v = await s(v, item, i); return v } catch (e) { return null }
    }))
    const phase = (t) => { assert.ok(phaseTitles.has(t), `phase "${t}" not in meta.phases`); phases.push(t) }
    const log = (m) => logs.push(String(m))
    const budget = { total: null, spent: () => 0, remaining: () => Infinity }
    // workflow(name, args): runs a sibling saved workflow inline, one level deep, sharing labels and calls.
    const workflow = async (name, childArgs) => {
      if (ctx.depth >= 1) throw new Error('workflow() nesting is one level only')
      let child
      try { child = loadWorkflow(join(REPO, 'workflows', String(name) + '.js')) } catch (e) { throw new Error(`unknown workflow ${name}`) }
      const r = await child.run(childArgs, responder, Object.assign({}, ctx, { depth: ctx.depth + 1 }))
      return r.result
    }
    const SafeMath = Object.create(Math)
    SafeMath.random = () => { throw new Error('Math.random is not allowed in workflows') }
    const result = await fn(agent, parallel, pipeline, phase, log, argsVal, budget, workflow, SafeMath)
    assert.deepEqual(harnessErrors, [], 'fixture errors')
    // The shared result contract.
    assert.ok(result && typeof result === 'object', 'workflow must return an object')
    assert.ok(['done', 'need_input', 'rejected', 'error'].includes(result.status), `bad status ${result.status}`)
    assert.ok(typeof result.node === 'string' && result.node, 'result.node required')
    assert.ok(result.artifacts && typeof result.artifacts === 'object', 'result.artifacts required')
    if (result.status === 'need_input') assert.ok(typeof result.question === 'string' && result.question.trim(), 'need_input needs a question')
    return { result, calls, logs, phases }
  }
  return { meta, run, src }
}

// Fixture helper: base responses keyed by label prefix (text before the first '.'), overridden per label or prefix.
export function responder(base, over = {}) {
  return (label, opts, prompt, calls) => {
    const keys = [label]
    const parts = label.split('.')
    for (let i = parts.length - 1; i > 0; i--) keys.push(parts.slice(0, i).join('.'))
    for (const k of keys) if (k in over) { const o = over[k]; return typeof o === 'function' ? o(label, opts, prompt, calls) : o }
    for (const k of keys) if (k in base) { const b = base[k]; return typeof b === 'function' ? b(label, opts, prompt, calls) : b }
    return undefined
  }
}

// Protected-action invariants that hold for every Vajra workflow, checked on every scenario.
export const GUARD_TEXT = 'Never push, open a PR, deploy'
export function assertSafe(calls) {
  for (const c of calls) {
    const own = c.prompt.replace(/<artifact [^>]*>[\s\S]*?<\/artifact>/g, '')   // only the script's own words
    if (c.isolation === 'worktree') assert.ok(c.prompt.includes(GUARD_TEXT), `${c.label}: writer prompt lacks the protected-action guard`)
    assert.ok(!/\bgit push\b/i.test(own.replace(/Never push[^\n]*|Do not run any deploy, push[^\n]*/g, '')), `${c.label}: prompt asks for git push`)
  }
}

export async function scenario(name, fn) {
  try { await fn(); console.log(`  ok  ${name}`) } catch (e) { console.log(`  FAIL ${name}\n       ${e.stack.split('\n').slice(0, 3).join('\n       ')}`); process.exitCode = 1 }
}
