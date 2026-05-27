# Custom Clients

Because Rocklake speaks the PostgreSQL wire protocol, any PostgreSQL-compatible client library can connect to it. This opens the door to building custom tooling, monitoring dashboards, migration scripts, administrative interfaces, and CI/CD integrations in any language with a PostgreSQL driver — which is effectively every programming language in existence.

This page covers the protocol details, language-specific examples for Python, Go, Node.js, Java, Ruby, and Rust, the important limitations of what SQL Rocklake actually accepts, and practical patterns for building production-quality custom clients.

## Connection Details

Rocklake accepts standard PostgreSQL protocol connections:

| Property | Value |
|----------|-------|
| Protocol version | 3.0 (PostgreSQL v7.4+ wire protocol) |
| Default port | 5432 |
| Authentication | `AuthenticationOk` (no auth) or `AuthenticationCleartextPassword` |
| TLS | SSLRequest negotiation supported |
| Database name | Any value accepted (single-catalog server) |
| Username | Any value accepted (logged for audit) |
| Encoding | UTF-8 only |

### Connection String Format

Standard PostgreSQL connection string format works:

```
postgresql://username:password@hostname:port/database

# Examples:
postgresql://localhost:5432/rocklake
postgresql://reader@rocklake.internal:5432/catalog?sslmode=require
```

## Language Examples

### Python (psycopg2)

```python
import psycopg2

# Connect
conn = psycopg2.connect(
    host="localhost",
    port=5432,
    dbname="rocklake",
    user="admin",
    password="secret"  # Only if authentication is configured
)
conn.autocommit = True

# List schemas
cur = conn.cursor()
cur.execute("SELECT schema_id, schema_name FROM ducklake_schemas WHERE database_id = 1 AND end_snapshot_id IS NULL")
schemas = cur.fetchall()
for schema_id, schema_name in schemas:
    print(f"Schema: {schema_name} (id={schema_id})")

# List tables in a schema
cur.execute("SELECT table_id, table_name FROM ducklake_tables WHERE schema_id = %s AND end_snapshot_id IS NULL", (1,))
tables = cur.fetchall()

# Get column definitions
cur.execute("""
    SELECT column_name, data_type, is_nullable 
    FROM ducklake_columns 
    WHERE table_id = %s AND end_snapshot_id IS NULL
    ORDER BY ordinal_position
""", (1,))
columns = cur.fetchall()

cur.close()
conn.close()
```

### Python (asyncpg)

```python
import asyncio
import asyncpg

async def main():
    conn = await asyncpg.connect(
        host='localhost',
        port=5432,
        database='rocklake',
        user='admin'
    )
    
    # List schemas
    rows = await conn.fetch(
        "SELECT schema_id, schema_name FROM ducklake_schemas WHERE database_id = $1 AND end_snapshot_id IS NULL",
        1
    )
    for row in rows:
        print(f"Schema: {row['schema_name']}")
    
    await conn.close()

asyncio.run(main())
```

### Go (pgx)

```go
package main

import (
    "context"
    "fmt"
    "log"
    
    "github.com/jackc/pgx/v5"
)

func main() {
    ctx := context.Background()
    
    conn, err := pgx.Connect(ctx, "postgres://localhost:5432/rocklake")
    if err != nil {
        log.Fatal(err)
    }
    defer conn.Close(ctx)
    
    // List schemas
    rows, err := conn.Query(ctx, 
        "SELECT schema_id, schema_name FROM ducklake_schemas WHERE database_id = $1 AND end_snapshot_id IS NULL", 
        1)
    if err != nil {
        log.Fatal(err)
    }
    defer rows.Close()
    
    for rows.Next() {
        var id int64
        var name string
        if err := rows.Scan(&id, &name); err != nil {
            log.Fatal(err)
        }
        fmt.Printf("Schema: %s (id=%d)\n", name, id)
    }
    
    // List tables
    rows, err = conn.Query(ctx,
        "SELECT table_id, table_name FROM ducklake_tables WHERE schema_id = $1 AND end_snapshot_id IS NULL",
        1)
    if err != nil {
        log.Fatal(err)
    }
    defer rows.Close()
    
    for rows.Next() {
        var id int64
        var name string
        rows.Scan(&id, &name)
        fmt.Printf("  Table: %s (id=%d)\n", name, id)
    }
}
```

### Node.js (pg)

```javascript
const { Client } = require('pg');

async function main() {
    const client = new Client({
        host: 'localhost',
        port: 5432,
        database: 'rocklake',
        user: 'admin',
    });
    
    await client.connect();
    
    // List schemas
    const schemas = await client.query(
        'SELECT schema_id, schema_name FROM ducklake_schemas WHERE database_id = $1 AND end_snapshot_id IS NULL',
        [1]
    );
    
    for (const row of schemas.rows) {
        console.log(`Schema: ${row.schema_name} (id=${row.schema_id})`);
    }
    
    // List tables in schema
    const tables = await client.query(
        'SELECT table_id, table_name FROM ducklake_tables WHERE schema_id = $1 AND end_snapshot_id IS NULL',
        [schemas.rows[0].schema_id]
    );
    
    for (const row of tables.rows) {
        console.log(`  Table: ${row.table_name} (id=${row.table_id})`);
    }
    
    await client.end();
}

main().catch(console.error);
```

### Java (JDBC)

```java
import java.sql.*;

public class RocklakeClient {
    public static void main(String[] args) throws SQLException {
        String url = "jdbc:postgresql://localhost:5432/rocklake";
        
        try (Connection conn = DriverManager.getConnection(url, "admin", "")) {
            // List schemas
            PreparedStatement ps = conn.prepareStatement(
                "SELECT schema_id, schema_name FROM ducklake_schemas WHERE database_id = ? AND end_snapshot_id IS NULL"
            );
            ps.setInt(1, 1);
            
            ResultSet rs = ps.executeQuery();
            while (rs.next()) {
                System.out.printf("Schema: %s (id=%d)%n", 
                    rs.getString("schema_name"), 
                    rs.getLong("schema_id"));
            }
            
            // List tables
            ps = conn.prepareStatement(
                "SELECT table_id, table_name FROM ducklake_tables WHERE schema_id = ? AND end_snapshot_id IS NULL"
            );
            ps.setInt(1, 1);
            
            rs = ps.executeQuery();
            while (rs.next()) {
                System.out.printf("  Table: %s (id=%d)%n",
                    rs.getString("table_name"),
                    rs.getLong("table_id"));
            }
        }
    }
}
```

### Ruby (pg gem)

```ruby
require 'pg'

conn = PG.connect(host: 'localhost', port: 5432, dbname: 'rocklake', user: 'admin')

# List schemas
result = conn.exec_params(
  'SELECT schema_id, schema_name FROM ducklake_schemas WHERE database_id = $1 AND end_snapshot_id IS NULL',
  [1]
)

result.each do |row|
  puts "Schema: #{row['schema_name']} (id=#{row['schema_id']})"
end

conn.close
```

### Rust (tokio-postgres)

```rust
use tokio_postgres::{NoTls, Error};

#[tokio::main]
async fn main() -> Result<(), Error> {
    let (client, connection) = tokio_postgres::connect(
        "host=localhost port=5432 dbname=rocklake user=admin",
        NoTls
    ).await?;
    
    tokio::spawn(async move { connection.await.unwrap(); });
    
    // List schemas
    let rows = client.query(
        "SELECT schema_id, schema_name FROM ducklake_schemas WHERE database_id = $1 AND end_snapshot_id IS NULL",
        &[&1i64]
    ).await?;
    
    for row in &rows {
        let id: i64 = row.get("schema_id");
        let name: &str = row.get("schema_name");
        println!("Schema: {} (id={})", name, id);
    }
    
    Ok(())
}
```

## Important Limitations

### Bounded SQL Only

Custom clients can send only the specific SQL statements that Rocklake's bounded SQL dispatcher recognizes. These are the catalog metadata queries that DuckDB's `ducklake` extension emits. Arbitrary SQL will be rejected:

```python
# This works (recognized pattern):
cur.execute("SELECT schema_id, schema_name FROM ducklake_schemas WHERE database_id = 1")

# This FAILS (arbitrary SQL):
cur.execute("SELECT count(*) FROM ducklake_schemas GROUP BY database_id")

# This FAILS (not a recognized table):
cur.execute("SELECT * FROM my_custom_table")
```

### No Data Access

Rocklake only manages catalog metadata. You cannot query actual data through the PG-wire connection. Data files (Parquet) must be accessed directly from object storage using appropriate libraries (PyArrow, DuckDB, DataFusion, etc.).

### No Prepared Statement Caching

Rocklake does not maintain server-side prepared statements across requests. Each query is parsed independently. Client-side connection pools that rely on prepared statement caching should disable this feature.

### Transaction Semantics

```python
# Transactions work for write operations:
conn.autocommit = False
cur.execute("INSERT INTO ducklake_schemas ...")
cur.execute("INSERT INTO ducklake_tables ...")
conn.commit()  # Atomic: both or neither

# But reads are always at the latest snapshot regardless of transaction state
```

## Use Cases

### Monitoring and Alerting

```python
import psycopg2
import time

def check_catalog_health(conn):
    """Monitor catalog for anomalies."""
    cur = conn.cursor()
    
    # Check writer epoch (high value = many restarts)
    cur.execute("SELECT value FROM rocklake_system WHERE key = 'epoch'")
    epoch = cur.fetchone()[0]
    if epoch > 10:
        alert(f"High writer epoch: {epoch}")
    
    # Check latest snapshot (should be advancing)
    cur.execute("SELECT value FROM rocklake_system WHERE key = 'latest_snapshot'")
    snapshot = cur.fetchone()[0]
    
    return {"epoch": epoch, "snapshot": snapshot}
```

### Migration Tooling

```python
def rename_schema_across_environments(old_name, new_name, environments):
    """Rename a schema consistently across dev, staging, prod."""
    for env in environments:
        conn = psycopg2.connect(host=env['host'], port=5432, dbname='rocklake')
        cur = conn.cursor()
        cur.execute(
            "UPDATE ducklake_schemas SET schema_name = %s WHERE schema_name = %s AND end_snapshot_id IS NULL",
            (new_name, old_name)
        )
        conn.commit()
        print(f"Renamed {old_name} -> {new_name} on {env['name']}")
        conn.close()
```

### CI/CD Integration

```python
def verify_table_exists(host, schema_name, table_name):
    """CI check: verify expected tables exist before deploying ETL."""
    conn = psycopg2.connect(host=host, port=5432, dbname='rocklake')
    cur = conn.cursor()
    
    # Find schema
    cur.execute(
        "SELECT schema_id FROM ducklake_schemas WHERE schema_name = %s AND end_snapshot_id IS NULL",
        (schema_name,)
    )
    row = cur.fetchone()
    if not row:
        raise AssertionError(f"Schema '{schema_name}' not found")
    
    # Find table
    cur.execute(
        "SELECT table_id FROM ducklake_tables WHERE schema_id = %s AND table_name = %s AND end_snapshot_id IS NULL",
        (row[0], table_name)
    )
    if not cur.fetchone():
        raise AssertionError(f"Table '{schema_name}.{table_name}' not found")
    
    print(f"✓ {schema_name}.{table_name} exists")
    conn.close()
```

### Administrative Dashboard

```python
from flask import Flask, jsonify
import psycopg2

app = Flask(__name__)

@app.route('/api/catalog/summary')
def catalog_summary():
    conn = get_connection()
    cur = conn.cursor()
    
    # Get counts
    cur.execute("SELECT count(*) FROM ducklake_schemas WHERE end_snapshot_id IS NULL")
    schema_count = cur.fetchone()[0]
    
    cur.execute("SELECT count(*) FROM ducklake_tables WHERE end_snapshot_id IS NULL")
    table_count = cur.fetchone()[0]
    
    return jsonify({
        'schemas': schema_count,
        'tables': table_count,
    })
```

## Connection Pooling

For high-frequency access, use connection pools:

### Python (psycopg2 pool)

```python
from psycopg2 import pool

connection_pool = pool.ThreadedConnectionPool(
    minconn=2,
    maxconn=10,
    host='localhost',
    port=5432,
    dbname='rocklake'
)

def query_catalog():
    conn = connection_pool.getconn()
    try:
        cur = conn.cursor()
        cur.execute("SELECT ...")
        return cur.fetchall()
    finally:
        connection_pool.putconn(conn)
```

### Go (pgxpool)

```go
pool, err := pgxpool.New(ctx, "postgres://localhost:5432/rocklake?pool_max_conns=10")
conn, err := pool.Acquire(ctx)
defer conn.Release()
```

## Error Handling

Rocklake returns standard SQLSTATE error codes:

| SQLSTATE | Meaning | Action |
|----------|---------|--------|
| 00000 | Success | — |
| 42601 | Syntax error (unrecognized SQL) | Fix the SQL pattern |
| 42P01 | Table not found | Check table/schema name |
| 57P04 | Writer fenced | Reconnect to new instance |
| 40001 | Transaction conflict | Retry |
| 08006 | Connection failure | Reconnect |

```python
try:
    cur.execute(sql)
except psycopg2.errors.SyntaxError:
    # SQL not recognized by bounded dispatcher
    print("This SQL pattern is not supported by Rocklake")
except psycopg2.errors.AdminShutdown:
    # Writer fenced
    reconnect()
```

## Best Practices for Production Clients

### Retry Logic

Network transients and writer failovers mean clients must implement retries:

```python
import time
from functools import wraps

def with_retry(max_attempts=3, backoff_base=0.1):
    def decorator(func):
        @wraps(func)
        def wrapper(*args, **kwargs):
            for attempt in range(max_attempts):
                try:
                    return func(*args, **kwargs)
                except (psycopg2.OperationalError, psycopg2.InterfaceError) as e:
                    if attempt == max_attempts - 1:
                        raise
                    sleep_time = backoff_base * (2 ** attempt)
                    time.sleep(sleep_time)
            return None
        return wrapper
    return decorator

@with_retry(max_attempts=3)
def get_table_count(conn):
    cur = conn.cursor()
    cur.execute("SELECT count(*) FROM ducklake_tables WHERE end_snapshot_id IS NULL")
    return cur.fetchone()[0]
```

### Connection Health Checks

Before using a pooled connection, verify it is still alive:

```python
def get_healthy_connection(pool):
    conn = pool.getconn()
    try:
        conn.cursor().execute("SELECT 1")
        return conn
    except Exception:
        pool.putconn(conn, close=True)
        return pool.getconn()
```

### Timeout Configuration

Set aggressive timeouts to avoid hanging on unresponsive instances:

```python
conn = psycopg2.connect(
    host="rocklake.internal",
    port=5432,
    dbname="rocklake",
    connect_timeout=5,         # 5 second connection timeout
    options="-c statement_timeout=10000"  # 10 second query timeout
)
```

### Logging and Observability

Log every catalog interaction for debugging:

```python
import logging

logger = logging.getLogger("rocklake-client")

def execute_with_logging(cur, sql, params=None):
    start = time.monotonic()
    try:
        cur.execute(sql, params)
        duration = time.monotonic() - start
        logger.info(f"catalog_query duration={duration:.3f}s sql={sql[:100]}")
        return cur
    except Exception as e:
        duration = time.monotonic() - start
        logger.error(f"catalog_query_error duration={duration:.3f}s error={e} sql={sql[:100]}")
        raise
```

## Further Reading

- **[DuckDB Compatibility](duckdb-compatibility.md)** — What SQL patterns are recognized
- **[Architecture: PG Wire Protocol](../architecture/pg-wire-protocol.md)** — Protocol implementation details
- **[pg-tide-relay](pg-tide-relay.md)** — Adding routing/auth in front of Rocklake
- **[Deployment: TLS](../deployment/tls.md)** — Securing connections
