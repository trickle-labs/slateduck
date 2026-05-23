# PG-Wire Protocol

SlateDuck implements PostgreSQL wire protocol v3.

## Startup Sequence

```mermaid
sequenceDiagram
    participant C as DuckDB
    participant S as SlateDuck
    C->>S: StartupMessage(user, database, params)
    S->>C: AuthenticationOk
    S->>C: ParameterStatus(server_version, ...)
    S->>C: BackendKeyData(pid, secret)
    S->>C: ReadyForQuery('I')
```

## Simple Query Protocol

```mermaid
sequenceDiagram
    participant C as DuckDB
    participant S as SlateDuck
    C->>S: Query(sql)
    S->>C: RowDescription(columns)
    S->>C: DataRow(values)
    S->>C: CommandComplete(tag)
    S->>C: ReadyForQuery('I')
```

## What Is Not Implemented

- SSL/TLS (use a proxy)
- SASL authentication (use network-level access control)
- COPY protocol (not used by DuckLake)
