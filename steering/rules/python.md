# Steering: Python

- id: python
- domain: python
- blocking: false
- file-patterns: ["**/*.py", "!**/*test*.py"]

## Tooling (non-negotiable)
ruff (lint+format), mypy `strict = true` (never weaken), pytest, Pydantic/
dataclasses, Powertools (Lambda) / structlog (services). Configure in
`pyproject.toml` as the single source of truth.

## Type safety
- Type hints on all public functions and class attributes (missing hints break
  DI resolution at runtime). Modern syntax (3.10+): `X | None`, `list[str]`,
  `match/case`. `Protocol` for structural typing, `TypedDict` for structured
  dicts (not `dict[str, Any]`). No `Any` without a comment.

## Data & design
- `@dataclass` (frozen for value objects), Pydantic for external input, `StrEnum`
  for finite string sets. Immutable by default (tuple/frozenset/frozen). No raw
  dicts for structured data; no tuple returns (use a dataclass).
- Single responsibility; ≤50-line functions, ≤4 params; guard clauses (max 3
  nesting); **no mutable default arguments** (`items: list | None = None`);
  imports at top; `__all__` for public API; files ≤500 lines.

## Errors
- Catch specific exceptions — no bare `except:` (catches KeyboardInterrupt/
  SystemExit) or generic `except Exception:` in business code. **Chain with
  `from exc`** to preserve traceback. `logger.exception()` for unexpected errors.
  Never swallow; don't log-and-rethrow. Custom exception types when callers branch.

## Lambda / AWS
- boto3 clients module-level with explicit `region_name`; Powertools decorators
  on every handler; no `print()`; validate env at startup (pydantic-settings);
  no mutable module-level state; `batchItemFailures` for SQS partial failures.

## Security (blocking — see security.md)
Pydantic validation at boundary; no `eval`/`exec`/`pickle.loads`/`yaml.load`/
`shell=True` on untrusted data; `secrets` not `random`; no `AKIA*` literals.

## Anti-patterns (fix on sight)
`print` in Lambda → Logger · bare `except:` → specific · missing `from exc` →
chain · `Any` everywhere → types/Protocol/TypedDict · mutable default arg →
`None` + create inside · `os.path.join` for S3 keys → f-string `/` · boto3 client
in handler → module-level · `yaml.load` → `safe_load` · `pickle.loads(untrusted)`
→ schema-validated JSON · raw dict → dataclass/Pydantic · `time.sleep` retry →
backoff lib · `json.dumps` with Decimals → custom encoder/`default=str` ·
hardcoded region/account → env/config.

## LLM/agent tool functions
Never raise from a tool function — return a structured error result whose
message is a directive that helps the agent self-correct (name the bad input,
describe the fix); strip stack traces / internal names. `Annotated[T, "desc"]`
for MCP params; add `max_length` to LLM-generated Pydantic string fields.
