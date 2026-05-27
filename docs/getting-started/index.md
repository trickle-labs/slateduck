# Getting Started

Welcome to Rocklake. This section takes you from knowing nothing about the project to running a fully functional lakehouse catalog backed by object storage. Whether you are evaluating Rocklake for a production data platform, exploring it for a side project, or just curious about how a database catalog can live in an S3 bucket, you will find your entry point here.

## Choose Your Path

The Getting Started section is organized as a progressive sequence. Each page builds on the previous one, but you can jump to whichever matches your current level of familiarity:

### :material-help-circle: [What is Rocklake?](what-is-rocklake.md)

Start here if you have never heard of Rocklake before, or if you know the name but are not sure what problem it solves. This page explains the landscape — what a lakehouse catalog is, why existing solutions require infrastructure you might not want, and how Rocklake eliminates that infrastructure by putting everything in object storage. No code, no commands, just a clear explanation of what this project is and why it exists.

### :material-rocket-launch: [Quickstart — Local](quickstart.md)

Start here if you want to get your hands dirty immediately. This page takes you from zero to a working catalog in under five minutes using your local filesystem as the storage backend. You will build Rocklake from source (or download a release binary), start the server, connect DuckDB, create a table, insert data, and query it. Every command is shown with its expected output so you can verify each step succeeded.

### :material-cloud-outline: [Quickstart — Cloud](quickstart-cloud.md)

Start here if you already have an AWS, GCS, or Azure account and want to see Rocklake running against a real object store. This page provides parallel tracks for each major cloud provider, covering bucket creation, IAM configuration, environment variable setup, and verification. By the end, your catalog will be living in the cloud with the same durability guarantees as your production data.

### :material-school: [Your First Lakehouse](first-lakehouse.md)

Start here if you have completed the quickstart and want to understand what Rocklake actually does at a deeper level. This tutorial walks through a realistic scenario — building a product analytics lakehouse from scratch — at a deliberate pace, explaining what happens in the catalog at each step. You will create schemas, define tables, evolve schemas, insert data across multiple transactions, and then use time travel to query historical states of your catalog. By the end, you will have a genuine mental model of how DuckLake catalogs work and what makes Rocklake's approach distinctive.

## Prerequisites

Before you begin, you will need:

- **A Rust toolchain** (for building from source) or a **pre-built binary** from the [releases page](https://github.com/trickle-labs/rocklake/releases). Rocklake is a single static binary with no runtime dependencies beyond libc.
- **DuckDB 1.2 or later** with the `ducklake` extension installed. You can install the extension from within DuckDB by running `INSTALL ducklake;` followed by `LOAD ducklake;`.
- **For cloud deployments:** credentials for your object-store provider (AWS access keys, GCS service account, or Azure connection string). The cloud quickstart covers the minimum permissions needed.

## What You Will Learn

By the time you finish this section, you will understand:

1. What problem Rocklake solves and where it fits in the modern data stack
2. How to start and stop the Rocklake server process
3. How to connect DuckDB to a Rocklake-backed catalog
4. How to create schemas, tables, and insert data through DuckDB
5. How time travel works — querying the catalog at any historical point
6. How to deploy against a real cloud object store with proper credentials
7. What the catalog looks like from the inside — which tables are written, which counters advance, which snapshots are created

This foundation prepares you for the [Concepts](../concepts/index.md) section, which explains the engineering principles behind what you have just seen in practice, and the [Deployment](../deployment/index.md) section, which covers production-ready configurations for every major cloud provider.
