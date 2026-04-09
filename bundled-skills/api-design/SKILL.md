---
name: api-design
description: REST API design, OpenAPI specs, versioning, pagination, error handling. Activate for API, endpoint, REST, OpenAPI, GraphQL, contract, schema tasks.
user-invocable: true
---

# API Design

## When to Activate
- User mentions: API, endpoint, REST, OpenAPI, GraphQL, contract, schema, versioning, pagination
- Designing a new service or exposing functionality to consumers
- Reviewing or improving existing API interfaces

## Design Process (Phase-Gated)

### Phase 1: Requirements
1. **Consumers**: Who calls this API? (frontend, mobile, third-party, internal service)
2. **Operations**: What CRUD operations are needed? What business operations?
3. **Data model**: What entities and relationships exist?
4. **Scale**: Expected request volume, latency requirements
5. **Auth**: Public, API key, OAuth, JWT?

### Phase 2: Resource Design
**Naming conventions:**
- Nouns, not verbs: `/users` not `/getUsers`
- Plural: `/users` not `/user`
- Nested for ownership: `/users/{id}/orders`
- Max 3 levels deep: `/users/{id}/orders/{id}/items` (stop here)
- kebab-case for multi-word: `/user-profiles`

**HTTP Methods:**
| Method | Purpose | Idempotent? | Response Code |
|--------|---------|-------------|--------------|
| GET | Read | Yes | 200 |
| POST | Create | No | 201 |
| PUT | Full replace | Yes | 200 |
| PATCH | Partial update | No | 200 |
| DELETE | Remove | Yes | 204 |

### Phase 3: Request/Response Design

**Pagination:**
```json
{
  "data": [...],
  "pagination": {
    "cursor": "eyJpZCI6MTAwfQ",
    "has_more": true,
    "total": 1523
  }
}
```
- Use cursor-based pagination (not offset) for large datasets
- Default page size: 20, max: 100
- Include `total` count when feasible

**Error Responses:**
```json
{
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "Email is required",
    "details": [
      {"field": "email", "issue": "required"}
    ]
  }
}
```

**Error Codes:**
| Code | Meaning | When |
|------|---------|------|
| 400 | Bad Request | Malformed input |
| 401 | Unauthorized | Missing/invalid auth |
| 403 | Forbidden | Valid auth, insufficient permissions |
| 404 | Not Found | Resource doesn't exist |
| 409 | Conflict | Duplicate, version conflict |
| 422 | Unprocessable | Valid syntax, invalid semantics |
| 429 | Rate Limited | Too many requests |
| 500 | Server Error | Unexpected failure |

### Phase 4: Versioning & Evolution
- URL versioning: `/v1/users` (simplest, most visible)
- Header versioning: `Accept: application/vnd.api+json;version=1`
- Never break backwards compatibility within a version
- Deprecation: 6-month warning via `Sunset` header + docs

### Phase 5: Security
- HTTPS everywhere (no exceptions)
- Rate limiting: per API key + per IP
- Input validation: whitelist, not blacklist
- Auth: OAuth 2.0 for user-facing, API keys for server-to-server
- CORS: explicit allowed origins (never `*` in production)
- Request IDs: generate and return `X-Request-ID` for tracing

### Phase 6: OpenAPI Spec Output
```yaml
openapi: 3.1.0
info:
  title: [Service Name]
  version: 1.0.0
paths:
  /resource:
    get:
      summary: List resources
      parameters: [...]
      responses:
        '200':
          description: Success
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/ResourceList'
components:
  schemas:
    Resource:
      type: object
      required: [id, name]
      properties:
        id: { type: string, format: uuid }
        name: { type: string, maxLength: 255 }
```

## REST vs GraphQL Decision

| Factor | REST | GraphQL |
|--------|------|---------|
| Multiple consumers with different needs | Worse (over/under-fetching) | Better |
| Simple CRUD | Better | Over-engineered |
| Caching | Easy (HTTP caching) | Hard (POST-based) |
| File uploads | Easy | Complex |
| Real-time | WebSockets | Subscriptions |
| Team size | Any | Needs schema governance |

**Default to REST.** Switch to GraphQL only when clients need flexible queries across multiple entities.

## Guardrails
- NEVER expose internal IDs directly (use UUIDs or slug)
- NEVER return more data than the client needs (principle of least privilege)
- ALWAYS validate all input at the API boundary
- ALWAYS version your API from day one
- Apply Zero Cognitive Bias — don't choose GraphQL because it's trendy
