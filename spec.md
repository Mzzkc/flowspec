# Flowspec: Static Data Flow Manifests for AI-Readable Codebases

---

## The Problem

Every AI coding tool — Claude Code, Cursor, Copilot, Mozart, Codex — operates on codebases at the text level. They read source files as strings. They understand syntax through tokenization. But they don't have a structural map of how data actually flows through a system.

When a human developer joins a team, they spend weeks building a mental model: "requests come in here, get validated there, hit the database through this layer, results flow back through these transformations." That mental model is what makes them effective. Without it, they're just reading files.

AI tools skip that step. They read files, infer local patterns, and guess at the bigger picture. This works for small changes but fails for anything structural — which is exactly where the highest-value work lives.

**Flowspec produces the mental model as a structured artifact.**

It crawls a project using language server protocols, static analysis, and AST parsing, then outputs a complete data flow manifest: every type, every function, every boundary, every flow path, every dead end, every orphan, every duplication. In a format any AI tool can consume directly.

---

## What the Manifest Contains

### 1. Entity Registry

Every meaningful unit in the codebase, identified and classified.

```yaml
entities:
  - id: "src/api/handlers/search.rs::handle_search"
    kind: function
    visibility: public
    module: "api::handlers::search"
    signature:
      inputs:
        - name: "query"
          type: "SearchQuery"
          source: "deserialized from HTTP request body"
        - name: "db"
          type: "Pool<Postgres>"
          source: "injected via app state"
      output:
        type: "Result<Json<SearchResponse>, ApiError>"
        consumers: ["HTTP response serialization"]
    annotations: ["#[get('/search')]", "async"]

  - id: "src/search/engine.rs::SemanticEngine"
    kind: struct
    visibility: public
    module: "search::engine"
    fields:
      - name: "qdrant_client"
        type: "QdrantClient"
        origin: "constructed in SemanticEngine::new()"
        used_by: ["SemanticEngine::query", "SemanticEngine::index"]
      - name: "embedding_model"
        type: "EmbeddingModel"
        origin: "loaded from config in SemanticEngine::new()"
        used_by: ["SemanticEngine::embed_query", "SemanticEngine::embed_document"]
    implements: ["SearchBackend"]
```

### 2. Flow Paths

Traced routes that data takes through the system, from entry point to final destination.

```yaml
flows:
  - id: "search-request-flow"
    description: "User search query from HTTP to ranked results"
    entry_point: "src/api/handlers/search.rs::handle_search"
    steps:
      - entity: "handle_search"
        action: "deserialize request body"
        input_type: "bytes"
        output_type: "SearchQuery"

      - entity: "SearchQuery::validate"
        action: "validate query constraints"
        input_type: "SearchQuery"
        output_type: "Result<ValidatedQuery, ValidationError>"
        branch_on_error: "return 400 Bad Request"

      - entity: "SemanticEngine::query"
        action: "embed query + vector search"
        input_type: "ValidatedQuery"
        output_type: "Vec<ScoredResult>"
        calls:
          - "SemanticEngine::embed_query"  # query -> embedding vector
          - "QdrantClient::search"          # embedding -> scored points
          - "results_to_scored"             # scored points -> ScoredResult

      - entity: "SearchResponse::from_results"
        action: "format for API response"
        input_type: "Vec<ScoredResult>"
        output_type: "SearchResponse"

    exit_point: "JSON serialization -> HTTP 200"

    # DIAGNOSTIC FLAGS
    issues: []
```

### 3. Boundary Map

Every interface where data crosses a meaningful boundary — module boundaries, crate boundaries, network calls, FFI, serialization/deserialization points.

```yaml
boundaries:
  - id: "api-to-search"
    type: module_boundary
    from: "api::handlers"
    to: "search::engine"
    crossing_points:
      - function: "SearchBackend::query"
        data_in: "ValidatedQuery"
        data_out: "Vec<ScoredResult>"
        contract: "trait SearchBackend"

  - id: "search-to-qdrant"
    type: network_boundary
    from: "search::engine"
    to: "external::qdrant"
    crossing_points:
      - function: "QdrantClient::search"
        data_in: "SearchRequest (serialized to protobuf)"
        data_out: "SearchResponse (deserialized from protobuf)"
        failure_modes: ["connection timeout", "not found", "server error"]
        error_handling: "mapped to search::Error::VectorStore"

  - id: "api-to-client"
    type: network_boundary
    from: "api::handlers"
    to: "external::http_client"
    crossing_points:
      - function: "JSON serialization"
        data_in: "SearchResponse (Rust struct)"
        data_out: "JSON bytes"
        contract: "OpenAPI schema at docs/api-v1.yaml"
```

### 4. Diagnostics

The high-value output — structural issues identified automatically.

```yaml
diagnostics:
  dead_ends:
    - entity: "src/search/legacy.rs::OldSearchEngine"
      issue: "Struct defined and implemented but never instantiated"
      evidence: "0 references to OldSearchEngine::new() outside of tests"
      suggestion: "Remove or migrate remaining functionality to SemanticEngine"
      severity: moderate

    - entity: "src/api/handlers/search.rs::SearchMetrics"
      issue: "Metrics struct constructed and populated but never read"
      evidence: "handle_search() creates SearchMetrics, calls .record(), but no consumer reads the metrics store"
      suggestion: "Wire metrics to /metrics endpoint or remove"
      severity: low

  orphan_consumers:
    - entity: "src/api/middleware/rate_limit.rs::get_user_tier"
      issue: "Reads UserTier from request extensions, but no middleware sets it"
      evidence: "No call to request.extensions_mut().insert(UserTier::...) found in middleware chain"
      suggestion: "Add user tier extraction to auth middleware"
      severity: high

  duplications:
    - entities:
        - "src/search/engine.rs::normalize_query"
        - "src/api/handlers/search.rs::clean_query_string"
      issue: "Both functions perform query normalization with overlapping logic"
      overlap: "lowercasing, whitespace collapsing, special character stripping"
      difference: "clean_query_string also handles URL decoding"
      suggestion: "Consolidate into search::query::normalize with URL decode option"
      severity: moderate

  contract_mismatches:
    - boundary: "api-to-client"
      issue: "SearchResponse struct has field 'score: f64' but OpenAPI schema specifies 'relevance_score: number'"
      evidence: "serde rename attribute missing, field name differs from contract"
      severity: high

  missing_error_paths:
    - boundary: "search-to-qdrant"
      issue: "QdrantClient::search can return Timeout but search::Error has no Timeout variant"
      evidence: "match statement at engine.rs:142 has _ => Error::Internal catch-all"
      suggestion: "Add Error::Timeout variant for proper upstream handling (retry vs. fail)"
      severity: moderate

  circular_dependencies:
    - cycle: ["config::Settings", "search::engine::SemanticEngine", "config::SearchConfig"]
      issue: "SemanticEngine reads from Settings which depends on SearchConfig which references SemanticEngine defaults"
      severity: moderate

  unreachable_code:
    - entity: "src/search/engine.rs::SemanticEngine::reindex_all"
      issue: "Public method with no callers in application code"
      evidence: "Only referenced in commented-out migration script"
      suggestion: "Either wire to admin endpoint or mark as dead code"
      severity: low
```

### 5. Dependency Graph

Module-level and crate-level dependency structure, with direction and weight.

```yaml
dependency_graph:
  modules:
    - from: "api::handlers"
      to: "search::engine"
      weight: 12  # number of cross-references
      direction: "unidirectional"  # good

    - from: "search::engine"
      to: "config"
      weight: 8
      direction: "unidirectional"

    - from: "config"
      to: "search::engine"
      weight: 2
      direction: "REVERSE"  # bad — circular potential
      issue: "config references search defaults, creating coupling"

  layer_violations:
    - rule: "api should not directly access database"
      violations:
        - "src/api/handlers/admin.rs imports sqlx::PgPool directly"
        - "src/api/handlers/export.rs runs raw SQL query"
```

### 6. Type Flow Matrix

Where each significant type is created, transformed, and consumed.

```yaml
type_flows:
  SearchQuery:
    created_at:
      - "HTTP deserialization in handle_search"
    transformed_to:
      - ValidatedQuery: "via SearchQuery::validate()"
    consumed_by:
      - "logging in handle_search (Debug format)"
    lifetime: "request-scoped"

  ValidatedQuery:
    created_at:
      - "SearchQuery::validate() on success"
    transformed_to:
      - "embedding vector via SemanticEngine::embed_query"
    consumed_by:
      - "SemanticEngine::query"
      - "SearchMetrics::record (but metrics are a dead end — see diagnostics)"
    lifetime: "request-scoped"

  ScoredResult:
    created_at:
      - "results_to_scored in SemanticEngine::query"
    transformed_to:
      - SearchResponse: "via SearchResponse::from_results"
    consumed_by:
      - "SearchResponse construction"
      - "result ranking/filtering"
    lifetime: "request-scoped"
```

---

## Architecture

### Multi-Language Support via Language Server Protocol

The core insight: language servers already compute most of this. LSP provides:
- `textDocument/references` — find all references to a symbol
- `textDocument/definition` — find where something is defined
- `textDocument/implementation` — find trait/interface implementations
- `callHierarchy/incomingCalls` and `outgoingCalls` — call graph
- `textDocument/documentSymbol` — all symbols in a file
- `workspace/symbol` — all symbols in a project

Flowspec launches the appropriate language server(s), crawls the workspace, and aggregates LSP responses into the manifest.

```
                    ┌─────────────────┐
                    │   Flowspec CLI   │
                    │                  │
                    │  flowspec analyze│
                    │  flowspec diff   │
                    │  flowspec watch  │
                    └───────┬──────────┘
                            │
                    ┌───────▼──────────┐
                    │  Orchestrator    │
                    │                  │
                    │  - Project detect│
                    │  - LS lifecycle  │
                    │  - Crawl control │
                    │  - Aggregation   │
                    └───────┬──────────┘
                            │
              ┌─────────────┼─────────────┐
              │             │             │
      ┌───────▼───┐  ┌─────▼─────┐  ┌────▼──────┐
      │rust-analyzer│ │  pyright  │  │ typescript │
      │            │  │           │  │ -language- │
      │  (Rust)    │  │ (Python)  │  │  server    │
      └───────┬────┘  └─────┬─────┘  └────┬──────┘
              │             │             │
      ┌───────▼─────────────▼─────────────▼───┐
      │           AST Enrichment Layer         │
      │                                        │
      │  - Pattern detection (beyond LSP)      │
      │  - Serialization boundary detection    │
      │  - Error path analysis                 │
      │  - Convention inference                │
      └───────────────┬────────────────────────┘
                      │
              ┌───────▼──────────┐
              │  Manifest Writer │
              │                  │
              │  - YAML/JSON     │
              │  - Diagnostics   │
              │  - Diff support  │
              └──────────────────┘
```

### What LSP Doesn't Give Us (AST Enrichment)

LSP provides the structural skeleton. Some diagnostics require deeper analysis:

**Serialization boundary detection.** LSP doesn't know that `serde::Serialize` means "this crosses a network boundary." AST analysis detects derive macros, decorator patterns, and serialization calls to identify where data changes form.

**Error path tracing.** LSP knows call graphs but not error propagation. AST analysis traces `Result`/`Option` chains, `try/catch` blocks, and error type mappings to find missing error paths.

**Dead end detection.** LSP can find "zero references" but can't distinguish "intentionally unused (test helper, future API)" from "accidentally orphaned." Heuristic analysis using visibility, documentation, and test coverage helps classify.

**Convention inference.** By analyzing naming patterns, module structure, and common code shapes across the project, Flowspec can infer conventions that aren't explicitly documented — then flag violations.

### Language Support Matrix

| Language | LSP Server | AST Enrichment | Maturity Target |
|---|---|---|---|
| Rust | rust-analyzer | syn/tree-sitter | Full (first-class) |
| Python | pyright / pylsp | ast module / tree-sitter | Full (first-class) |
| TypeScript/JS | tsserver | tree-sitter | Full (first-class) |
| Go | gopls | go/ast | High |
| Java/Kotlin | Eclipse JDT LS | tree-sitter | Medium |
| C/C++ | clangd | libclang | Medium |

Tree-sitter provides a universal fallback for AST parsing across languages. Language-specific enrichment layers add deeper analysis.

---

## CLI Interface

```bash
# Full analysis — produces manifest
flowspec analyze ./my-project --output flowspec-manifest.yaml

# Specific diagnostics only
flowspec diagnose ./my-project --checks dead-ends,orphans,duplications

# Diff between two manifests (detect what changed structurally)
flowspec diff manifest-v1.yaml manifest-v2.yaml

# Watch mode — re-analyze on file changes, incremental
flowspec watch ./my-project --output flowspec-manifest.yaml

# Single flow trace — follow one entry point through the system
flowspec trace ./my-project --from "api::handlers::search::handle_search"

# Validate against architectural rules
flowspec lint ./my-project --rules .flowspec/rules.yaml

# Output format options
flowspec analyze ./my-project --format yaml    # default, most readable
flowspec analyze ./my-project --format json    # for programmatic consumption
flowspec analyze ./my-project --format summary # human-readable report
```

### Configuration

```yaml
# .flowspec/config.yaml
project:
  name: "naurva"
  languages: ["rust"]  # auto-detected if omitted

analysis:
  entry_points:        # where to start flow tracing
    - "src/main.rs::main"
    - "src/api/**::handle_*"
  ignore:
    - "target/"
    - "tests/fixtures/"
    - "benches/"

  # Depth limits for very large projects
  max_call_depth: 20
  max_type_chain: 10

diagnostics:
  enabled:
    - dead_ends
    - orphan_consumers
    - duplications
    - contract_mismatches
    - missing_error_paths
    - circular_dependencies
    - unreachable_code
    - layer_violations

  # Project-specific rules
  layer_rules:
    - name: "api should not access database directly"
      from: "api::**"
      to: "sqlx::*"
      allowed_through: ["repository::*", "db::*"]

    - name: "search should not know about HTTP"
      from: "search::**"
      to: ["actix_web::*", "hyper::*", "http::*"]

  # Suppress known issues
  suppressions:
    - entity: "src/search/legacy.rs::OldSearchEngine"
      diagnostic: "dead_end"
      reason: "Kept for migration rollback until v2.0"
      expires: "2026-06-01"
```

---

## Integration Points

### Mozart AI Compose

Flowspec manifests drop directly into Mozart's specification corpus:

```
.mozart/
├── spec/
│   ├── architecture.yaml     # human-authored design intent
│   └── flow-manifest.yaml    # flowspec-generated structural reality
```

The planner reads both. The gap between "what the architecture says" and "what the code actually does" is the backlog. Mozart scores can reference flow paths, boundary definitions, and diagnostics directly in their prompts and validations.

```yaml
# In a Mozart score
validations:
  - type: command_succeeds
    command: 'flowspec diagnose {workspace} --checks dead-ends,orphans --format json | python3 -c "import json,sys; d=json.load(sys.stdin); high=[i for i in d.get(\"diagnostics\",[]) if i.get(\"severity\")==\"high\"]; sys.exit(1 if len(high)>0 else 0)"'
    description: "No new high-severity flowspec diagnostics"
```

### Claude Code / Other AI Tools

Any AI coding tool can consume the manifest. Drop it in context:

```
<context>
Here is the structural flow manifest for this project:
{contents of flowspec-manifest.yaml}

The diagnostics section shows current issues.
The flow paths show how data moves through the system.
The boundary map shows where data crosses interfaces.
</context>
```

This replaces the "spend 30 minutes reading code to build a mental model" step that AI tools currently do (badly) on every session.

### CI/CD

```yaml
# GitHub Actions example
- name: Flowspec structural check
  run: |
    flowspec diagnose . --checks all --format json > flowspec-report.json
    # Fail if new high-severity diagnostics
    jq '.diagnostics | map(select(.severity == "high")) | length' flowspec-report.json | \
      xargs -I{} test {} -eq 0
```

### IDE Integration (Future)

Flowspec as an LSP server itself — providing diagnostics, flow visualization, and "show me where this data goes" queries directly in the editor. But CLI-first is the right starting point.

---

## Implementation Language

**Rust.** For the same reasons as the rest of your stack:
- Fast enough to analyze large codebases interactively
- Strong type system models the manifest structure well
- tree-sitter bindings are mature (tree-sitter itself is C with Rust bindings)
- LSP client libraries exist (lsp-types for protocol types)
- Single binary distribution, no runtime dependencies

---

## What This Enables

With Flowspec, Mozart's planner doesn't just read source files — it reads a structural map. When decomposing "add semantic search," the planner can see:
- Which modules will be affected (boundary map)
- What data flows need to change (flow paths)
- Where new code needs to integrate (crossing points)
- What existing issues the new work might interact with (diagnostics)
- What tests are needed at each boundary (boundary contracts)

That's the difference between an AI that reads code and an AI that understands a codebase. Flowspec produces the understanding as a structured artifact that any tool can consume.

For solo developers and small teams, this is the "six months of context a new team member needs" — generated automatically, kept current, and available to every AI tool in the workflow.

For Mozart specifically, it closes the gap between "execute this score" and "understand this project well enough to plan scores autonomously." The planner needs Flowspec. Without it, planning is text-level. With it, planning is structural.
