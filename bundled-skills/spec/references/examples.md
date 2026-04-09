# Spec Examples — Reference

## Example 1: Small Feature Spec

```markdown
# Spec: WebSocket Price Feed

**Version:** 1.0.0 | **Date:** 2026-03-16 | **Status:** Draft

## 1. Requirements

### 1.1 Problem Statement
The dashboard currently polls the API every 5 seconds for price updates, causing
stale data and unnecessary server load. Users need real-time price updates (<500ms latency).

### 1.2 User Stories
- As a trader, I want live price updates, so that I see current market data without refreshing.
- As a system operator, I want to reduce API load, so that the server handles more concurrent users.

### 1.3 Acceptance Criteria
GIVEN a user has the dashboard open
WHEN a price changes on the backend
THEN the dashboard reflects the new price within 500ms

GIVEN the WebSocket connection drops
WHEN the client detects disconnection
THEN it automatically reconnects with exponential backoff

### 1.4 Scope Boundaries
- **In scope:** Price feed WebSocket, client reconnection, heartbeat
- **Out of scope:** Order execution over WebSocket, chat, notifications
- **Dependencies:** FastAPI WebSocket support (already available)
- **Assumptions:** Single server deployment (no WS clustering needed yet)

### 1.5 Non-Functional Requirements
- Latency: <500ms end-to-end
- Connections: Support 100 concurrent WebSocket clients
- Memory: <50MB additional per 100 connections

## 2. Design

### 2.1 Research Analysis
| Question | Analysis |
|----------|----------|
| Domain | WebSocket (RFC 6455), persistent bidirectional TCP, frames not HTTP req/res |
| Business Rules | Prices must be authoritative (server-push only), client is read-only |
| Industry Standards | FastAPI WebSocket, starlette.websockets, JSON messages |
| Data | PriceTick(symbol, price, volume, timestamp) — flat, no relations |
| Edge Cases | Connection drop, stale client, backpressure on slow consumers |

### 2.2 ADR-1: Message Format
**Options:** 1) JSON 2) MessagePack 3) Protobuf
**Decision:** JSON — simplest, browser-native, adequate for 100 clients
**Rejected:** MessagePack/Protobuf add complexity without need at current scale

### 2.3 Data Model
PriceTick: { symbol: str, price: float, volume: float, ts: datetime }

### 2.5 Component Architecture
- `api/ws.py` — WebSocket endpoint + connection manager
- `frontend/lib/ws.ts` — Client with auto-reconnect
- `frontend/hooks/use-price-feed.ts` — React hook

## 3. Tasks

### Task 1: WebSocket Connection Manager
**Depends on:** none
**Files:** src/quant_mind/api/ws.py
**Acceptance Test:** pytest tests/test_api/test_ws.py -v
**Complexity:** M

### Task 2: Price Broadcast
**Depends on:** Task 1
**Files:** src/quant_mind/api/ws.py
**Acceptance Test:** Connect 2 clients, publish price, both receive within 500ms
**Complexity:** S

### Task 3: Frontend Client
**Depends on:** Task 1
**Files:** frontend/src/lib/ws.ts, frontend/src/hooks/use-price-feed.ts
**Acceptance Test:** Open dashboard, verify live price updates in browser console
**Complexity:** M
```

## Example 2: Refactor Spec (Abbreviated)

```markdown
# Spec: Extract Fee Model from Trading Engine

**Version:** 1.0.0 | **Date:** 2026-03-16 | **Status:** Draft

## 1. Requirements
### 1.1 Problem Statement
Fee calculations are duplicated across 4 files (lmsr.py, kelly.py, edge_calculator.py,
order_router.py). Changes require updating all 4 locations. DRY violation.

### 1.4 Scope Boundaries
- **In scope:** Extract fee logic into engine/fee_model.py, update all callers
- **Out of scope:** Changing fee calculation logic, adding new fee types
- **Assumptions:** All 4 implementations are functionally identical (verified by diff)

## 2. Design
### 2.2 ADR-1: Extraction Strategy
**Options:** 1) Pure functions 2) FeeModel class 3) Fee protocol/ABC
**Decision:** Pure functions — simplest, no state needed, matches engine/ conventions
**Rejected:** Class adds unnecessary state; ABC is over-engineering for internal code

## 3. Tasks
### Task 1: Create fee_model.py with extracted functions
### Task 2: Update lmsr.py to use fee_model
### Task 3: Update kelly.py to use fee_model
### Task 4: Update edge_calculator.py to use fee_model
### Task 5: Update order_router.py to use fee_model
### Task 6: Run full test suite — zero behavioral changes expected
```

## Example 3: API Spec (Abbreviated)

```markdown
# Spec: FnO Advisory API

## 2. Design
### 2.4 API Contract

POST /api/fno/analyze
  Request:  { symbol: str, expiry: date, strategy: "bull_call"|"bear_put"|"iron_condor" }
  Response: { recommendation: str, greeks: Greeks, risk_reward: float, max_loss: float }
  Errors:   400: invalid symbol, 422: invalid strategy, 503: market data unavailable
  Auth:     required (Bearer token)

GET /api/fno/chain/{symbol}
  Request:  query params: expiry=date, strike_range=int
  Response: { calls: [Option], puts: [Option], spot: float, iv_percentile: float }
  Errors:   404: symbol not found, 503: exchange unavailable
  Auth:     required
```

## Spec Sizing Guide

| Requirement Type | Expected Spec Size | Sections to Emphasize |
|-----------------|-------------------|----------------------|
| Bug fix | 1-2 pages | Problem statement, root cause analysis, test |
| Small feature | 2-4 pages | User stories, acceptance criteria, tasks |
| API endpoint | 3-5 pages | API contract, data model, security |
| Large feature | 5-10 pages | Full spec, all sections |
| Architecture change | 8-15 pages | ADRs, migration plan, rollback |
| Migration | 5-10 pages | Rollback plan, data model, phased tasks |
