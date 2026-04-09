---
name: database-design
description: Database schema design, normalization, indexing, migration strategy. Activate for schema, database, migration, indexing, ERD, data model tasks.
user-invocable: true
---

# Database Design Architect

## When to Activate
- User mentions: schema, database, migration, indexing, ERD, data model, normalization, denormalization, table design
- Creating or modifying any database layer
- Designing data storage for a new feature or project

## Process (Phase-Gated)

### Phase 1: Requirements Analysis
Apply 5-Question Research Framework:
1. **Domain**: What entities exist? What are their relationships?
2. **Business Rules**: What constraints govern the data? (uniqueness, nullability, cascading)
3. **Access Patterns**: How will this data be queried? (read-heavy? write-heavy? analytics?)
4. **Scale**: How much data? Growth rate? Retention policy?
5. **Edge Cases**: Soft deletes? Multi-tenancy? Time-series? Audit trails?

### Phase 2: Schema Design
1. **Entity identification** — list all entities with their attributes
2. **Relationship mapping** — 1:1, 1:N, M:N with join tables
3. **Normalization decision** — start at 3NF, document any intentional denormalization with rationale
4. **Type selection** — choose appropriate types (UUID vs serial, JSONB vs relational, TIMESTAMPTZ vs TIMESTAMP)
5. **Constraint definition** — NOT NULL, UNIQUE, CHECK, FK with ON DELETE behavior

**Zero-Bias Check:**
- Am I choosing this schema because it's familiar, or because it fits the access patterns?
- Am I over-normalizing for theoretical purity when the queries need denormalized data?
- Am I under-normalizing because "it's easier" when it'll cause data inconsistency?

### Phase 3: Index Strategy
| Query Pattern | Index Type |
|--------------|-----------|
| Exact match (WHERE x = ?) | B-tree (default) |
| Range scan (WHERE x BETWEEN) | B-tree |
| Full-text search | GIN / tsvector |
| JSON field queries | GIN on JSONB |
| Geospatial | GiST / SP-GiST |
| Composite WHERE | Composite index (most selective column first) |
| Covering queries | INCLUDE columns |

**Rules:**
- Index columns that appear in WHERE, JOIN ON, ORDER BY
- Composite index order matters: most selective first
- Don't index columns with low cardinality (booleans) unless combined
- Monitor index usage — unused indexes slow writes for nothing
- Partial indexes for filtered queries (WHERE status = 'active')

### Phase 4: Migration Plan
1. **Forward migration** — CREATE/ALTER statements
2. **Backward migration** — DROP/revert statements (always reversible)
3. **Data migration** — if existing data needs transformation
4. **Zero-downtime check** — can this migration run without locking the table?
5. **Validation** — post-migration queries to verify data integrity

### Phase 5: Output
```
## Schema Design: [Feature/System Name]

### Entities & Relationships
[ERD in Mermaid or text]

### Table Definitions
[CREATE TABLE statements with comments]

### Index Strategy
[CREATE INDEX statements with rationale]

### Migration Plan
[Ordered migration steps]

### Design Decisions
| Decision | Chosen | Rejected | Rationale |
|----------|--------|----------|-----------|
```

## Database-Specific Patterns

### PostgreSQL
- Use TIMESTAMPTZ not TIMESTAMP
- Use UUID for public-facing IDs, SERIAL for internal
- Use JSONB for semi-structured data, not JSON
- Use ENUMs sparingly (hard to modify in production)
- Partition large tables by date range

### SQLite
- Use WAL mode for concurrent reads
- INTEGER PRIMARY KEY is the rowid (fast)
- No native UUID — store as TEXT or BLOB
- Use STRICT tables (SQLite 3.37+) for type enforcement
- Single-writer — design for minimal write contention

### Time-Series Data (QuantMind)
- Partition by time range (daily/weekly/monthly)
- Use BRIN indexes for sequential timestamps
- Consider TimescaleDB extension for PostgreSQL
- Retention policies: auto-drop old partitions
- Downsampling: store aggregates for historical queries

## Guardrails
- NEVER design schema without understanding access patterns first
- ALWAYS provide backward migration
- ALWAYS document intentional denormalization with rationale
- Apply Zero Cognitive Bias Protocol for technology selection
