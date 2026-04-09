---
name: performance-profiler
description: Performance profiling, bottleneck detection, optimization. Activate for slow, performance, latency, profiling, bottleneck, P99, optimization tasks.
user-invocable: true
---

# Performance Profiler

## When to Activate
- User mentions: slow, performance, latency, profiling, bottleneck, P99, optimization, throughput, memory leak
- System or endpoint is slower than expected
- Pre-launch performance validation
- Resource usage is unexpectedly high

## Golden Rule
> **Measure first. Optimize second. Never guess.**

## Triage Process

### Step 1: Classify the Problem
| Symptom | Likely Category | First Tool |
|---------|----------------|-----------|
| High CPU | Compute-bound | `py-spy` / `perf` / `cProfile` |
| High memory / growing | Memory leak | `tracemalloc` / `memray` |
| Slow responses, low CPU | I/O-bound | `strace` / async profiler |
| Intermittent slowness | Contention / GC | Lock profiler / GC logs |
| Slow queries | Database | `EXPLAIN ANALYZE` |
| High latency, low load | Network / DNS | `tcpdump` / `curl -w` |

### Step 2: Profile (Don't Guess)

**Python:**
```python
# CPU profiling
import cProfile
cProfile.run('main()', 'output.prof')
# Visualize: snakeviz output.prof

# Memory profiling
import tracemalloc
tracemalloc.start()
# ... run code ...
snapshot = tracemalloc.take_snapshot()
top = snapshot.statistics('lineno')[:10]

# Line-level timing
# pip install line_profiler
@profile
def slow_function():
    ...
# kernprof -l -v script.py
```

**Node.js/TypeScript:**
```bash
# CPU profiling
node --prof app.js
node --prof-process isolate-*.log > profile.txt

# Memory heap snapshot
node --inspect app.js  # Chrome DevTools → Memory tab

# Flame graph
npx 0x app.js
```

**FFmpeg / Media Pipeline:**
```bash
# Benchmark encode settings
ffmpeg -benchmark -i input.mp4 ... output.mp4 2>&1 | grep "bench"

# Profile filter chain
ffmpeg -filter_complex_threads 1 ...  # isolate filter bottleneck
```

### Step 3: Identify the Bottleneck
Apply the **80/20 rule**: 80% of time is spent in 20% of code.

**Before/After Template:**
```
## Performance Analysis: [Component]

### Baseline
- Metric: [what we measured]
- Value: [current number]
- Target: [where we need to be]
- Method: [how we measured]

### Bottleneck
- Location: [file:line or component]
- Category: [CPU/Memory/IO/Network/DB]
- Root cause: [why it's slow]
- Evidence: [profiler output / flame graph]

### Optimization
- Change: [what we did]
- Result: [new number]
- Improvement: [X% faster / Y MB less]
- Trade-off: [what we gave up, if anything]
```

### Step 4: Common Optimization Patterns

| Problem | Pattern | Example |
|---------|---------|---------|
| N+1 queries | Batch/eager load | `SELECT ... WHERE id IN (...)` |
| Repeated computation | Memoize/cache | `@functools.lru_cache` |
| Large JSON serialization | Streaming / pagination | Generator + chunked response |
| Blocking I/O | Async / parallel | `asyncio.gather()` / `ThreadPoolExecutor` |
| String concatenation in loop | StringBuilder / join | `''.join(parts)` |
| Full table scan | Add index | `CREATE INDEX CONCURRENTLY` |
| Large file processing | Streaming | Process in chunks, not memory |
| Slow cold start | Lazy loading | Import on first use |

### Step 5: Validate
- Re-run the exact same profiling from Step 2
- Compare before/after numbers
- If improvement < 10%, the optimization was not worth the complexity
- Run regression tests — faster but broken is worse than slow

## Domain-Specific Profiling

### Trading/Quant (QuantMind)
- Latency: measure order-to-fill, signal-to-execution
- Throughput: ticks processed per second
- Memory: position/order object lifecycle
- Critical path: signal detection → risk check → order submission
- Target: <100ms end-to-end for real-time strategies

### Video Pipeline (YouTube Automation)
- Stage timing: scraping, LLM calls, TTS, video assembly, upload
- FFmpeg: encode time vs quality settings
- Ollama: inference latency, VRAM usage, batch size
- Pexels/Pixabay: API response time, download parallelism
- Target: full pipeline <15 minutes per video

## Zero-Bias Checks
- Am I optimizing what's measurably slow, or what I THINK is slow?
- Am I micro-optimizing a function that runs once, while ignoring a loop that runs 10K times?
- Am I adding complexity for a 5% improvement that nobody will notice?
- Have I checked if this is an infrastructure problem (disk, network, RAM) before touching code?

## Guardrails
- NEVER optimize without profiling data (no guessing)
- NEVER sacrifice readability for <10% improvement
- ALWAYS keep the baseline measurement for comparison
- ALWAYS run tests after optimization (faster + broken = worthless)
- Simplest fix first: caching > algorithmic change > rewrite
