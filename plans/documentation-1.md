# SlateDuck Documentation Plan

## Overview

This plan defines the complete documentation site for SlateDuck, built with MkDocs
Material, deployed via GitHub Actions to GitHub Pages, and covering every aspect of the
project from first principles through production operations. The goal is not a reference
dump or a marketing brochure — it is a body of writing that genuinely helps people
understand, deploy, operate, and contribute to SlateDuck. That means explaining the
reasoning behind decisions, acknowledging trade-offs honestly, providing examples that
work, and treating the reader as an intelligent engineer who deserves full information.

Documentation for infrastructure software fails in predictable ways: it presents the
happy path and hides the edges; it lists API parameters without explaining when to use
them; it describes what a system does without explaining why it works that way; it uses
technical terms before defining them and links to other pages that also skip the
definition. Good documentation is harder to write than good code, and the discipline
required is the same — clarity of thought expressed as clarity of prose. Every page in
this site should feel like it was written by someone who deeply understands the system
and genuinely wants the reader to understand it too.

The documentation serves four distinct audiences simultaneously, and each needs to be
served without alienating the others. The curious newcomer who has heard about DuckLake
and wants to know what SlateDuck adds needs a gentle on-ramp that does not assume prior
knowledge of LSM trees or wire protocols. The evaluating architect deciding whether
SlateDuck fits their stack needs honest answers about latency characteristics, failure
modes, and operational complexity — including the cases where a managed PostgreSQL
would be a better choice. The production operator who has already deployed SlateDuck
needs a reliable reference for CLI flags, troubleshooting symptoms, and upgrade paths.
The contributor who wants to fix a bug or add a feature needs to understand the codebase
well enough to navigate it confidently. Each audience has a clear entry point and a
natural path through the material, and the navigation structure is designed to make
those paths obvious without hiding content from audiences who want more depth.

---

## Technology Stack

### MkDocs Material

MkDocs Material is the right choice for this project because it combines a clean,
professional reading experience with a rich feature set that supports exactly the kinds
of content SlateDuck needs: fenced code blocks with syntax highlighting for Rust, SQL,
TOML, bash, and YAML; Mermaid diagram rendering for architecture visuals and sequence
diagrams; tabbed content blocks for multi-cloud deployment guides; admonition boxes for
warnings, tips, and "why this matters" asides; and instant search that works well on
technical reference material. The dark/light mode toggle, sticky navigation, and mobile
responsiveness mean that the site reads well whether someone is at their desk reviewing
the architecture section or on their phone checking a CLI command in the field.

MkDocs itself is a static-site generator with a straightforward build model: Markdown
source files go in, a self-contained HTML site comes out. There is no database, no
server-side rendering, and no runtime dependencies beyond a static file host — which
aligns naturally with SlateDuck's own philosophy of eliminating infrastructure. The
build is reproducible, fast (seconds for a full rebuild), and requires only Python and
a handful of pip packages. GitHub Pages handles the hosting at zero cost.

- **Framework:** [MkDocs](https://www.mkdocs.org/) with [Material for MkDocs](https://squidfundinglab.github.io/mkdocs-material/)
- **Theme features:** dark/light mode toggle, instant navigation, integrated search,
  code copy buttons, content tabs, admonitions, annotations, diagrams
- **Plugins:**
  - `search` — instant full-text search across all pages, with query highlighting in
    results and keyboard navigation
  - `minify` — HTML/CSS/JS minification for fast load times; documentation readers
    should not wait for a page
  - `git-revision-date-localized` — "last updated" and "created" timestamps derived
    from git history, displayed on every page as a freshness indicator
  - `social` — automatic Open Graph social cards for link previews in Slack, Twitter,
    and Discord; when someone links to a documentation page the preview should look
    professional
  - `redirects` — handle URL moves without breaking bookmarks or external links;
    documentation URLs are promises and should be kept
  - `glightbox` — image and diagram lightbox so architecture diagrams can be zoomed
    without leaving the page
- **Extensions:**
  - `pymdownx.superfences` — fenced code blocks with Mermaid diagram rendering; every
    architecture diagram in the site is a Mermaid source block, not a static image,
    so it can be updated with a text edit
  - `pymdownx.tabbed` — tabbed content blocks for multi-provider comparisons (S3 vs.
    GCS vs. Azure configs, Strategy B vs. Strategy C deployment, local vs. cloud
    quickstart); the reader picks the tab relevant to their situation
  - `pymdownx.details` — collapsible sections for dense reference material that should
    be accessible but not visually overwhelming on first read
  - `pymdownx.highlight` and `pymdownx.inlinehilite` — syntax highlighting for Rust,
    SQL, TOML, bash, YAML, and JSON; code that the reader might copy and paste must be
    readable
  - `admonition` — callout boxes styled as warnings (red), tips (green), notes (blue),
    and custom "why this matters" asides that give readers the deeper reasoning without
    interrupting the main flow for those who do not need it
  - `attr_list` and `md_in_html` — custom styling for comparison tables and feature
    grids where standard Markdown tables are too limiting
  - `toc` — auto-generated table of contents with `permalink: true` so individual
    sections can be linked directly

### GitHub Actions CI/CD

The documentation build and deployment are fully automated. A push to `main` that
touches any file under `docs/` or `mkdocs.yml` triggers an automatic build and deploy
to GitHub Pages, typically completing in under two minutes. Pull requests trigger a
build-only job that verifies the documentation compiles cleanly — `mkdocs build
--strict` turns any broken internal link, missing nav entry, or misconfigured
extension into a build failure, which means documentation errors are caught before
merge rather than after deploy.

The deploy job uses the official `actions/upload-pages-artifact` and
`actions/deploy-pages` actions to publish to GitHub Pages. Concurrency is configured
to cancel in-progress deployments on the same branch, which prevents a slow deploy
from blocking a faster subsequent push.

A `requirements-docs.txt` file at the workspace root pins every documentation
dependency — `mkdocs-material`, plugins, and their transitive dependencies — to exact
versions. This ensures that a build on any developer's machine produces the same output
as CI, and that the documentation does not silently break when a plugin releases a new
version.

### Directory Structure

```
docs/
├── index.md                          # Landing page / hero
├── getting-started/
│   ├── index.md                      # Getting started overview
│   ├── what-is-slateduck.md          # Conceptual introduction
│   ├── quickstart.md                 # 5-minute local setup
│   ├── quickstart-cloud.md           # Cloud deployment (S3/GCS/Azure)
│   └── first-lakehouse.md            # End-to-end tutorial: create, query, time-travel
├── concepts/
│   ├── index.md                      # Concepts overview
│   ├── lakehouse-primer.md           # What is a lakehouse? Why does it matter?
│   ├── ducklake.md                   # DuckLake format explained
│   ├── slatedb.md                    # SlateDB as a storage engine
│   ├── catalog-immutability.md       # The immutability principle in depth
│   ├── mvcc.md                       # MVCC and snapshot isolation
│   ├── time-travel.md                # Time travel as a first-class feature
│   ├── reader-scaleout.md            # Horizontal read scale-out
│   ├── writer-fencing.md             # Single-writer model and fencing
│   └── fact-store-vision.md          # The general fact store future
├── architecture/
│   ├── index.md                      # Architecture overview with diagrams
│   ├── system-design.md              # Full system design narrative
│   ├── crate-map.md                  # Workspace crates and their responsibilities
│   ├── key-layout.md                 # Binary key encoding for all 28 tables
│   ├── value-encoding.md             # Protobuf values, SDKV header, versioning
│   ├── sql-dispatcher.md             # Bounded SQL dispatch design
│   ├── pgwire-protocol.md            # PostgreSQL wire protocol implementation
│   ├── transaction-model.md          # How catalog transactions work
│   ├── counter-allocation.md         # ID allocation and counter management
│   └── data-flow.md                  # Read path and write path explained
├── deployment/
│   ├── index.md                      # Deployment overview
│   ├── local-dev.md                  # Local filesystem development setup
│   ├── docker.md                     # Docker / Docker Compose deployment
│   ├── aws-s3.md                     # AWS S3 deployment guide
│   ├── aws-s3-express.md             # S3 Express One Zone for low latency
│   ├── gcs.md                        # Google Cloud Storage deployment
│   ├── azure.md                      # Azure Blob Storage deployment
│   ├── minio.md                      # MinIO self-hosted object storage
│   ├── kubernetes.md                 # Kubernetes sidecar pattern
│   ├── lambda.md                     # AWS Lambda / serverless deployment
│   ├── credential-isolation.md       # IAM separation: catalog vs. data plane
│   └── tls-and-auth.md              # TLS certificates and password authentication
├── operations/
│   ├── index.md                      # Operations overview
│   ├── cli-reference.md              # Full CLI command reference
│   ├── configuration.md              # All configuration options explained
│   ├── monitoring.md                 # Prometheus metrics, dashboards, alerting
│   ├── gc-and-retention.md           # Visibility GC, retain-from, retention policies
│   ├── excision.md                   # Physical deletion: when, why, how
│   ├── checkpoints.md               # SlateDB checkpoints and catalog backups
│   ├── export-import.md              # NDJSON export/import and migration
│   ├── repair.md                     # Catalog verification and repair tooling
│   ├── encryption.md                 # At-rest encryption with block transformers
│   ├── upgrading.md                  # Version upgrades and catalog-format migration
│   └── troubleshooting.md            # Common problems, diagnostics, recovery
├── integration/
│   ├── index.md                      # Integration overview
│   ├── duckdb.md                     # DuckDB connection and usage guide
│   ├── duckdb-compatibility.md       # Version matrix and corpus validation
│   ├── pg-tide-relay.md              # Streaming ingest via pg-tide-relay
│   ├── datafusion.md                 # DataFusion CatalogProvider integration
│   ├── native-extension.md           # Strategy C: embedded DuckDB extension (v0.5+)
│   └── custom-clients.md             # Onboarding new DuckLake-compatible clients
├── design-decisions/
│   ├── index.md                      # Design decisions overview
│   ├── why-slatedb.md                # Why SlateDB over PostgreSQL/SQLite/other KV
│   ├── strategy-b-first.md           # Why PG-wire sidecar before native extension
│   ├── bounded-sql.md                # Why a bounded dispatcher, not a general SQL engine
│   ├── protobuf-encoding.md          # Why Protobuf (not FlatBuffers, not Bincode)
│   ├── immutability-tradeoffs.md     # Benefits and costs of never deleting
│   ├── single-writer.md              # Single-writer constraints and workarounds
│   ├── key-design-rationale.md       # Why each key is shaped the way it is
│   └── what-slateduck-is-not.md      # Explicit non-goals and anti-patterns
├── performance/
│   ├── index.md                      # Performance overview
│   ├── latency-model.md              # Expected latency by operation and backend
│   ├── benchmarks.md                 # Published benchmark results
│   ├── tuning.md                     # SlateDB tuning knobs and their effects
│   ├── when-to-use.md                # Workload fit: when SlateDuck is the right choice
│   └── vs-alternatives.md            # Honest comparison with alternatives
├── internals/
│   ├── index.md                      # Internals overview
│   ├── tag-allocation.md             # Full tag byte table and allocation policy
│   ├── mvcc-filter.md                # MVCC visibility filter implementation
│   ├── inlined-data.md               # 0xFD dynamic inlined rows
│   ├── schema-version.md             # Schema version tracking and increment rules
│   ├── type-aware-stats.md           # Column statistics and type-aware pruning
│   ├── sqlstate-mapping.md           # Error codes and their PostgreSQL mapping
│   ├── wire-corpus.md                # How wire corpus capture and replay works
│   └── crash-safety.md               # Crash injection points and recovery guarantees
├── contributing/
│   ├── index.md                      # Contributing overview
│   ├── development-setup.md          # Full dev environment setup
│   ├── testing.md                    # Test pyramid: property, unit, golden, crash
│   ├── code-style.md                 # Style guide, naming conventions, lint rules
│   ├── architecture-guide.md         # Where code lives and how to navigate it
│   └── release-process.md            # How releases are cut and published
├── reference/
│   ├── index.md                      # Reference overview
│   ├── catalog-tables.md             # All 28 DuckLake catalog tables documented
│   ├── sql-supported.md              # Every SQL shape the dispatcher accepts
│   ├── error-codes.md                # SQLSTATE codes, causes, and fixes
│   ├── metrics.md                    # All exported Prometheus metrics
│   ├── environment-vars.md           # Environment variables reference
│   └── glossary.md                   # Terminology glossary
├── roadmap/
│   ├── index.md                      # Project roadmap and release timeline
│   └── changelog.md                  # Release-by-release changelog
├── assets/
│   ├── images/                       # Architecture diagrams, screenshots
│   ├── stylesheets/
│   │   └── extra.css                 # Custom theme overrides
│   └── javascripts/
│       └── extra.js                  # Custom interactions (if needed)
└── overrides/
    └── main.html                     # Theme template overrides (hero, footer)
```

---

## Content Plan by Section

### 1. Landing Page (`index.md`)

The landing page is the most important page on the site because it is the first thing
most people see and the thing that determines whether they read further. It must
communicate what SlateDuck is, why it matters, and who it is for — in roughly that
order — without burying the reader in technical detail before they are oriented. The
opening sentence or two should be a direct, concrete pitch: "Your entire DuckLake
catalog lives in the same S3 bucket as your Parquet data. No database server required."
Below that, three concise value-proposition columns — truly serverless, immutable
history, horizontal read scale-out — each with two or three sentences explaining what
the property means in practice, not just naming it. A prominent "Get Started in 5
Minutes" button links directly to the local quickstart.

Below the fold, the landing page should show a simplified architecture diagram (a
Mermaid block rendering an ASCII-art style box diagram of DuckDB → sidecar → SlateDB →
S3) to give visual learners an immediate mental model. A three-column comparison table
contrasting SlateDuck with PostgreSQL-backed and SQLite-backed DuckLake across the
dimensions that evaluators actually care about — infrastructure required, time-travel
cost, horizontal read scale, object-store backends, operational complexity — communicates
at a glance where SlateDuck fits. The table must include the cases where the alternatives
win, not just the cases where SlateDuck wins. Evaluators who see a one-sided comparison
table immediately distrust the rest of the documentation.

The landing page closes with navigation cards for each major section: Getting Started,
Concepts, Deployment, Operations, and Design Decisions. Each card has a one-line
description of what the section contains and a direct link in. The tone throughout is
confident, direct, and technically precise — this is not marketing copy, and readers
will notice the difference.

### 2. Getting Started

#### What is SlateDuck? (`getting-started/what-is-slateduck.md`)

This page is a narrative explanation pitched at an engineer who knows SQL and has heard
of data lakes but may not know what a catalog is, why it matters, or what DuckLake and
SlateDB are. It should open by describing the problem: modern data teams keep their
analytical data in object storage as Parquet files, but to query that data sensibly you
need a catalog — a database that tracks which files belong to which tables, what the
schema is, and what the data looked like at any point in time. Every existing lakehouse
solution delegates this catalog to an external database (PostgreSQL, Hive Metastore,
AWS Glue, Nessie), which means you are running and paying for a database server even
though your actual data is already in a bucket. SlateDuck's premise is that the catalog
itself can live in the same bucket, as key-value entries managed by SlateDB — an
LSM-tree embedded storage engine that runs entirely on top of object storage.

The page should explain DuckLake briefly and clearly: it is a lakehouse format from the
DuckDB team that defines a catalog as a set of 28 SQL tables, versioned with snapshot
IDs, queryable over the PostgreSQL wire protocol. It is simpler and more transparent
than Iceberg or Delta Lake because the catalog is just a database, not a forest of JSON
and Avro manifest files. It explains SlateDB briefly: an embedded key-value store written
in Rust that durably stores all state in object storage, provides atomic multi-key
writes and transactional isolation, and enforces a single-writer constraint for
correctness. SlateDuck is the layer that maps DuckLake's 28-table relational catalog
onto SlateDB's key-value model, exposing the result over the PostgreSQL wire protocol
so DuckDB can connect without any client-side changes.

The page should close by explaining the two things SlateDuck adds beyond DuckLake's
catalog model: a binding commitment that committed catalog facts are never physically
deleted by normal operation (giving you infinite time travel and horizontal read scale
by construction), and a path toward a general fact store where any schema can be hosted
on the same immutable substrate. Both are introduced briefly here and linked to their
full treatments in the Concepts section.

#### Quickstart — Local (`getting-started/quickstart.md`)

This page delivers a working lakehouse in five minutes on any machine with a Rust
toolchain, no cloud credentials needed. It is the page that converts a curious reader
into someone who has actually run the software, and it must be perfect: every command
should work as written, the expected output should be shown after each step so the
reader knows they are on track, and there must be no unexplained preconditions. It
begins with `cargo build --release`, shows the binary appearing in `target/release/`,
then walks through `slateduck serve --catalog ./my-catalog --bind 0.0.0.0:5432`, shows
the startup log lines, opens a DuckDB session with the `ATTACH` command, creates a
schema and table, inserts a few rows, runs a `SELECT`, and then demonstrates a
time-travel query using a historical snapshot ID. The page ends by pointing to the
cloud quickstart for readers who want to move beyond localhost.

#### Quickstart — Cloud (`getting-started/quickstart-cloud.md`)

This page uses MkDocs Material's tabbed content blocks to present three parallel tracks:
AWS S3, Google Cloud Storage, and Azure Blob Storage. Each tab is self-contained and
covers bucket creation, the minimum IAM permissions needed (with the actual policy JSON,
not a vague description), environment variable setup for the object-store credentials,
starting the sidecar pointed at the cloud bucket, and verifying the connection from
DuckDB. Callout boxes in each tab highlight the common pitfalls for that provider:
on AWS, the importance of path-style vs. virtual-hosted-style addressing and the
`AWS_ALLOW_HTTP` flag for testing against MinIO; on GCS, the difference between
Application Default Credentials and service account JSON; on Azure, the distinction
between connection strings and managed identity. The page links to the full deployment
guide for each provider when readers need deeper configuration.

#### Your First Lakehouse (`getting-started/first-lakehouse.md`)

This is the golden-path tutorial — the end-to-end story that a new user should walk
through to understand what SlateDuck actually does. Unlike the quickstart, which moves
as fast as possible, this tutorial moves at a deliberate pace and explains what is
happening at each step. It creates a realistic scenario: a small analytics warehouse
for tracking product events. The tutorial creates a schema, creates a table with a
meaningful column structure, inserts data across several transactions, queries it, runs
a schema migration (adding a column), inserts more data after the migration, and then
demonstrates that time travel back to before the migration gives the old schema and the
old data — not just the current state. Along the way, each SQL statement is followed by
a paragraph explaining what happened in the catalog: which tables were written, what
snapshot ID was assigned, why the counter increment and the row insert happened
atomically. The tutorial ends by pointing forward: here is the Architecture section if
you want to understand how this works at the code level; here is the Concepts section
if you want to understand the principles that drove the design; here is the Operations
section when you are ready to run this in production.

### 3. Concepts

The Concepts section is the philosophical and technical backbone of the documentation.
These pages answer the "why" questions that the Getting Started section deliberately
defers. They are written as flowing technical essays — several paragraphs per idea,
concrete examples woven in rather than appended as afterthoughts, and an honest
treatment of trade-offs that does not pretend every design choice was costless. Readers
who skip the Concepts section and go straight to Deployment will be able to operate
SlateDuck, but they will not understand why certain things work the way they do, why
certain configurations matter, or why certain errors mean what they mean.

#### Lakehouse Primer (`concepts/lakehouse-primer.md`)

This page makes the documentation self-contained for readers who are new to the
lakehouse concept. It explains data lakes (object storage full of Parquet files), the
role of a catalog (tracking schema, table-to-file mappings, and version history), why
combining the two is called a "lakehouse," and why the catalog is the harder part — not
because storing metadata is hard, but because doing so correctly under concurrent writes,
with snapshot isolation, with crash safety, and with support for time travel requires
serious database engineering. The page briefly surveys the landscape: Apache Iceberg and
Delta Lake encode their catalogs as files scattered through the data lake (flexible but
slow and operationally complex); Hive Metastore and AWS Glue use dedicated database
services (reliable but requiring infrastructure); DuckLake takes the simpler approach
of delegating to an ordinary SQL database with a well-defined schema. SlateDuck then
asks: if DuckLake needs a SQL database, can that database itself be a file in the bucket?
This page sets up that question; the rest of the Concepts section answers it.

#### DuckLake Format (`concepts/ducklake.md`)

DuckLake is the format SlateDuck implements, and this page explains it in full from
SlateDuck's perspective. It covers the 28 catalog tables (their names, what they store,
and how they relate to each other), the MVCC model based on `begin_snapshot` and
`end_snapshot` columns, how the `ducklake` DuckDB extension interacts with the catalog
over the PostgreSQL wire protocol, and what the spec says about which operations must
be atomic. The page is frank about what SlateDuck implements versus what it delegates:
SlateDuck owns the catalog plane (the 28 tables in SlateDB), and DuckDB owns the data
plane (writing and reading Parquet files directly). The separation of concerns is a
design strength that the page should explain clearly — it means SlateDuck never needs
to understand Parquet, and DuckDB never needs to understand SlateDB.

The page also explains the bounded query set: DuckLake's spec defines a finite set of
SQL operations against the catalog tables — point SELECTs, range SELECTs filtered by
snapshot ID, INSERTs, and targeted UPDATEs to set `end_snapshot`. This set is small
and well-understood, which is why SlateDuck can implement it with a bounded SQL
dispatcher rather than a general SQL engine. Any reader who wonders "can I run arbitrary
SQL against SlateDuck?" needs to find the answer here, with a clear explanation of why
the answer is "no, by design, for good reasons" rather than "not yet."

#### SlateDB Storage Engine (`concepts/slatedb.md`)

SlateDB is not a household name yet, and readers need a solid explanation of what it
is and why it was chosen. This page explains LSM trees accessibly: they batch writes
into a write-ahead log and periodic sorted files called SSTs, then compact them in the
background. All durable state lives in the object store — WAL segments and SST files
are just objects in S3 — which means SlateDB has no local disk requirement beyond
ephemeral caching and can be embedded in stateless Lambda functions or containers
without persistent volumes. The key API guarantees are explained: atomic `WriteBatch`
and transactional `DbTransaction` for multi-key writes that are all-or-nothing across
crashes; `DbReader` and `DbSnapshot` for consistent, non-torn reads; single-writer
enforcement and writer fencing to prevent concurrent corruption.

The page should also be honest about what SlateDB does not provide: there is no built-in
SQL, no multi-writer support (one writer per database instance), no multi-region
replication, and no built-in encryption (SlateDuck uses SlateDB's block transformer
API for that). These constraints shaped SlateDuck's design in concrete ways — the
single-writer model is why SlateDuck serializes all writes through a single actor; the
absence of SQL is why SlateDuck implements its own key layout and value encoding. A
reader who understands SlateDB's constraints will understand SlateDuck's design
choices without needing them restated in every section.

#### Catalog Immutability (`concepts/catalog-immutability.md`)

This is one of the most important pages in the documentation because catalog-data
immutability is the most distinctive architectural decision in SlateDuck. The page
should explain the commitment clearly: every catalog fact committed at a given
`dl_snapshot_id` is readable at that snapshot ID forever, and can only be physically
removed via the explicit, audited `slateduck excise` command. Normal GC only advances
a query-visibility floor (`retain-from`); it does not delete bytes. This is not a
default that operators can change — it is the architectural premise from which
everything else derives.

The page must then explain why this is not just an ideological position but a
load-bearing engineering choice. Because catalog-data keys are stable once written,
any reader anywhere can serve a query at any historical snapshot without coordinating
with the writer or with other readers. The catalog is a content-addressable log, and
replicas are pure caches. This is the direct cause of the horizontal read scale-out
property. The page should walk through a concrete scenario: a writer is actively
adding files to snapshot 101 while a reader is serving a planning query at snapshot
99. Because snapshot-99 rows are distinct keys that were written at snapshot 99 and
never modified (the `end_snapshot` update that retired them at snapshot 100 is a
value change, not a key deletion), the reader's prefix scan sees exactly the rows it
needs without any coordination mechanism.

The page must also be honest about the costs. Infinite physical retention consumes
object storage. A catalog that has been running for years accumulates SST files. The
`retain-from` mechanism limits how far back queries can see, and the excision command
physically frees storage — but both are opt-in operations that require operator action.
This is the right trade-off for the target workload (immutability as the default,
deletion as the exception), but operators need to understand it before they deploy.

#### MVCC and Snapshot Isolation (`concepts/mvcc.md`)

DuckLake's versioning model uses `begin_snapshot` and `end_snapshot` columns in catalog
rows to express the snapshot interval during which a given version of a row is visible.
This page explains how that model maps to SlateDB's key-value layout: each versioned
row gets a key that includes `begin_snapshot` as a suffix, so different versions of the
same logical entity (e.g., a table definition before and after an `ALTER TABLE`) occupy
distinct keys rather than overwriting each other. The MVCC filter applied at read time
is simply `begin_snapshot ≤ target_snapshot_id AND (end_snapshot IS NULL OR
target_snapshot_id < end_snapshot)` — two integer comparisons, no row locks, no
multi-version overhead in the storage engine.

The page should explain the distinction between DuckLake's `dl_snapshot_id` (the
application-level catalog version) and SlateDB's internal read views (`kv_snapshot`,
`kv_read_view`). These are different things at different layers of the stack, and
confusing them leads to misdiagnosis when something goes wrong. It should also explain
the `schema_version` concept: a counter that increments with every schema-mutating
operation (creating or dropping tables, altering columns) but not with data-only
operations, used by DuckDB to invalidate its query plan cache efficiently.

#### Time Travel (`concepts/time-travel.md`)

This page frames time travel not as a feature layered on top of the system but as the
natural consequence of the storage model. If every committed fact is preserved at its
original `dl_snapshot_id`, then reading the state of the catalog at any historical
point is just an MVCC query at that snapshot ID — there is no special "time travel
mode" and no additional storage overhead beyond what the immutability guarantee already
requires. The page should walk through concrete DuckDB SQL examples showing how to
query at a specific snapshot, how to find the snapshot IDs associated with a series of
schema changes, and how to use time travel to reproduce the exact state of a table that
was used to generate a quarterly report six months ago.

The page must also explain the interaction between time travel and retention policies.
An operator who has configured `--retention-days 30` can advance `retain-from` to the
snapshot corresponding to thirty days ago, after which queries at older snapshots return
a snapshot-out-of-retention-window error. This does not delete data — it only gates
query visibility. Readers who need historical access beyond the retention window can
request excision be deferred by pinning a snapshot. These operational details matter
because they define the contract between the system and its users.

#### Reader Scale-Out (`concepts/reader-scaleout.md`)

This page explains why immutability enables unbounded horizontal read scale-out and
what that means concretely. Because catalog-data keys are stable once written (the
only permitted mutation to a version row is a terminal `end_snapshot` update, which
cannot change a reader's view at the row's own `begin_snapshot`), multiple readers
can serve queries from the same object storage prefix without any coordination with
the writer and without any coordination with each other. A reader is just a process
that opens a SlateDB `DbReader` against a current checkpoint or manifest, applies the
MVCC filter, and returns results. That process can be a long-lived sidecar, a Lambda
function, a container in a Kubernetes job, or an edge worker — the storage model does
not care.

The page should present a realistic scale scenario: a writer sidecar handles all
INSERT/UPDATE catalog operations, while ten reader sidecars serve SELECT queries from
DuckDB clients. The readers never need to talk to the writer; they open periodic
snapshots against the shared S3 prefix and serve reads from there. Adding more readers
does not require any writer-side changes, no leader election, and no replication
protocol. This is fundamentally different from a read-replica model in PostgreSQL,
where each replica must receive and apply a WAL stream from the primary, and the
primary's write throughput is bounded by how fast replicas can keep up.

#### Writer Fencing (`concepts/writer-fencing.md`)

The single-writer constraint is one of SlateDB's core guarantees, and this page
explains what happens when it is violated: a second process tries to write to the same
catalog, SlateDB's fencing mechanism detects the conflict, and the stale writer
receives an error. SlateDuck maps this to `SQLSTATE 57P04` (connection failure,
reconnect), which DuckDB interprets correctly. The page should walk through the takeover
protocol step by step: the new writer opens the catalog, calls `flush()` to establish
a durable reader-visible baseline, publishes its endpoint under the `0xFF` system key,
and then begins accepting client connections. Any in-flight requests to the old writer
that were not yet committed are lost — but because SlateDB's `DbTransaction` is atomic
across crash, there are no partial writes to clean up.

The page should also discuss the writer restart latency: on S3 Standard, fencing and
takeover completes in tens of seconds; on S3 Express One Zone, in roughly ten seconds.
These numbers come from the phase-0 latency baseline measurements and should be cited
explicitly. Operators designing for high availability need to understand these recovery
times and plan their pod restart policies and client retry configurations accordingly.

#### General Fact Store Vision (`concepts/fact-store-vision.md`)

This page is forward-looking and should be written honestly as such: it describes a
direction the project is committed to exploring, not a feature that already exists. The
core idea is that the storage substrate SlateDuck uses for DuckLake — append-only keys
scoped by a monotonically increasing version identifier, Protobuf values with a versioned
header, counter allocation under a dedicated namespace, `retain-from` advancement, and
audited excision — is not specific to DuckLake. It is a generic fact log over object
storage, and any relational schema can be hosted on it. The v2.x roadmap entry
(`slateduck-factstore` crate) is where this goes, but the page should explain the
reasoning so readers understand that the immutability principle is not just about
DuckLake correctness — it is the foundation of a more general storage architecture.

The page should be transparent about what is speculative. The `slateduck-factstore`
crate does not exist yet. Multi-schema isolation (one SlateDB `Db` per schema), the
generic `assert`/`retract`/`as_of` API, and the Datalog query interface are ideas
under consideration, not committed features. The purpose of this page is to give
readers the conceptual context that makes certain design choices — like the 1-byte
tag space, the separation between `slateduck-core` and `slateduck-catalog`, the strict
naming conventions — feel like deliberate architecture rather than arbitrary decisions.

### 4. Architecture

The Architecture section is for engineers who want to understand how SlateDuck works
at the code level — where the logic lives, what the data looks like on the wire and in
storage, and how the pieces fit together. These pages assume familiarity with the
Concepts section and go deeper into implementation detail, with concrete examples of
byte sequences, AST patterns, and Rust API calls.

#### System Design (`architecture/system-design.md`)

This is the master architecture page — a flowing narrative that describes the full
system from DuckDB's perspective through to object storage. It starts with the
two-plane separation: SlateDuck owns the catalog plane (schema definitions, snapshot
IDs, data-file registrations) while DuckDB owns the data plane (reading and writing
Parquet files). The two planes connect only via `data_path` values in catalog rows —
SlateDuck records where files live but never reads or writes them.

The page should include two Mermaid sequence diagrams: one for the read path (DuckDB
sends a `SELECT` over the PostgreSQL wire; `slateduck-pgwire` receives it; `slateduck-
sql` parses and classifies the AST; `slateduck-catalog` executes the corresponding
read operation; `slateduck-core` does a SlateDB `scan_prefix` with an MVCC filter;
results flow back up the stack and are encoded as PG wire rows) and one for the write
path (DuckDB sends `BEGIN`; subsequent `INSERT`s and `UPDATE`s accumulate in a
`PendingCatalogTxn`; `COMMIT` triggers a single SlateDB `DbTransaction` committing all
mutations atomically; `flush()` is called to advance reader visibility; the PG wire
`COMMIT` response is sent). Both diagrams should show the crate boundary at each step.

The page should also cover the concurrency model: one SlateDB writer per catalog,
serialized through a single actor; multiple readers per process, each opening a
`DbReader` or `DbSnapshot` against the current checkpoint. The implications for
scalability — one writer is a bottleneck for high-throughput write workloads, readers
scale horizontally — should be stated plainly here, with forward references to the
Performance section for quantitative analysis.

#### Crate Map (`architecture/crate-map.md`)

Each of the six workspace crates has a clearly bounded responsibility, and this page
explains what that responsibility is, what the crate's public API surface looks like,
and what it depends on. The crate dependency graph (rendered as Mermaid) shows the
layering: `slateduck-core` has no workspace dependencies; `slateduck-catalog` depends
on `slateduck-core`; `slateduck-sql` depends on `slateduck-core`; `slateduck-pgwire`
depends on all three; `slateduck-ffi` re-exports `slateduck-catalog` through a C ABI;
`slateduck-sqlite-vfs` is an optional spike with no dependency on the main stack.

For each crate, the page gives the primary module structure, the key public types (e.g.,
`CatalogStore`, `CatalogReader`, `CatalogWriter` in `slateduck-catalog`; `CatalogKey`,
`SnapshotId`, `TableTag` in `slateduck-core`), and a sentence on what belongs in the
crate versus what does not. The purpose is to help a new contributor quickly find where
a given behavior lives and where a new behavior should go.

#### Key Layout (`architecture/key-layout.md`)

This is a complete reference for the binary key encoding across all 28 DuckLake tables
plus the `0xFD` inlined-data, `0xFE` counter, and `0xFF` system namespaces. Each table
gets a row explaining its tag byte, the fields that compose the key (in order), their
types and byte widths, whether `begin_snapshot` is included in the key (and why), and
the dominant access pattern the key layout serves. The page should include concrete
examples: for `ducklake_table` (tag `0x05`), show the actual byte sequence for a
specific (schema_id, table_id, begin_snapshot) triple so readers can see that the key
is indeed big-endian integers concatenated after the tag.

The page should also explain the design principles that constrain every key choice:
the most common spec query for each table should be a single prefix scan or point read,
never a full-table scan; versioned tables include `begin_snapshot` so historical
versions are distinct keys rather than overwritten values; the 1-byte tag namespace
is allocated up front for all 28 tables even if the implementation is deferred,
preventing tag collisions from future additions.

#### Value Encoding (`architecture/value-encoding.md`)

All catalog values are Protobuf messages prefixed with a 5-byte SDKV header:
`encoding_version: u8` followed by the 4-byte magic `b"SDKV"`. The magic verification
catches silent data corruption — if the first four bytes after the version byte do not
match `SDKV`, the decoder refuses to proceed and returns `SQLSTATE XX001` (data
corrupted). The `encoding_version` byte enables forward and backward compatibility:
older readers encountering an unknown version return `SQLSTATE 22P02` (invalid parameter
value) rather than silently misinterpreting the data; newer writers can introduce a new
version while older readers continue to serve requests against rows written in older
versions.

The page should explain the schema evolution story concretely: when a new field is
added to a Protobuf message, older readers that do not know about the field simply ignore
it (Protobuf's wire-level unknown-field preservation ensures the field is not lost if
the message is re-serialized). When a field is removed, existing rows that contain it
continue to decode correctly because Protobuf decoders tolerate unknown field numbers.
The page should also explain what the encoding is not responsible for: the key layout
encodes the row's identity and sort position; the value encoding encodes the row's
non-key fields. The division of responsibilities is strict and intentional.

#### SQL Dispatcher (`architecture/sql-dispatcher.md`)

The `slateduck-sql` crate implements a bounded SQL classifier that recognises exactly
the set of SQL shapes emitted by DuckDB's `ducklake` extension and dispatches each to
the corresponding `CatalogStore` operation. It never executes SQL — it pattern-matches
AST nodes produced by `sqlparser-rs` and extracts bound parameters. The page should
explain why AST-level matching rather than string matching: a query string can be
whitespace-normalized or parameterized in different ways by different client versions,
but the AST structure of a `SELECT max(snapshot_id) FROM ducklake_snapshot` is
unambiguous and stable.

The page should include the complete taxonomy of supported statement shapes, organized
by category: snapshot queries, table/schema/column metadata queries, data-file queries,
stats queries, session control (`SET`, `SHOW`), catalog function calls, write shapes
(INSERT, UPDATE), and generated inlined-table DDL/DML. For each shape, the page should
show a representative SQL example and the Rust dispatch call it maps to. Shapes not in
the taxonomy return `SQLSTATE 0A000` (feature not supported) before any execution
attempt, which is both a security property (no SQL injection surface) and a correctness
property (no accidental partial execution of unsupported queries).

#### PG-Wire Protocol (`architecture/pgwire-protocol.md`)

The PostgreSQL wire protocol is the interface between DuckDB and SlateDuck, and this
page explains the implementation in full. It covers the startup sequence (SSL handshake,
startup message, authentication, `BackendKeyData`, `ReadyForQuery`), the simple query
protocol (a single `Query` message followed by `RowDescription`, zero or more
`DataRow` messages, and `CommandComplete`), and the extended query protocol
(`Parse`/`Bind`/`Describe`/`Execute`/`Sync` sequences that DuckDB uses for all
prepared statements). The page should explain how DuckDB's handshake probes — queries
against `pg_catalog.pg_type`, `pg_catalog.pg_namespace`, `current_schema()`,
`version()` — are handled, what values they return, and why those specific values are
required for DuckDB to proceed past the handshake phase.

The type OID table is reproduced here in full, covering every OID observed in the phase-0
wire corpus: `bool` (16), `int8` (20), `int4` (23), `int2` (21), `float4` (700),
`float8` (701), `text` (25), `varchar` (1043), `timestamp` (1114), `timestamptz` (1184),
`uuid` (2950), `json` (114), `jsonb` (3802). For each type, the page explains how
SlateDuck encodes values in text format and what the binary format code handling looks
like (binary format codes not observed in the corpus return `SQLSTATE 0A000`).

#### Transaction Model (`architecture/transaction-model.md`)

SlateDuck's transaction model bridges two transaction systems: the PostgreSQL `BEGIN` /
`COMMIT` / `ROLLBACK` protocol that DuckDB expects and the SlateDB `DbTransaction` that
provides catalog atomicity. The bridge is the `PendingCatalogTxn` struct in the session
state, which accumulates `INSERT` and `UPDATE` statements between `BEGIN` and `COMMIT`.
On `COMMIT`, the accumulated statements are translated to SlateDB `put` operations and
committed in a single `DbTransaction`, ensuring all-or-nothing semantics across crashes.
`ROLLBACK` or a disconnected client drops the pending batch with no catalog side effects.

The page should explain the 64 MiB batch limit: if a transaction accumulates more than
64 MiB of pending catalog writes before `COMMIT`, the connection is refused with
`SQLSTATE 54001` (program limit exceeded). This limit exists because SlateDB's
`DbTransaction` is an in-memory structure during accumulation, and allowing unbounded
growth would eventually exhaust the sidecar process's memory. The page should also
explain the `flush()` call after every commit: `flush()` is the visibility barrier
that makes a committed write visible to readers opening new `DbReader`s or `DbSnapshot`s.
Without it, a reader that opens immediately after the writer's `DbTransaction::commit`
might not see the new rows.

#### Counter Allocation (`architecture/counter-allocation.md`)

DuckLake requires monotonically increasing IDs for snapshots, catalog objects, files,
and per-table columns. SlateDuck implements these as SlateDB-backed counters under the
`0xFE` namespace. The critical invariant is that the counter increment and the row that
consumes the allocated ID must commit in the same `DbTransaction` — if they are separate
writes and a crash occurs between them, the ID may be permanently lost or the row may
appear with an ID that the counter has not yet advanced past. The page should walk
through the allocation protocol concretely: (1) read the current counter from the
in-memory cache; (2) compute the new value; (3) open a `DbTransaction`; (4) write the
new counter value and the consuming row in the same transaction; (5) commit; (6) on
success, update the in-memory cache. A crash at step 4 leaves both the counter and the
row unchanged; a crash after commit leaves both advanced; there is no intermediate state.

#### Data Flow (`architecture/data-flow.md`)

This page makes the system's behaviour under real workloads concrete by walking through
the full call chain for two representative operations: a `SELECT` and an `INSERT`. For
the SELECT, it follows the query from the moment DuckDB sends the PostgreSQL wire
message, through the `pgwire` crate's message dispatch, through the SQL classifier's
AST analysis, through `CatalogReader::list_data_files`, through SlateDB's
`scan_prefix`, through the MVCC filter, through Protobuf deserialization, through value
encoding into PostgreSQL `DataRow` wire messages, and back to DuckDB. For the INSERT
(which in DuckLake context usually means registering a new Parquet file), it follows
the same path through the write side: accumulation in `PendingCatalogTxn`, counter
allocation, key encoding, transaction commit, `flush()`, and the PG wire `CommandComplete`.

The page should include timing annotations where they are known from the phase-0
baseline: SlateDB `get` on S3 Standard is roughly 20–40 ms; a full prefix scan of a
thousand rows is roughly 100–200 ms; a durable `DbTransaction::commit` is roughly
50–100 ms on S3 Standard. These are not contractual SLA numbers — they are rough
baselines from the phase-0 measurements, with the caveat that real-world performance
depends heavily on object-store request latency, which varies by region, time of day,
and workload contention.

### 5. Deployment

The Deployment section is the most operationally practical section in the documentation.
Every page is self-contained: a reader following any single deployment guide should not
need to cross-reference other guides or search the internet to stand up a working
deployment. Each page includes the full set of prerequisites, step-by-step commands
with expected output, a working `slateduck serve` invocation, a DuckDB connection
snippet that proves the deployment is working, and a troubleshooting callout for the
most common failure modes on that specific platform.

#### Local Development (`deployment/local-dev.md`)

The local development guide covers the case where both the catalog and the DuckDB client
run on the same machine, the object store is the local filesystem, and the goal is fast
iteration on catalog code or a simple evaluation. It walks through building from source,
the directory structure that `slateduck serve --catalog ./my-catalog` creates under the
local path, how to inspect the underlying SlateDB SST files for debugging, and how to
reset to a clean state by removing the catalog directory. It should address the "why
does my query not see the row I just inserted" question, which almost always means the
reader opened before the writer's `flush()` completed.

#### Docker (`deployment/docker.md`)

The Docker guide provides a complete `docker-compose.yml` that stands up three services:
MinIO (the object store), a SlateDuck sidecar connected to MinIO, and a one-shot DuckDB
container that runs a connection test and exits. A reader can bring up the entire stack
with a single `docker compose up`, watch the health checks pass, and then connect from
their host DuckDB with the given `ATTACH` command. The guide explains the networking:
the sidecar needs to reach MinIO's API port (9000), and the DuckDB client needs to reach
the sidecar's PostgreSQL port (5432). It explains the MinIO bucket initialization step
(the sidecar will fail to start if the bucket does not exist), and it shows how to
inspect MinIO's web console (port 9001) to see the SlateDB SST and WAL files being
written as catalog operations execute.

#### AWS S3 (`deployment/aws-s3.md`)

The AWS S3 guide is the most important cloud deployment page because S3 is the primary
production target. It covers IAM policy creation (the catalog-only policy for the
sidecar, the data-only policy for DuckDB) with the exact JSON for each policy. It
explains the S3 bucket configuration (versioning not required, lifecycle rules to clean
up SlateDB's WAL segments after compaction, server-side encryption options), the
`AWS_REGION` and optional `AWS_ENDPOINT_URL` environment variables, and the
`slateduck serve` command with the `s3://` catalog URI. It should include a note on
S3 request costs: a busy SlateDuck deployment will make many `GetObject` and `PutObject`
requests, and operators with high-throughput workloads should understand the billing
implications before using S3 Standard as a production catalog backend.

The S3 Express One Zone variant deserves a paragraph explaining what changes: the bucket
URL scheme is `s3express://`, latency is roughly 3–5× lower for small-object operations
(which catalog reads and writes are), the cost model is different (storage is more
expensive per GB, requests are cheaper per call), and Express One Zone buckets are
single-AZ (relevant for durability decisions). The performance implications are covered
in depth in the Performance section, with a forward link.

#### Credential Isolation (`deployment/credential-isolation.md`)

Credential isolation is one of SlateDuck's architectural security properties and
deserves its own page rather than a footnote in the S3 guide. The principle is that
the sidecar should hold credentials scoped only to the `catalogs/` prefix — it should
not be able to read or write Parquet files under `data/`. DuckDB, conversely, should
hold credentials scoped only to the `data/` prefix — it should not be able to
overwrite or delete catalog SST files. This separation means a compromised sidecar
cannot corrupt the data files, and a compromised DuckDB client cannot corrupt the
catalog. The page provides IAM policy JSON for all three major providers (AWS, GCS,
Azure), explains how to test the isolation (the page walks through attempting a
credential-violating operation and observing the expected `SQLSTATE 42501`), and
addresses the GC/maintenance exception: the `slateduck gc apply` and `slateduck excise`
jobs require both policies because they delete orphaned Parquet files as well as
catalog entries.

#### Kubernetes (`deployment/kubernetes.md`)

The Kubernetes deployment guide covers the sidecar pattern in detail: a SlateDuck
container in the same pod as DuckDB, connected via `localhost`, with an object store as
the catalog backend. It provides a complete pod spec with resource requests and limits,
health check endpoints (`/health` for liveness, `/ready` for readiness, `/metrics` for
Prometheus scraping), environment variable injection for object-store credentials via
Kubernetes Secrets, and a `ConfigMap` for the catalog configuration. The guide discusses
when the sidecar pattern is appropriate (single-writer per pod, tight coupling between
the client and catalog) versus when a shared sidecar service is better (multiple DuckDB
pods reading from the same catalog, where the writer is a separate deployment). It
notes the graceful-shutdown consideration: Kubernetes sends SIGTERM before killing the
pod, and the sidecar must flush pending writes and call `flush()` before exiting to
ensure all committed transactions are visible to the next writer.

### 6. Operations

The Operations section is written for people who have already deployed SlateDuck and
need to keep it running. It assumes the reader has a working deployment and focuses on
day-2 concerns: what knobs to turn, what metrics to watch, how to recover from common
failure modes, and how to safely perform maintenance operations like GC, excision,
backup, and upgrade.

#### CLI Reference (`operations/cli-reference.md`)

The CLI reference is a complete, structured description of every `slateduck` subcommand,
flag, and argument. It is organized by command group (serve, inspect, verify, gc,
excise, checkpoint, export, import, rebuild, repair) with a table of flags for each
command showing name, type, default, and description. Each command section includes one
or two complete invocation examples with typical output. This page is the first place
an operator goes when they need to remember an argument they used six months ago, and
it should be comprehensive enough that they do not need to run `--help` in production.
The page notes which commands are read-only (safe to run anytime), which are
plan-only by default and require `--apply` to mutate state, and which cannot be undone.

#### Monitoring (`operations/monitoring.md`)

Good monitoring documentation goes beyond listing metric names — it explains what the
metrics mean in context, what baselines to expect, and what anomalies indicate. This
page begins with the operational questions an operator should be able to answer from
metrics: Is the catalog healthy? Is the writer making progress? Are reads slow? Are
there unusual error rates? For each question, it identifies the relevant metrics and
explains the expected ranges. For example, `slateduck_snapshots_created_total` should
increase monotonically; a sudden stop indicates the writer has stalled. Object-store
throttle retries (`slateduck_object_store_throttle_retries_total`) are normal in small
quantities but indicate sustained overload if the rate is more than a few per minute.

The page includes a sample Prometheus alerting rule for writer staleness and a sample
Grafana dashboard JSON. It also explains the SlateDB metrics that SlateDuck re-exports:
compaction backlog, WAL segment count, and SST file count are all relevant to
understanding the storage health of the catalog. An operator who understands these
metrics can diagnose most problems — slow reads, high write latency, growing storage
costs — before they become incidents.

#### GC and Retention (`operations/gc-and-retention.md`)

The GC and retention page explains the distinction between visibility GC (safe,
advancing the query-visibility floor, never deleting bytes) and physical excision
(rare, audited, physically removes bytes) in practical operational terms. It walks
through concrete scenarios: an operator who wants 30-day time travel should configure
`--retention-days 30` and schedule `slateduck gc apply` to run daily; an operator who
wants infinite history should configure nothing (the default). The page explains how
`catalog.pin_snapshot(id)` prevents GC from advancing past a snapshot that a long-
running analytical query is still reading against, and what the error looks like when
a query tries to read a snapshot that has been advanced past by GC.

The `slateduck gc plan` command is explained in detail: it shows what the advancement
would do (which snapshots would become query-inaccessible, how much potential storage
space that represents) without making any changes, and its output should be reviewed
before running `slateduck gc apply` in production for the first time.

#### Excision (`operations/excision.md`)

Excision deserves its own page because it is the only way to physically delete catalog
data, and it is designed to be rare, deliberate, and auditable. The page opens by
explaining what excision is and what it is not: it is not routine cleanup (that is GC),
it is not automatic (it never runs without explicit `--apply`), and it is not
undoable (once bytes are deleted from the object store, they are gone). Its primary
use cases are compliance (GDPR right-to-erasure requests require physical deletion of
specific data), bounded-retention deployments (operators who have agreed to keep only
N months of history and need to free storage), and recovery from data poisoning
incidents.

The page walks through the full excision workflow: `slateduck excise plan --before
<snapshot>` to see what would be deleted; reviewing the plan; running `slateduck excise
apply --before <snapshot> --reason "GDPR erasure request #1234" --operator alice` to
execute the deletion with a recorded audit entry; and then running `slateduck verify
catalog` to confirm the catalog is still consistent after excision. It explains that
the audit entry written under `0xFF | "excised"` accumulates indefinitely — the audit
trail of excision events is itself preserved by the immutability guarantee, which means
you can always see what was deleted and when, even after the actual data is gone.

#### Upgrading (`operations/upgrading.md`)

Upgrades require more discussion than a simple "run the new binary" because SlateDuck
uses a `catalog-format-version` stored under `0xFF` to gate binary compatibility. An
older binary encountering a higher `catalog-format-version` refuses to open the catalog
with `SQLSTATE 0A000`. This is a safety mechanism: it prevents a downgraded binary from
silently misinterpreting catalog data written in a newer format. The upgrade page
explains the compatibility matrix: patch versions are backward-compatible; minor version
upgrades may include new catalog rows but not schema changes and are forward-compatible;
major version upgrades may change the `catalog-format-version` and require a migration.
For migrations, the page documents the three-step path: `slateduck export` the current
catalog to NDJSON, reinitialize with the new binary, `slateduck import` the NDJSON. A
round-trip test verifies that all snapshot IDs, file registrations, and MVCC visibility
intervals are equivalent before and after.

#### Troubleshooting (`operations/troubleshooting.md`)

The troubleshooting guide is organised by symptom, not by error code. Each symptom
entry has a short description of what the operator observes (e.g., "DuckDB connects but
all queries return no rows"), a differential diagnosis of the most likely causes
(in this case: `flush()` not completing before the reader opened; the catalog was
initialized with the wrong `--catalog` path; the writer has stalled and readers see a
stale checkpoint), and a step-by-step resolution for each cause. The page should cover
the most common operational issues: writer fencing errors after a restart; stale reads
immediately after a write; slow query planning that turns out to be a large prefix scan;
`verify catalog` failures after a crash; and missing Parquet files that `verify
data-files` reports.

### 7. Integration

#### DuckDB (`integration/duckdb.md`)

DuckDB is the primary client, and this page is the full reference for using DuckDB with
SlateDuck. It covers the `ATTACH` syntax with all options, the `USE` command, how to
reference tables and schemas, how time-travel queries work at the SQL level, what
DuckDB does and does not support through SlateDuck (all DuckLake tutorial operations
work; complex catalog queries that SlateDuck's bounded dispatcher does not support
return `SQLSTATE 0A000`), and the version compatibility matrix. It includes examples
of common operations: creating schemas and tables, inserting data, querying, altering
tables, running time-travel queries, dropping tables, and viewing catalog metadata. The
page should also explain the session configuration: `timezone`, `DateStyle`, and
`client_encoding` are accepted by the sidecar; any other `SET` statements are silently
ignored, which is intentional — DuckDB sends many session-variable settings during
startup that SlateDuck does not need to honor.

#### DuckDB Compatibility (`integration/duckdb-compatibility.md`)

This page maintains the version compatibility matrix and explains the process for
validating new DuckDB versions. The wire corpus is central to compatibility: for each
supported DuckDB version, a corpus fixture under `tests/fixtures/wire-corpus/
duckdb-{version}.jsonl` contains every SQL statement DuckDB emits against a real
DuckLake catalog, and the replay harness verifies that SlateDuck produces
bit-for-bit identical responses. When a new DuckDB version is released, the process is:
(1) capture a new corpus by running the full DuckLake tutorial against a PostgreSQL-
backed DuckLake; (2) classify each new statement shape against the bounded dispatcher
taxonomy; (3) add support for any new shapes in the dispatcher; (4) run the replay test
and verify it passes; (5) update this page with the new version entry and sign-off.
The page explains why minor version bumps require new corpus capture even if the SQL
is unchanged — the PostgreSQL wire protocol handshake can also change between versions,
and the handshake replay test is the first thing that runs.

#### Custom Clients (`integration/custom-clients.md`)

This page documents the process for onboarding any DuckLake-compatible client that is
not DuckDB. The process is the same regardless of client: capture the client's full
SQL corpus by running it against a PostgreSQL-backed DuckLake; classify each statement
against the dispatcher taxonomy (category A: already supported; category B: new but
within the bounded set; category C: outside the bounded set); implement category-B
extensions behind a feature flag; reject category-C statements with `SQLSTATE 0A000`;
add corpus replay tests in CI. The page explains why category-C statements are not
just "not yet implemented" but "will not be implemented without a compelling reason" —
the bounded dispatcher is a deliberate architectural choice that keeps the security
profile tight and the conformance test suite complete.

### 8. Design Decisions

The Design Decisions section is the most important section in the documentation for
an evaluating architect. These pages present the reasoning behind every major choice,
including the cases where the chosen approach has real costs. They are written as
honest engineering assessments, not product marketing.

#### Why SlateDB? (`design-decisions/why-slatedb.md`)

This page compares SlateDB against the realistic alternatives for a catalog-plane
backend: managed PostgreSQL (the existing reference implementation — well-understood,
mature, has SQL, but requires a persistent server), SQLite (lightweight, zero-
infrastructure for local use, but not safe for concurrent cloud access without a custom
VFS), FoundationDB (distributed, multi-writer, excellent reliability guarantees, but
requires running an FDB cluster — not serverless), TiKV (distributed KV with ACID
transactions, but again requires a cluster). SlateDB's unique position is that all
durable state lives in the object store — no server, no persistent disk beyond the
cache, no cluster to operate. Its LSM design, writer fencing, and atomic transaction
API provide exactly the guarantees SlateDuck needs without any infrastructure beyond
a bucket. The page is honest about the costs: single-writer is a real constraint;
SlateDB is younger and has less battle-testing than PostgreSQL; object-store latency
(tens of milliseconds per round trip) is higher than local-disk latency.

#### Immutability Trade-offs (`design-decisions/immutability-tradeoffs.md`)

This is the most philosophically substantive page in the Design Decisions section. It
opens with the argument for immutability: if you never delete committed facts, you get
horizontal read scale-out for free (readers never need to coordinate with writers because
the data they are reading cannot change), time travel as the natural read mode (historical
snapshots are just MVCC queries, not special backup restores), an auditable fact log
(every change to the catalog is preserved forever by default), and a substrate that can
host additional schemas without changing the storage engine. These are not incidental
benefits — they are the direct mechanical consequence of "keys are never deleted outside
excision."

The page then presents the costs clearly. Infinite retention consumes storage. A catalog
that grows continuously will eventually have many SST files, increasing compaction
overhead. The `retain-from` mechanism and bounded-retention configuration exist
precisely to address this, but they require active operator decisions; the default is
growth. The excision command exists for compliance cases but is designed to be rare,
not routine. Operators who are accustomed to PostgreSQL's autovacuum or SQLite's
automatic storage reclamation will find the SlateDuck model more explicit and less
automatic — which is intentional (automatic deletion of committed facts would violate
the immutability contract) but does require a different operational mindset.

#### What SlateDuck Is Not (`design-decisions/what-slateduck-is-not.md`)

This page is explicitly a list of anti-patterns and wrong use cases — not what SlateDuck
wants to avoid being, but what it genuinely is not and what a user should not expect
of it. SlateDuck is not a general SQL engine: the bounded dispatcher supports only the
DuckLake catalog query set, and running arbitrary analytical SQL through the sidecar is
not supported. SlateDuck is not a multi-writer database in v1: one catalog, one writer,
period; the v0.7 partitioning pattern (one SlateDB database per dataset) is the
workaround for workloads that need multiple concurrent writers. SlateDuck is not a
data-plane proxy: DuckDB reads and writes Parquet files directly; SlateDuck only manages
the catalog metadata. SlateDuck is not a replacement for PostgreSQL-backed DuckLake in
all scenarios: if you have low-latency analyst queries on a high-traffic catalog and
already run PostgreSQL, the managed PostgreSQL path is likely simpler and faster. The
page ends with a clear "choose SlateDuck when" and "choose PostgreSQL when" framework
that helps readers make the right decision for their use case.

### 9. Performance

#### Latency Model (`performance/latency-model.md`)

Every DuckLake catalog operation ultimately bottoms out in object-store API calls, and
the latency of those calls is the dominant term in catalog operation latency. This page
builds a quantitative model starting from the phase-0 measurements: single `GetObject`
on S3 Standard is roughly 20–40 ms; on S3 Express One Zone, roughly 5–10 ms; on a
local filesystem, roughly 0.1–1 ms. A `list_data_files` call for a table with 1,000
files requires scanning a prefix that spans multiple SST blocks, resulting in roughly
3–5 `GetObject` calls at SST read time; at S3 Standard latency that is roughly 100–200
ms. `create_snapshot` requires a `DbTransaction::commit` that writes the WAL segment
(one `PutObject`) and then the `flush()` visibility barrier (at least one more); at
S3 Standard that is roughly 50–100 ms. These model predictions are validated against
the `benchmarks/phase-2-baseline.json` data and updated with each benchmark run.

The page is honest about the latency gap with PostgreSQL: a PostgreSQL `SELECT` against
a local-disk-backed catalog on the same LAN as the DuckDB client is roughly 1–5 ms.
SlateDuck's S3-backed catalog is roughly 10–50× slower for individual operations. For
DuckLake's typical usage pattern — catalog reads happen once per query to fetch the file
list, then DuckDB reads from S3 directly — this latency is often acceptable, but for
workloads that issue hundreds of short catalog lookups per second, the gap is real and
operators need to understand it.

#### vs. Alternatives (`performance/vs-alternatives.md`)

The comparison page presents a direct, honest, multi-dimensional comparison between
SlateDuck, PostgreSQL-backed DuckLake, and SQLite-backed DuckLake across the dimensions
that an evaluating engineer cares about: catalog read latency (cold and warm), catalog
write latency (single snapshot creation), infrastructure required to operate, horizontal
read scalability, time-travel cost, object-store backend flexibility, operational
complexity for backup and restore, and correctness guarantees under concurrent writes
and process failures. SlateDuck wins on infrastructure simplicity, horizontal read
scale, and object-store-native durability. It loses on catalog operation latency (object-
store round trips are expensive relative to local-disk or in-memory database operations).
The table is presented without spin: where SlateDuck is slower, the page says so, and
gives the quantitative gap. An evaluator who reads this page should be able to make a
well-informed decision without needing to do their own benchmarking first.

### 10. Internals

#### Crash Safety (`internals/crash-safety.md`)

Crash safety is the most critical correctness property for a storage system, and this
page documents every crash injection point in SlateDuck's test suite along with the
guarantee that holds at each point. The key points are: a crash after the S3 `PutObject`
for a new Parquet file but before the catalog `DbTransaction::commit` leaves the file
as an orphan (not referenced by any snapshot) and the catalog unchanged — the orphaned-
file sweep will clean it up; a crash during `DbTransaction::commit` leaves no partial
write because SlateDB's WAL write is atomic; a crash after `DbTransaction::commit`
but before `flush()` leaves the transaction committed but not yet visible to readers
opening new `DbReader`s — the `flush()` call on writer restart makes it visible.

Each scenario is covered by a crash-injection test using the `fail-parallel` crate to
inject failures at specific points in the code, then verifying that the catalog is
consistent and all pre-crash committed transactions are visible after a restart. The
page explains how to run these tests locally (they require disabling the `fail` points
in the binary, which requires the `fail/failpoints` cargo feature), and why the test
suite is structured the way it is.

#### Wire Corpus (`internals/wire-corpus.md`)

The wire corpus is the source of truth for what SlateDuck's bounded dispatcher must
support, and this page explains how it was captured, what it contains, and how it is
used in testing. The corpus is a sequence of JSONL records, each containing the raw
PostgreSQL wire bytes of a request and the expected response, captured by running DuckDB
against a real PostgreSQL-backed DuckLake while a packet capture tool recorded the
conversation. The page explains the capture methodology (the exact DuckDB version, the
DuckLake extension version, and the DuckLake tutorial steps that were run), the fixture
file format, and the replay harness that plays back the corpus against SlateDuck and
diffs the responses. It also explains what "bit-for-bit identical" means in practice:
row ordering in responses must match, but server-generated timestamps and session-level
UUIDs are masked before comparison.

### 11. Contributing

#### Testing (`contributing/testing.md`)

The test pyramid for SlateDuck has five layers: property tests (using `proptest` to
verify key encoding invariants, round-trip correctness, and ID monotonicity across
simulated crashes), unit tests (per-function correctness in isolation, using
`tokio::test` for async code), golden tests (bit-for-bit output comparison against the
SQLite-backed DuckLake reference for the full DuckLake tutorial), wire-corpus replay
tests (response comparison against the captured corpus for every supported DuckDB
version), and crash-injection tests (using `fail-parallel` to verify atomicity and
durability at every required crash point). This page explains what each layer tests,
what coverage is expected for new code (property tests for any new key encoding or
value encoding; unit tests for catalog operation logic; golden test re-run for any
change to the SQL dispatcher; crash-injection tests for any new write path), and how to
run each layer selectively during development. It should also explain how to add a new
golden fixture when a new DuckDB version needs to be validated.

#### Code Style (`contributing/code-style.md`)

The code style guide covers more than formatting. It documents the naming conventions
from the roadmap's cross-cutting section: `dl_snapshot_id` for DuckLake-level snapshot
identifiers (never "snapshot_id" unqualified), `kv_snapshot` or `kv_read_view` for
SlateDB-level read views (never confused with `dl_snapshot_id`), `pending_txn` or
`pending_batch` for in-progress catalog writes. These conventions exist because the two
snapshot concepts appear together in several code paths and confusing them leads to bugs
that are hard to diagnose. The page explains the module organisation conventions (what
belongs in `slateduck-core` versus `slateduck-catalog`, what a public API in each
should look like), the error-handling patterns (all errors bubble as typed `SlateDuckError`
variants and are converted to SQLSTATE codes at the pgwire boundary), and the
documentation expectations for public API items.

### 12. Reference

Reference pages are the most frequently consulted pages by operators and integrators
who already understand the system and just need to look something up. They should be
dense, complete, and scannable. Every table should have a caption explaining what it
contains; every CLI flag should have its type, default, and a one-sentence description.
Reference pages should not be read cover to cover — they should be searchable via
Ctrl+F and navigable via the in-page table of contents.

#### Catalog Tables (`reference/catalog-tables.md`)

All 28 DuckLake catalog tables documented in tabular form, with columns, SQL types,
key composition, MVCC behaviour (versioned or not), and whether the table was
implemented in each release phase. The table should cross-reference the Architecture
key-layout page for the binary key encoding of each table. This page is the definitive
reference for a contributor implementing support for a new DuckLake spec table or for
an operator trying to understand what a `verify catalog` inconsistency means.

#### Supported SQL (`reference/sql-supported.md`)

Every SQL statement shape that SlateDuck's bounded dispatcher accepts, organised by
category, with a representative SQL example, the Rust dispatch function it maps to,
and the parameter types. This is the authoritative compatibility reference for client
developers: if a shape is listed here, it will work; if it is not listed, it will
return `SQLSTATE 0A000`. The page should also explain the shape notation used
(e.g., `SELECT ... FROM ducklake_{table} WHERE id = $1 AND begin_snapshot <= $2
AND (end_snapshot IS NULL OR $2 < end_snapshot)`) and how `$N` parameters are bound.

#### Glossary (`reference/glossary.md`)

The glossary defines every project-specific term that appears in the documentation.
Each entry gets two to four sentences explaining what the term means, why it exists,
and how it relates to other terms. The glossary is not a jargon dump — it is the
single place where the precise meaning of terms is established. Key entries include:
`dl_snapshot_id` (the monotonically increasing integer that identifies a DuckLake
catalog snapshot; distinct from SlateDB's internal read views), `catalog-data fact`
(a row in one of the 28 DuckLake catalog tables; committed facts are never physically
deleted outside excision), `infrastructure state` (counter values, writer epoch,
`retain-from` — managed with transactional updates, not subject to the immutability
constraint), `retain-from` (the query-visibility floor advanced by `slateduck gc apply`;
advancing it makes old snapshots query-inaccessible but does not delete bytes),
`excision` (the explicit, audited physical deletion of catalog facts; the only path to
byte-level deletion), `writer fencing` (SlateDB's mechanism that prevents two processes
from writing to the same catalog simultaneously), and `kv_snapshot` (a SlateDB-level
read view, distinct from a DuckLake `dl_snapshot_id`).

---

## Writing Style Guide

The documentation uses an engaging, direct style that respects the reader's intelligence
and rewards careful reading. Good technical documentation is not neutral — it has a
point of view and defends it with evidence. When a design choice was right, say so and
explain why. When a choice had costs, say so and explain the trade-off. The reader
should come away from every page feeling that they have been given the full picture, not
a curated subset designed to make the project look good.

### The Anti-Anemia Standard

The most common failure mode in technical documentation is thinness — pages that list
bullet points where paragraphs belong, that name things without explaining them, that
describe API parameters without showing what happens when you use them, that answer
"what" without ever touching "why" or "when" or "what goes wrong if you don't." This
documentation explicitly rejects that pattern. **Every page in the SlateDuck
documentation must be lengthy, interesting, informative, useful, engaging, and
written in longer paragraphs throughout.**

"Lengthy" does not mean padded. It means that every topic is treated with the depth
it deserves. A concept page on MVCC snapshots should not be three paragraphs long; it
should be long enough that a reader who arrives with no prior knowledge of MVCC comes
away with a genuine mental model of how visibility filtering works, why the
`begin_snapshot`/`end_snapshot` pair was chosen over alternatives, what happens at the
boundary conditions, and how it connects to the time-travel feature they care about
in practice. A deployment guide for AWS S3 should not be a list of environment
variables with no context; it should walk the reader through the IAM model, explain
why the catalog and data prefixes are separated, show the policy templates with
annotated comments, and anticipate the three most common configuration mistakes with
clear error messages and resolution steps. A CLI reference entry for `slateduck gc`
should not just list its flags; it should explain what GC actually does to the
catalog, what data it is safe to delete and why, what happens if you run it while
writers are active, and how to choose a retention window that matches your workload.

"Interesting" and "engaging" mean that the writing does not read like a specification
document. It reads like it was written by a person who finds this material genuinely
fascinating — because the design of a content-addressable catalog on an LSM tree with
immutability guarantees is, in fact, genuinely interesting. The author should let that
show. Surprising facts get pointed out: the single-writer constraint that sounds like a
limitation turns out to enable unlimited reader scale-out. The immutability property
that sounds like it would consume unbounded storage turns out to enable time travel as
a zero-cost consequence rather than a premium feature. These are things worth
communicating with enthusiasm, not just recording in a table.

"Longer paragraphs" is a concrete, measurable prescription. The target for narrative
sections — Getting Started, Concepts, Architecture, Design Decisions, Performance — is
paragraphs of five to ten sentences where each sentence advances the argument or builds
on the previous one. A one-sentence paragraph is almost always a stub masquerading as
content. A two-sentence paragraph usually means the idea was not fully developed before
the author moved on. When reviewing a page, read each paragraph and ask: does this
paragraph fully develop its idea, or does it assume the reader already understood it
before they got here? If the answer is the latter, the paragraph needs to be longer.

The anti-anemia standard applies to every page in the site. There are no exemptions for
"it's just a reference page" or "it's just a quickstart." Reference pages can be dense
and scannable while still having substantive introductory paragraphs for each section.
Quickstart pages can be fast-moving while still explaining why each step is necessary
and what goes wrong if you skip it. The test is simple: if a reader could skim a page
in 30 seconds and feel they got everything it has to offer, the page is too thin.

The following principles govern every page on the site:

**Lead with the "why."** Every page opens by explaining why the reader should care about
this topic. What problem does it solve? What would go wrong without it? What question
does reading this page answer? The "why" should be answered in the first paragraph; a
reader who does not find a clear "why" in the first paragraph will likely stop reading.
This applies to reference pages too — a CLI reference should open by telling the reader
what the CLI is for and when they would reach for it, not by immediately listing the
first alphabetical command.

**Use longer paragraphs for narrative sections.** The Getting Started, Concepts, and
Design Decisions sections read like well-written technical essays. Ideas are developed
in paragraphs of four to eight sentences, with each sentence contributing something
to the argument rather than just rephrasing the previous one. Concrete examples are
woven into the explanation rather than listed as afterthoughts under headers. A
paragraph that covers one idea completely is better than three bullet points that each
cover a third of an idea.

**Be honest about trade-offs.** This is the most important stylistic principle.
Every design decision has costs; every deployment model has limitations; every
performance claim has caveats. When SlateDuck is slower than PostgreSQL for a
particular workload, the documentation says so and quantifies the gap. When the
single-writer model is a real bottleneck, the documentation says so and explains the
workaround. Readers who encounter an honest limitation in the documentation before they
hit it in production will trust the documentation. Readers who hit a limitation not
mentioned in the documentation will stop trusting it, and rightly so.

**Use admonitions with purpose.** Material for MkDocs supports styled admonition boxes
(warnings, tips, notes, dangers). Use them for information that genuinely needs to
stand out from the surrounding text: a "Warning" box for an operation that cannot be
undone; a "Tip" box for a non-obvious optimization; a "Note" box for context that is
important but not part of the main flow; a collapsible "Details" block for an
explanation that deeper readers want but casual readers can skip. Do not overuse
admonitions — if every third paragraph is inside a callout box, the callout boxes lose
their emphasis.

**Include working examples at every level.** Every concept page gets at least one
DuckDB SQL example showing the concept in action. Every deployment guide has copy-paste
`slateduck serve` and `duckdb` commands with expected output. Every CLI reference entry
has a complete invocation example. Every error code in the SQLSTATE table has a
concrete "this error appears when you..." description. Examples do not need to be
complex to be useful — a three-line SQL snippet that demonstrates a concept is worth
a hundred words of prose about the concept.

**Define terms before using them.** No acronym or project-specific term appears without
definition or a link to the glossary. This applies even to terms the author considers
obvious: a reader who has never worked with an LSM tree will not know what "compaction"
means, and a reader new to DuckLake will not know what `dl_snapshot_id` means. Either
define the term inline the first time it appears on a page, or link to the glossary
entry that defines it. Both are acceptable; the inline definition is preferred when the
page is the natural first-read for the audience who would encounter the term.

**Address the reader directly.** Use second-person: "you can query historical snapshots"
not "it is possible to query historical snapshots"; "run `slateduck verify catalog` to
check integrity" not "integrity can be verified by running `slateduck verify catalog`."
Active voice, imperative mood for instructions, and second person throughout. The reader
is doing something, and the documentation is talking to them about it.

**Keep reference material scannable.** Tables, code blocks, and short entries for
reference pages. Reserve flowing prose for conceptual and tutorial content; keep
reference pages dense and navigable. Every reference table should have column headers
that explain what the column contains, every code block should have a language tag for
syntax highlighting, and every section should have a permalink anchor so it can be
linked from other pages and from error messages.

---

## MkDocs Configuration (`mkdocs.yml`)

```yaml
site_name: SlateDuck Documentation
site_url: https://trickle-labs.github.io/slateduck/
site_description: >-
  A DuckLake catalog on SlateDB — your entire lakehouse in a single S3 bucket,
  no database server required.
repo_name: trickle-labs/slateduck
repo_url: https://github.com/trickle-labs/slateduck
edit_uri: edit/main/docs/

theme:
  name: material
  features:
    - navigation.instant
    - navigation.instant.prefetch
    - navigation.tracking
    - navigation.tabs
    - navigation.tabs.sticky
    - navigation.sections
    - navigation.expand
    - navigation.path
    - navigation.top
    - navigation.footer
    - search.suggest
    - search.highlight
    - search.share
    - content.code.copy
    - content.code.annotate
    - content.tabs.link
    - content.action.edit
    - toc.follow
    - announce.dismiss
  palette:
    - media: "(prefers-color-scheme: light)"
      scheme: default
      primary: deep purple
      accent: amber
      toggle:
        icon: material/brightness-7
        name: Switch to dark mode
    - media: "(prefers-color-scheme: dark)"
      scheme: slate
      primary: deep purple
      accent: amber
      toggle:
        icon: material/brightness-4
        name: Switch to light mode
  font:
    text: Inter
    code: JetBrains Mono
  icon:
    repo: fontawesome/brands/github
    logo: material/database-outline
  custom_dir: docs/overrides

plugins:
  - search
  - minify:
      minify_html: true
  - git-revision-date-localized:
      enable_creation_date: true
      type: timeago
  - social
  - glightbox
  - redirects:
      redirect_maps: {}

markdown_extensions:
  - abbr
  - admonition
  - attr_list
  - def_list
  - footnotes
  - md_in_html
  - tables
  - toc:
      permalink: true
      toc_depth: 3
  - pymdownx.arithmatex:
      generic: true
  - pymdownx.betterem
  - pymdownx.caret
  - pymdownx.details
  - pymdownx.emoji:
      emoji_index: !!python/name:material.extensions.emoji.twemoji
      emoji_generator: !!python/name:material.extensions.emoji.to_svg
  - pymdownx.highlight:
      anchor_linenums: true
      line_spans: __span
      pygments_lang_class: true
  - pymdownx.inlinehilite
  - pymdownx.keys
  - pymdownx.mark
  - pymdownx.smartsymbols
  - pymdownx.superfences:
      custom_fences:
        - name: mermaid
          class: mermaid
          format: !!python/name:pymdownx.superfences.fence_code_format
  - pymdownx.tabbed:
      alternate_style: true
  - pymdownx.tasklist:
      custom_checkbox: true
  - pymdownx.tilde

extra:
  social:
    - icon: fontawesome/brands/github
      link: https://github.com/trickle-labs/slateduck
  generator: false
  status:
    new: Recently added
    deprecated: Deprecated

extra_css:
  - assets/stylesheets/extra.css

nav:
  - Home: index.md
  - Getting Started:
    - getting-started/index.md
    - What is SlateDuck?: getting-started/what-is-slateduck.md
    - Quickstart (Local): getting-started/quickstart.md
    - Quickstart (Cloud): getting-started/quickstart-cloud.md
    - Your First Lakehouse: getting-started/first-lakehouse.md
  - Concepts:
    - concepts/index.md
    - Lakehouse Primer: concepts/lakehouse-primer.md
    - DuckLake Format: concepts/ducklake.md
    - SlateDB Engine: concepts/slatedb.md
    - Catalog Immutability: concepts/catalog-immutability.md
    - MVCC & Snapshots: concepts/mvcc.md
    - Time Travel: concepts/time-travel.md
    - Reader Scale-Out: concepts/reader-scaleout.md
    - Writer Fencing: concepts/writer-fencing.md
    - Fact Store Vision: concepts/fact-store-vision.md
  - Architecture:
    - architecture/index.md
    - System Design: architecture/system-design.md
    - Crate Map: architecture/crate-map.md
    - Key Layout: architecture/key-layout.md
    - Value Encoding: architecture/value-encoding.md
    - SQL Dispatcher: architecture/sql-dispatcher.md
    - PG-Wire Protocol: architecture/pgwire-protocol.md
    - Transaction Model: architecture/transaction-model.md
    - Counter Allocation: architecture/counter-allocation.md
    - Data Flow: architecture/data-flow.md
  - Deployment:
    - deployment/index.md
    - Local Development: deployment/local-dev.md
    - Docker: deployment/docker.md
    - AWS S3: deployment/aws-s3.md
    - S3 Express One Zone: deployment/aws-s3-express.md
    - Google Cloud Storage: deployment/gcs.md
    - Azure Blob Storage: deployment/azure.md
    - MinIO: deployment/minio.md
    - Kubernetes: deployment/kubernetes.md
    - Lambda / Serverless: deployment/lambda.md
    - Credential Isolation: deployment/credential-isolation.md
    - TLS & Authentication: deployment/tls-and-auth.md
  - Operations:
    - operations/index.md
    - CLI Reference: operations/cli-reference.md
    - Configuration: operations/configuration.md
    - Monitoring: operations/monitoring.md
    - GC & Retention: operations/gc-and-retention.md
    - Excision: operations/excision.md
    - Checkpoints: operations/checkpoints.md
    - Export & Import: operations/export-import.md
    - Repair: operations/repair.md
    - Encryption: operations/encryption.md
    - Upgrading: operations/upgrading.md
    - Troubleshooting: operations/troubleshooting.md
  - Integration:
    - integration/index.md
    - DuckDB: integration/duckdb.md
    - DuckDB Compatibility: integration/duckdb-compatibility.md
    - pg-tide-relay: integration/pg-tide-relay.md
    - DataFusion: integration/datafusion.md
    - Native Extension: integration/native-extension.md
    - Custom Clients: integration/custom-clients.md
  - Design Decisions:
    - design-decisions/index.md
    - Why SlateDB?: design-decisions/why-slatedb.md
    - Strategy B First: design-decisions/strategy-b-first.md
    - Bounded SQL: design-decisions/bounded-sql.md
    - Protobuf Encoding: design-decisions/protobuf-encoding.md
    - Immutability Trade-offs: design-decisions/immutability-tradeoffs.md
    - Single-Writer Model: design-decisions/single-writer.md
    - Key Design Rationale: design-decisions/key-design-rationale.md
    - What SlateDuck Is Not: design-decisions/what-slateduck-is-not.md
  - Performance:
    - performance/index.md
    - Latency Model: performance/latency-model.md
    - Benchmarks: performance/benchmarks.md
    - Tuning: performance/tuning.md
    - When to Use SlateDuck: performance/when-to-use.md
    - vs. Alternatives: performance/vs-alternatives.md
  - Internals:
    - internals/index.md
    - Tag Allocation: internals/tag-allocation.md
    - MVCC Filter: internals/mvcc-filter.md
    - Inlined Data: internals/inlined-data.md
    - Schema Version: internals/schema-version.md
    - Type-Aware Stats: internals/type-aware-stats.md
    - SQLSTATE Mapping: internals/sqlstate-mapping.md
    - Wire Corpus: internals/wire-corpus.md
    - Crash Safety: internals/crash-safety.md
  - Contributing:
    - contributing/index.md
    - Development Setup: contributing/development-setup.md
    - Testing: contributing/testing.md
    - Code Style: contributing/code-style.md
    - Architecture Guide: contributing/architecture-guide.md
    - Release Process: contributing/release-process.md
  - Reference:
    - reference/index.md
    - Catalog Tables: reference/catalog-tables.md
    - Supported SQL: reference/sql-supported.md
    - Error Codes: reference/error-codes.md
    - Metrics: reference/metrics.md
    - Environment Variables: reference/environment-vars.md
    - Glossary: reference/glossary.md
  - Roadmap:
    - roadmap/index.md
    - Changelog: roadmap/changelog.md
```

---

## GitHub Actions Workflow

```yaml
# .github/workflows/docs.yml
name: Documentation

on:
  push:
    branches: [main]
    paths:
      - 'docs/**'
      - 'mkdocs.yml'
      - '.github/workflows/docs.yml'
  pull_request:
    paths:
      - 'docs/**'
      - 'mkdocs.yml'

permissions:
  contents: write
  pages: write
  id-token: write

concurrency:
  group: docs-${{ github.ref }}
  cancel-in-progress: true

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0  # Required for git-revision-date-localized

      - uses: actions/setup-python@v5
        with:
          python-version: '3.12'
          cache: 'pip'

      - name: Install dependencies
        run: pip install -r requirements-docs.txt

      - name: Build documentation
        run: mkdocs build --strict

      - name: Upload artifact
        if: github.event_name == 'push' && github.ref == 'refs/heads/main'
        uses: actions/upload-pages-artifact@v3
        with:
          path: site/

  deploy:
    if: github.event_name == 'push' && github.ref == 'refs/heads/main'
    needs: build
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - name: Deploy to GitHub Pages
        id: deployment
        uses: actions/deploy-pages@v4
```

---

## Implementation Phases

### Phase D1 — Scaffolding (days 1–2)

Before any content is written, the infrastructure must exist and be verified. This phase
creates the `mkdocs.yml` at the workspace root with the full configuration above; the
`docs/` directory structure with all subdirectories listed in the file tree; the
`docs/assets/stylesheets/extra.css` file with the initial custom theme overrides (custom
colour for the `why-this-matters` admonition type, typography tweaks if needed);
the `docs/overrides/main.html` template with the hero section for the landing page;
and the `.github/workflows/docs.yml` workflow with the build and deploy jobs. The
`requirements-docs.txt` file is created with pinned versions of every documentation
dependency: `mkdocs-material`, `mkdocs-minify-plugin`, `mkdocs-git-revision-date-
localized-plugin`, `mkdocs-redirects`, `mkdocs-glightbox`, `pillow`, and `cairosvg`
(the last two are required by the social plugin for Open Graph card generation).

The phase concludes by running `mkdocs serve` locally and verifying that the site builds
without errors, the navigation renders correctly, the search index is populated, and at
least one diagram renders correctly in a stub page. Every section directory gets an
`index.md` stub with the section title and a two-sentence description so that the
navigation links do not 404 during the content-writing phases that follow.

### Phase D2 — Getting Started and Landing (days 3–5)

The pages in this phase are the highest-traffic pages on the site and must be
polished before anything else is written. The landing page is written first because it
forces the author to articulate the core value proposition clearly; that articulation
should inform the tone and framing of everything else. The "What is SlateDuck?" page
is written second because it is the conceptual foundation that all other getting-started
content builds on. The local quickstart is written third and verified by actually running
the commands on a clean machine — if any command produces unexpected output or requires
an undocumented step, the quickstart is wrong and must be fixed before proceeding. The
cloud quickstart is written fourth, tested against real S3, GCS, and Azure buckets, and
the first-lakehouse tutorial is written last, incorporating the time-travel demonstration
that validates the immutability property.

### Phase D3 — Concepts and Architecture (days 6–12)

These pages require the most sustained writing effort because they are the most
intellectually demanding. The Concepts pages are written in order from most foundational
to most advanced: Lakehouse Primer → DuckLake Format → SlateDB Engine → Catalog
Immutability → MVCC → Time Travel → Reader Scale-Out → Writer Fencing → Fact Store
Vision. Each page is cross-linked to the relevant Architecture page so readers who want
to go from "what and why" to "how" have a clear path. The Architecture pages are written
in roughly the same order: System Design first (because it sets the context for all
others), then Crate Map, Key Layout, Value Encoding, SQL Dispatcher, PG-Wire Protocol,
Transaction Model, Counter Allocation, and Data Flow. The Mermaid diagrams in System
Design and Data Flow require the most iteration — they should be reviewed by at least
one other person to verify they accurately represent the implementation.

### Phase D4 — Deployment and Operations (days 13–19)

Deployment guides are written in order from simplest to most complex: Local Dev →
Docker → MinIO → AWS S3 → S3 Express → GCS → Azure → Kubernetes → Lambda →
Credential Isolation → TLS/Auth. Each guide is tested against a real environment
before it is committed. Operations pages are written in dependency order: CLI Reference
first (because it is referenced from nearly every other operations page) → Configuration
→ Monitoring → GC and Retention → Excision → Checkpoints → Export/Import → Repair →
Encryption → Upgrading → Troubleshooting. The troubleshooting guide is written last
and incorporates failure modes discovered while testing the deployment guides.

### Phase D5 — Integration and Design Decisions (days 20–24)

Integration pages are written in order from most important to least: DuckDB → DuckDB
Compatibility → Custom Clients → pg-tide-relay → DataFusion → Native Extension. The
DuckDB page requires a complete end-to-end test to verify all examples work; the
compatibility page requires running the corpus replay test suite to confirm the version
matrix is accurate. The Design Decisions pages require the most care of any section
because they must present both sides of each choice honestly. The order of writing is
driven by what content is most likely to be read first: Why SlateDB → Strategy B First
→ Bounded SQL → Protobuf Encoding → Immutability Trade-offs → Single-Writer → Key
Design Rationale → What SlateDuck Is Not. Each page should be reviewed by the person
who made the original design decision to verify that the reasoning is represented
accurately.

### Phase D6 — Performance, Internals, and Reference (days 25–30)

Performance pages require real benchmark data from `benchmarks/phase-2-baseline.json`
and subsequent runs. The Latency Model is written first to establish the theoretical
framework; the Benchmarks page then presents the measured data against that framework;
the Tuning page explains how to improve on the baseline numbers; the When to Use
SlateDuck page synthesises both into a decision framework; and the vs. Alternatives
page applies all of it to a direct comparison. Internals pages are written in order
of complexity: Tag Allocation → MVCC Filter → Inlined Data → Schema Version → Type-
Aware Stats → SQLSTATE Mapping → Wire Corpus → Crash Safety. Reference pages are
generated largely from the source: Catalog Tables derives from `tags.rs`; Supported
SQL derives from the dispatcher source; Error Codes derives from the SQLSTATE mapping
table; Metrics derives from the metrics module; Environment Variables derives from
the CLI flag definitions.

### Phase D7 — Contributing, Roadmap, and Polish (days 31–35)

Contributing pages are written from the perspective of a brand-new contributor who
has just cloned the repository: Development Setup walks them from zero to a passing
test suite; Testing explains the test pyramid and what coverage is expected; Code Style
covers the naming conventions and module organisation; Architecture Guide explains how
to navigate the codebase; Release Process explains how to cut a release. The Roadmap
and Changelog pages are populated from the ROADMAP.md, reformatted for web reading
with status badges. The polish phase runs `mkdocs build --strict` to zero all warnings,
audits every cross-link to verify it resolves correctly, reviews every code example to
confirm it still works, checks every page on a mobile viewport, and verifies that the
top 20 terms a new user would search for (e.g., "time travel," "S3," "DuckDB," "GC,"
"encryption," "upgrade," "writer fencing") return the correct pages in the search index.

---

## Content Sources

The documentation is not written from scratch — it synthesises and expands on a rich
body of existing project documentation and source code. Each primary source is listed
below with the documentation sections it primarily feeds, so that authors writing each
section know where to look for the raw material.

| Source | Primary content for |
|--------|---------------------|
| `plans/blueprint.md` | Architecture (all pages), Design Decisions (all pages), key layout details, value encoding spec, transaction model, SQLSTATE mapping, type-aware stats |
| `ROADMAP.md` | Roadmap page, Changelog, Concepts (all pages reference the roadmap for versioning context), feature-coverage-by-release callouts |
| `README.md` | Landing page pitch and architecture overview; Getting Started introduction |
| `docs/quickstart.md` | Getting Started quickstart guides (expand the existing stub into full coverage) |
| `docs/architecture.md` | Architecture overview; System Design starting point |
| `docs/compatibility.md` | DuckDB Compatibility page (expand into full version matrix with corpus validation process) |
| `CONTRIBUTING.md` | Contributing section (expand into five full pages) |
| `benchmarks/phase-2-baseline.json` | Performance pages: Latency Model baseline numbers, Benchmarks page |
| `docs/phase-0/access-patterns.md` | Key Layout rationale, SQL Dispatcher design |
| `docs/phase-0/slatedb-api-validation.md` | Architecture Transaction Model, SlateDB Engine concept page |
| `docs/phase-0/latency-baseline.json` | Performance Latency Model, Benchmarks |
| `docs/phase-0/credential-isolation.md` | Credential Isolation deployment guide |
| `crates/slateduck-core/src/tags.rs` | Tag Allocation internals, Catalog Tables reference (authoritative source) |
| `crates/slateduck-pgwire/src/` | PG-Wire Protocol architecture, SQL Dispatcher architecture |
| `crates/slateduck-catalog/src/` | Operations reference, Catalog Tables reference |
| `crates/slateduck-sql/src/` | SQL Dispatcher architecture, Supported SQL reference |
| `tests/fixtures/wire-corpus/` | Wire Corpus internals, DuckDB Compatibility |
| `tests/golden/` | DuckDB Compatibility, golden test methodology in Contributing/Testing |

---

## Quality Gates

The documentation release is considered complete when every item in this checklist is
verified. Each gate exists for a specific reason; the rationale is given alongside the
check so that the person running the verification understands what they are testing.

**Build correctness.** `mkdocs build --strict` must produce zero warnings on the CI
build environment. The `--strict` flag turns broken internal links, missing nav entries,
and malformed extension directives into build failures. Any warning in the output is a
defect in the documentation source that must be fixed before publishing.

**Link integrity.** No broken internal or external links. Internal links are caught by
`--strict`. External links should be tested with a link checker (the `mkdocs-htmlproofer`
plugin or equivalent) before the initial publish and then spot-checked on each
subsequent update. Broken external links erode reader trust and are a maintenance
burden.

**Content completeness and depth.** No stub pages, and no thin pages. Every page in
the navigation must be lengthy, informative, and engaging — not just technically
accurate but genuinely useful to the audience it serves. A page that has real content
but only covers the surface of its topic is as much a failure as a stub. The minimum
bar for any page is: a substantive introduction explaining why the topic matters, full
coverage of the core subject with longer paragraphs that develop each idea completely,
at least one working code or command example, and a connection to related pages for
readers who want to go deeper. For concept and design pages, the minimum is closer to
a full technical essay — long enough that a reader who arrives without prior knowledge
of the topic leaves with a complete mental model. A draft is not considered complete
until a reviewer who is not the author has read it and confirmed it is not anemic.
The question to ask of every page before it ships: would a reader who spends five
minutes on this page come away materially better informed than before? If the answer
is anything other than "yes, clearly," the page needs more work.

**Example correctness.** Every `bash` and `sql` code block that represents a user-
executable command must have been run against the actual binary on the actual object-
store backend indicated. A code example that does not work is misinformation. Example
verification should be tracked in a checklist alongside the phase-D4 deployment guide
work, where the most examples exist.

**Search coverage.** The top 20 terms a new user would search for must return relevant
results. The list of terms should be assembled before the polish phase by asking
multiple people "what would you search for on the SlateDuck documentation site?" and
combining the answers. Acceptable search result: the most relevant page is in the top
three results for the term.

**Mobile rendering.** All pages render correctly on a 375 px wide viewport (iPhone SE
size). This particularly matters for wide tables in the Reference section and for the
architecture diagrams; Mermaid diagrams in particular can overflow their container on
narrow viewports if not styled carefully.

**Accessibility.** Every image and diagram has a meaningful `alt` attribute (not just
the filename). Heading hierarchy is correct (one `h1` per page, `h2` sections,
`h3` subsections — no skipped levels). Body text has sufficient contrast against the
background in both light and dark mode. These checks can be partially automated with
a browser accessibility tool.

**Peer review.** At least one person other than the primary author has read every page
in the Getting Started and Concepts sections. These are the highest-traffic pages and
the ones most likely to confuse a new reader; a second reader finds the gaps that the
author is blind to after repeated revision.

---

## Maintenance Plan

Good documentation rots without active maintenance. The maintenance plan defines the
minimal set of processes that keep the site accurate and current over the lifetime of
the project, without creating an unreasonable ongoing burden.

The single most important process is the PR template checklist: every pull request that
changes observable user-facing behaviour (adds a CLI flag, changes a configuration
option, modifies an error code, changes a metric name, alters the protocol) must
include a corresponding documentation update as a condition of merge. The PR template
includes a checkbox: "Documentation updated (if behaviour changed)." A PR that changes
behaviour without updating documentation is a documentation bug introduced deliberately,
and the review process should treat it as such.

For stable ongoing operations, the following checks happen on each release cycle: new
CLI commands get a CLI Reference entry before the release is tagged (operators who
upgrade and reach for `--help` should find the same information in the documentation);
new Prometheus metrics get a Metrics reference entry (monitoring setups break silently
when metrics disappear or are renamed); new error codes get an Error Codes entry (a
DBA who sees `SQLSTATE XX001` for the first time should be able to find it in the
documentation immediately). Benchmark results are updated with each performance-relevant
release — results that are months or years old without a revision date are misleading,
and the `git-revision-date-localized` plugin surfaces the last-updated date on every
page to make stale content visible.

The DuckDB compatibility matrix is updated within two weeks of a new DuckDB release:
the corpus capture process runs, the replay tests pass, and the matrix entry is added.
If the replay tests fail, an incompatibility entry is added with a description of the
issue and an expected resolution timeline. An up-to-date compatibility matrix is one of
the first things a new user checks before adopting SlateDuck, and an outdated matrix
(or one that silently says nothing about recent versions) is a significant barrier to
adoption.

Pages that have not been updated in six months are flagged by the `git-revision-date-
localized` plugin with a visible "last updated" timestamp. The six-month flag is a
trigger for a review: is the content still accurate? Has the API changed? Are there new
best practices that should be incorporated? Not all six-month-old pages need updates —
the Concepts section in particular should be stable — but the review ensures that
operators are not following outdated guidance.


---

## MkDocs Configuration (`mkdocs.yml`)

```yaml
site_name: SlateDuck Documentation
site_url: https://trickle-labs.github.io/slateduck/
site_description: >-
  A DuckLake catalog on SlateDB — your entire lakehouse in a single S3 bucket,
  no database server required.
repo_name: trickle-labs/slateduck
repo_url: https://github.com/trickle-labs/slateduck
edit_uri: edit/main/docs/

theme:
  name: material
  features:
    - navigation.instant
    - navigation.instant.prefetch
    - navigation.tracking
    - navigation.tabs
    - navigation.tabs.sticky
    - navigation.sections
    - navigation.expand
    - navigation.path
    - navigation.top
    - navigation.footer
    - search.suggest
    - search.highlight
    - search.share
    - content.code.copy
    - content.code.annotate
    - content.tabs.link
    - content.action.edit
    - toc.follow
    - announce.dismiss
  palette:
    - media: "(prefers-color-scheme: light)"
      scheme: default
      primary: deep purple
      accent: amber
      toggle:
        icon: material/brightness-7
        name: Switch to dark mode
    - media: "(prefers-color-scheme: dark)"
      scheme: slate
      primary: deep purple
      accent: amber
      toggle:
        icon: material/brightness-4
        name: Switch to light mode
  font:
    text: Inter
    code: JetBrains Mono
  icon:
    repo: fontawesome/brands/github
    logo: material/database-outline
  custom_dir: docs/overrides

plugins:
  - search
  - minify:
      minify_html: true
  - git-revision-date-localized:
      enable_creation_date: true
      type: timeago
  - social
  - glightbox
  - redirects:
      redirect_maps: {}

markdown_extensions:
  - abbr
  - admonition
  - attr_list
  - def_list
  - footnotes
  - md_in_html
  - tables
  - toc:
      permalink: true
      toc_depth: 3
  - pymdownx.arithmatex:
      generic: true
  - pymdownx.betterem
  - pymdownx.caret
  - pymdownx.details
  - pymdownx.emoji:
      emoji_index: !!python/name:material.extensions.emoji.twemoji
      emoji_generator: !!python/name:material.extensions.emoji.to_svg
  - pymdownx.highlight:
      anchor_linenums: true
      line_spans: __span
      pygments_lang_class: true
  - pymdownx.inlinehilite
  - pymdownx.keys
  - pymdownx.mark
  - pymdownx.smartsymbols
  - pymdownx.superfences:
      custom_fences:
        - name: mermaid
          class: mermaid
          format: !!python/name:pymdownx.superfences.fence_code_format
  - pymdownx.tabbed:
      alternate_style: true
  - pymdownx.tasklist:
      custom_checkbox: true
  - pymdownx.tilde

extra:
  social:
    - icon: fontawesome/brands/github
      link: https://github.com/trickle-labs/slateduck
  generator: false
  status:
    new: Recently added
    deprecated: Deprecated

extra_css:
  - assets/stylesheets/extra.css

nav:
  - Home: index.md
  - Getting Started:
    - getting-started/index.md
    - What is SlateDuck?: getting-started/what-is-slateduck.md
    - Quickstart (Local): getting-started/quickstart.md
    - Quickstart (Cloud): getting-started/quickstart-cloud.md
    - Your First Lakehouse: getting-started/first-lakehouse.md
  - Concepts:
    - concepts/index.md
    - Lakehouse Primer: concepts/lakehouse-primer.md
    - DuckLake Format: concepts/ducklake.md
    - SlateDB Engine: concepts/slatedb.md
    - Catalog Immutability: concepts/catalog-immutability.md
    - MVCC & Snapshots: concepts/mvcc.md
    - Time Travel: concepts/time-travel.md
    - Reader Scale-Out: concepts/reader-scaleout.md
    - Writer Fencing: concepts/writer-fencing.md
    - Fact Store Vision: concepts/fact-store-vision.md
  - Architecture:
    - architecture/index.md
    - System Design: architecture/system-design.md
    - Crate Map: architecture/crate-map.md
    - Key Layout: architecture/key-layout.md
    - Value Encoding: architecture/value-encoding.md
    - SQL Dispatcher: architecture/sql-dispatcher.md
    - PG-Wire Protocol: architecture/pgwire-protocol.md
    - Transaction Model: architecture/transaction-model.md
    - Counter Allocation: architecture/counter-allocation.md
    - Data Flow: architecture/data-flow.md
  - Deployment:
    - deployment/index.md
    - Local Development: deployment/local-dev.md
    - Docker: deployment/docker.md
    - AWS S3: deployment/aws-s3.md
    - S3 Express One Zone: deployment/aws-s3-express.md
    - Google Cloud Storage: deployment/gcs.md
    - Azure Blob Storage: deployment/azure.md
    - MinIO: deployment/minio.md
    - Kubernetes: deployment/kubernetes.md
    - Lambda / Serverless: deployment/lambda.md
    - Credential Isolation: deployment/credential-isolation.md
    - TLS & Authentication: deployment/tls-and-auth.md
  - Operations:
    - operations/index.md
    - CLI Reference: operations/cli-reference.md
    - Configuration: operations/configuration.md
    - Monitoring: operations/monitoring.md
    - GC & Retention: operations/gc-and-retention.md
    - Excision: operations/excision.md
    - Checkpoints: operations/checkpoints.md
    - Export & Import: operations/export-import.md
    - Repair: operations/repair.md
    - Encryption: operations/encryption.md
    - Upgrading: operations/upgrading.md
    - Troubleshooting: operations/troubleshooting.md
  - Integration:
    - integration/index.md
    - DuckDB: integration/duckdb.md
    - DuckDB Compatibility: integration/duckdb-compatibility.md
    - pg-tide-relay: integration/pg-tide-relay.md
    - DataFusion: integration/datafusion.md
    - Native Extension: integration/native-extension.md
    - Custom Clients: integration/custom-clients.md
  - Design Decisions:
    - design-decisions/index.md
    - Why SlateDB?: design-decisions/why-slatedb.md
    - Strategy B First: design-decisions/strategy-b-first.md
    - Bounded SQL: design-decisions/bounded-sql.md
    - Protobuf Encoding: design-decisions/protobuf-encoding.md
    - Immutability Trade-offs: design-decisions/immutability-tradeoffs.md
    - Single-Writer Model: design-decisions/single-writer.md
    - Key Design Rationale: design-decisions/key-design-rationale.md
    - What SlateDuck Is Not: design-decisions/what-slateduck-is-not.md
  - Performance:
    - performance/index.md
    - Latency Model: performance/latency-model.md
    - Benchmarks: performance/benchmarks.md
    - Tuning: performance/tuning.md
    - When to Use SlateDuck: performance/when-to-use.md
    - vs. Alternatives: performance/vs-alternatives.md
  - Internals:
    - internals/index.md
    - Tag Allocation: internals/tag-allocation.md
    - MVCC Filter: internals/mvcc-filter.md
    - Inlined Data: internals/inlined-data.md
    - Schema Version: internals/schema-version.md
    - Type-Aware Stats: internals/type-aware-stats.md
    - SQLSTATE Mapping: internals/sqlstate-mapping.md
    - Wire Corpus: internals/wire-corpus.md
    - Crash Safety: internals/crash-safety.md
  - Contributing:
    - contributing/index.md
    - Development Setup: contributing/development-setup.md
    - Testing: contributing/testing.md
    - Code Style: contributing/code-style.md
    - Architecture Guide: contributing/architecture-guide.md
    - Release Process: contributing/release-process.md
  - Reference:
    - reference/index.md
    - Catalog Tables: reference/catalog-tables.md
    - Supported SQL: reference/sql-supported.md
    - Error Codes: reference/error-codes.md
    - Metrics: reference/metrics.md
    - Environment Variables: reference/environment-vars.md
    - Glossary: reference/glossary.md
  - Roadmap:
    - roadmap/index.md
    - Changelog: roadmap/changelog.md
```

---

## GitHub Actions Workflow

```yaml
# .github/workflows/docs.yml
name: Documentation

on:
  push:
    branches: [main]
    paths:
      - 'docs/**'
      - 'mkdocs.yml'
      - '.github/workflows/docs.yml'
  pull_request:
    paths:
      - 'docs/**'
      - 'mkdocs.yml'

permissions:
  contents: write
  pages: write
  id-token: write

concurrency:
  group: docs-${{ github.ref }}
  cancel-in-progress: true

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0  # Required for git-revision-date-localized

      - uses: actions/setup-python@v5
        with:
          python-version: '3.12'
          cache: 'pip'

      - name: Install dependencies
        run: |
          pip install \
            mkdocs-material \
            mkdocs-minify-plugin \
            mkdocs-git-revision-date-localized-plugin \
            mkdocs-redirects \
            mkdocs-glightbox \
            pillow \
            cairosvg

      - name: Build documentation
        run: mkdocs build --strict

      - name: Upload artifact
        if: github.event_name == 'push' && github.ref == 'refs/heads/main'
        uses: actions/upload-pages-artifact@v3
        with:
          path: site/

  deploy:
    if: github.event_name == 'push' && github.ref == 'refs/heads/main'
    needs: build
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - name: Deploy to GitHub Pages
        id: deployment
        uses: actions/deploy-pages@v4
```

---

