# Extension Schema Registration

SlateDuck uses an extension schema allowlist to control which PostgreSQL schemas can perform extension DDL and DML operations (e.g., creating extension tables and inserting extension rows). Any operation targeting an unregistered schema is rejected with SQLSTATE **42501** (permission denied).

## Default Schema

The default allowed schema is `pgtrickle`. Existing deployments require no configuration change.

## Configuring the Allowed List

### CLI Flag

Pass a comma-separated list to `--extension-schemas`:

```sh
slateduck serve --extension-schemas pgtrickle,myextension
```

### Environment Variable

Set `SLATEDUCK_EXTENSION_SCHEMAS` before starting the server:

```sh
export SLATEDUCK_EXTENSION_SCHEMAS=pgtrickle,myextension
slateduck serve
```

The CLI flag takes precedence over the environment variable.

## Error Behavior

If a client attempts DDL or DML against an unregistered schema, SlateDuck returns:

```
ERROR:  permission denied for schema <schema_name> (SQLSTATE 42501)
```

This applies to:

- `CREATE TABLE` in an extension schema
- `INSERT INTO` an extension table
- `SELECT * FROM` an extension table
- `DELETE FROM` an extension table

## Security Notes

The allowlist is enforced at the SQL executor level before any catalog mutation occurs. It prevents both accidental and intentional writes to unrecognized extension namespaces. For the narrowest security profile, enumerate only the schemas your deployment actually uses.
