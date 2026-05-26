//! PG Wire protocol handler implementation.
//!
//! Implements SimpleQueryHandler and ExtendedQueryHandler for the pgwire crate.
//! Supports optional password authentication (cleartext with constant-time comparison).

use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::{BufMut, Bytes, BytesMut};
use futures::sink::{Sink, SinkExt};
use futures::StreamExt;
use pgwire::api::auth::{
    finish_authentication, save_startup_parameters_to_metadata, DefaultServerParameterProvider,
};
use pgwire::api::copy::CopyHandler;
use pgwire::api::portal::Portal;
use pgwire::api::query::{ExtendedQueryHandler, SimpleQueryHandler};
use pgwire::api::results::FieldInfo;
use pgwire::api::results::{DescribePortalResponse, DescribeStatementResponse, Response};
use pgwire::api::stmt::{QueryParser, StoredStatement};
use pgwire::api::store::PortalStore;
use pgwire::api::{
    ClientInfo, ClientPortalStore, NoopErrorHandler, PgWireConnectionState, PgWireServerHandlers,
    Type, METADATA_USER,
};
use pgwire::error::{ErrorInfo, PgWireError, PgWireResult};
use pgwire::messages::response::ErrorResponse;
use pgwire::messages::startup::Authentication;
use pgwire::messages::{copy::CopyOutResponse, PgWireBackendMessage, PgWireFrontendMessage};
use sqlparser::ast::{Expr, SelectItem, SetExpr, Statement};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use tokio::sync::Mutex;

use slateduck_catalog::CatalogStore;
use slateduck_core::rows::ColumnRow;
use slateduck_sql::{classify_statement, ParamValues, StatementKind};

use crate::copy_parser;
use crate::executor;
use crate::notify::NotifyManager;
use crate::server::AuthConfig;
use crate::session::{BootstrapSchemaRow, SessionState};

/// SlateDuck COPY handler: parses binary COPY FROM STDIN data for ducklake_*
/// tables and stores the bootstrap rows in the session for later commit.
#[derive(Clone)]
pub struct SlateDuckCopyHandler {
    session: Arc<Mutex<SessionState>>,
}

impl SlateDuckCopyHandler {
    pub fn new(session: Arc<Mutex<SessionState>>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl CopyHandler for SlateDuckCopyHandler {
    async fn on_copy_data<C>(
        &self,
        _client: &mut C,
        copy_data: pgwire::messages::copy::CopyData,
    ) -> PgWireResult<()>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        // Append incoming bytes to the accumulator for the active COPY table.
        let mut session = self.session.lock().await;
        if let Some(acc) = &mut session.pending_copy {
            acc.data.extend_from_slice(&copy_data.data);
        }
        Ok(())
    }

    async fn on_copy_done<C>(
        &self,
        client: &mut C,
        _done: pgwire::messages::copy::CopyDone,
    ) -> PgWireResult<()>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        // Drain the accumulator outside the lock to avoid holding it during parsing.
        let (table, data) = {
            let mut session = self.session.lock().await;
            match session.pending_copy.take() {
                Some(acc) => (acc.table, acc.data),
                None => {
                    // No active COPY — still send CommandComplete.
                    return send_copy_done(client, 0).await;
                }
            }
        };

        // Parse the binary COPY stream and extract bootstrap rows.
        let rows = copy_parser::parse_binary_copy_rows(&data);
        let row_count = rows.len();

        {
            let mut session = self.session.lock().await;
            match table.as_str() {
                "ducklake_snapshot" if !rows.is_empty() => {
                    // Any row means DuckDB has initialised a snapshot.
                    session.bootstrap.has_snapshot = true;
                }
                "ducklake_schema" => {
                    for row in &rows {
                        // ducklake_schema column order:
                        //   0: schema_id (BIGINT)
                        //   1: schema_uuid (UUID)
                        //   2: begin_snapshot (BIGINT)
                        //   3: end_snapshot (BIGINT, nullable)
                        //   4: schema_name (VARCHAR)  ← what we need
                        //   5: path (VARCHAR, nullable)
                        //   6: path_is_relative (BOOLEAN, nullable)
                        if let Some(name) = copy_parser::extract_varchar(row, 4) {
                            session
                                .bootstrap
                                .schemas
                                .push(BootstrapSchemaRow { schema_name: name });
                        }
                    }
                }
                // ducklake_snapshot_changes and ducklake_metadata are accepted
                // but not persisted; they don't affect catalog bootstrap state.
                _ => {}
            }
        }

        send_copy_done(client, row_count).await
    }
}

/// Send `CommandComplete "COPY N"` to the client.
async fn send_copy_done<C>(client: &mut C, rows: usize) -> PgWireResult<()>
where
    C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
    C::Error: Debug,
    PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
{
    use pgwire::messages::response::CommandComplete;
    client
        .send(PgWireBackendMessage::CommandComplete(CommandComplete::new(
            format!("COPY {rows}"),
        )))
        .await?;
    Ok(())
}

/// The main SlateDuck query handler.
pub struct SlateDuckHandler {
    pub catalog: Arc<Mutex<CatalogStore>>,
    pub session: Arc<Mutex<SessionState>>,
    pub parser: Arc<SlateDuckQueryParser>,
    pub auth: Arc<AuthConfig>,
    /// Shared LISTEN/NOTIFY manager for this server instance.
    pub notify_manager: Arc<NotifyManager>,
    /// Allowed extension schema names (configurable via --extension-schemas).
    pub extension_schemas: Arc<Vec<String>>,
}

impl SlateDuckHandler {
    pub fn new(catalog: Arc<Mutex<CatalogStore>>) -> Self {
        Self {
            catalog,
            session: Arc::new(Mutex::new(SessionState::new())),
            parser: Arc::new(SlateDuckQueryParser),
            auth: Arc::new(AuthConfig::default()),
            notify_manager: Arc::new(NotifyManager::new()),
            extension_schemas: Arc::new(vec!["pgtrickle".to_string()]),
        }
    }

    pub fn new_with_auth(catalog: Arc<Mutex<CatalogStore>>, auth: Arc<AuthConfig>) -> Self {
        Self {
            catalog,
            session: Arc::new(Mutex::new(SessionState::new())),
            parser: Arc::new(SlateDuckQueryParser),
            auth,
            notify_manager: Arc::new(NotifyManager::new()),
            extension_schemas: Arc::new(vec!["pgtrickle".to_string()]),
        }
    }

    pub fn new_with_config(
        catalog: Arc<Mutex<CatalogStore>>,
        auth: Arc<AuthConfig>,
        notify_manager: Arc<NotifyManager>,
        extension_schemas: Arc<Vec<String>>,
    ) -> Self {
        Self {
            catalog,
            session: Arc::new(Mutex::new(SessionState::new())),
            parser: Arc::new(SlateDuckQueryParser),
            auth,
            notify_manager,
            extension_schemas,
        }
    }

    /// If `sql` is a `COPY (SELECT ...) TO STDOUT`, execute the inner SELECT
    /// and stream PostgreSQL binary COPY frames directly to the client.
    async fn try_stream_copy_to_stdout<C>(
        &self,
        client: &mut C,
        sql: &str,
    ) -> PgWireResult<Option<usize>>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        let StatementKind::CopyToStdout { query } =
            classify_statement(sql).unwrap_or(StatementKind::Unsupported(String::new()))
        else {
            return Ok(None);
        };

        let params = ParamValues::default();
        let mut session = self.session.lock().await;
        let mut responses = executor::execute_sql(
            &query,
            &params,
            &self.catalog,
            &mut session,
            &self.notify_manager,
            &self.extension_schemas,
        )
        .await
        .map_err(|e| -> PgWireError { e.into() })?;

        let response = responses.pop().ok_or_else(|| {
            PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".to_string(),
                "XX000".to_string(),
                "COPY TO STDOUT inner query returned no response".to_string(),
            )))
        })?;

        let pgwire::api::results::Response::Query(query_response) = response else {
            return Err(PgWireError::UserError(Box::new(ErrorInfo::new(
                "ERROR".to_string(),
                "XX000".to_string(),
                "COPY TO STDOUT inner statement must return rows".to_string(),
            ))));
        };

        let row_schema = query_response.row_schema();
        let projected_indices = projected_copy_indices(&query, row_schema.as_ref());
        let columns = projected_indices.len();
        client
            .send(PgWireBackendMessage::CopyOutResponse(CopyOutResponse::new(
                1,
                columns as i16,
                vec![1; columns],
            )))
            .await?;

        // Build one contiguous binary COPY payload (header + rows + trailer).
        // Some clients are strict about frame boundaries during binary COPY reads.
        let mut payload = BytesMut::new();
        payload.extend_from_slice(binary_copy_header().as_ref());

        let mut row_count = 0usize;
        let mut rows = query_response.data_rows();
        while let Some(row_result) = rows.next().await {
            let row = row_result?;
            let projected_row = project_copy_row_data(
                &row.data,
                row.field_count as usize,
                &projected_indices,
                row_schema.as_ref(),
            )?;
            payload.put_i16(columns as i16);
            payload.put_slice(&projected_row);
            row_count += 1;
        }

        // End-of-copy marker: int16 -1.
        payload.put_i16(-1);

        client
            .send(PgWireBackendMessage::CopyData(
                pgwire::messages::copy::CopyData::new(payload.freeze()),
            ))
            .await?;

        client
            .send(PgWireBackendMessage::CopyDone(
                pgwire::messages::copy::CopyDone::new(),
            ))
            .await?;

        Ok(Some(row_count))
    }
}

fn binary_copy_header() -> Bytes {
    // Signature + flags (0) + header extension length (0).
    const SIGNATURE: &[u8] = b"PGCOPY\n\xff\r\n\0";
    let mut buf = BytesMut::with_capacity(SIGNATURE.len() + 8);
    buf.put_slice(SIGNATURE);
    buf.put_i32(0);
    buf.put_i32(0);
    buf.freeze()
}

fn projected_copy_indices(sql: &str, schema: &[FieldInfo]) -> Vec<usize> {
    projection_names(sql)
        .and_then(|names| {
            let schema_names = schema
                .iter()
                .map(|field| field.name().to_lowercase())
                .collect::<Vec<_>>();
            names
                .iter()
                .map(|name| schema_names.iter().position(|field| field == name))
                .collect::<Option<Vec<_>>>()
        })
        .filter(|indices| !indices.is_empty())
        .unwrap_or_else(|| (0..schema.len()).collect())
}

fn projection_names(sql: &str) -> Option<Vec<String>> {
    let dialect = PostgreSqlDialect {};
    let mut statements = Parser::parse_sql(&dialect, sql).ok()?;
    let statement = statements.pop()?;
    let Statement::Query(query) = statement else {
        return None;
    };
    let SetExpr::Select(select) = query.body.as_ref() else {
        return None;
    };

    let mut names = Vec::new();
    for item in &select.projection {
        let name = projection_item_name(item)?;
        if name == "*" {
            return None;
        }
        names.push(name);
    }
    Some(names)
}

fn projection_item_name(item: &SelectItem) -> Option<String> {
    match item {
        SelectItem::UnnamedExpr(expr) => Some(expr_last_identifier(expr)),
        SelectItem::ExprWithAlias { alias, .. } => Some(alias.value.to_lowercase()),
        SelectItem::QualifiedWildcard(_, _) | SelectItem::Wildcard(_) => Some("*".to_string()),
    }
}

fn expr_last_identifier(expr: &Expr) -> String {
    match expr {
        Expr::Identifier(id) => id.value.to_lowercase(),
        Expr::CompoundIdentifier(parts) => parts
            .last()
            .map(|id| id.value.to_lowercase())
            .unwrap_or_default(),
        Expr::Cast { expr, .. } | Expr::Nested(expr) => expr_last_identifier(expr),
        _ => expr.to_string().to_lowercase(),
    }
}

fn project_copy_row_data(
    row_data: &BytesMut,
    field_count: usize,
    projected_indices: &[usize],
    schema: &[FieldInfo],
) -> PgWireResult<BytesMut> {
    let fields = split_copy_row_fields(row_data, field_count)?;
    let mut projected = BytesMut::new();
    for &index in projected_indices {
        let Some(field) = fields.get(index) else {
            return Err(copy_out_error(
                "COPY TO STDOUT projection index out of range",
            ));
        };
        match field {
            Some(value) => {
                let datatype = schema
                    .get(index)
                    .map(|field| field.datatype())
                    .ok_or_else(|| copy_out_error("COPY TO STDOUT schema index out of range"))?;
                let value = encode_binary_copy_field(value, datatype)?;
                projected.put_i32(value.len() as i32);
                projected.put_slice(&value);
            }
            None => projected.put_i32(-1),
        }
    }
    Ok(projected)
}

fn encode_binary_copy_field(value: &[u8], datatype: &Type) -> PgWireResult<Vec<u8>> {
    if datatype == &Type::UUID {
        if value.len() == 16 {
            return Ok(value.to_vec());
        }
        let uuid_text = std::str::from_utf8(value)
            .map_err(|_| copy_out_error("COPY TO STDOUT UUID field is not valid UTF-8"))?;
        let uuid = uuid::Uuid::parse_str(uuid_text)
            .map_err(|_| copy_out_error("COPY TO STDOUT UUID field is invalid"))?;
        return Ok(uuid.as_bytes().to_vec());
    }

    if datatype == &Type::INT8 {
        if value.len() == 8 {
            return Ok(value.to_vec());
        }
        let int_text = std::str::from_utf8(value)
            .map_err(|_| copy_out_error("COPY TO STDOUT INT8 field is not valid UTF-8"))?;
        let value = int_text
            .parse::<i64>()
            .map_err(|_| copy_out_error("COPY TO STDOUT INT8 field is invalid"))?;
        return Ok(value.to_be_bytes().to_vec());
    }

    if datatype == &Type::INT4 {
        if value.len() == 4 {
            return Ok(value.to_vec());
        }
        let int_text = std::str::from_utf8(value)
            .map_err(|_| copy_out_error("COPY TO STDOUT INT4 field is not valid UTF-8"))?;
        let value = int_text
            .parse::<i32>()
            .map_err(|_| copy_out_error("COPY TO STDOUT INT4 field is invalid"))?;
        return Ok(value.to_be_bytes().to_vec());
    }

    if datatype == &Type::INT2 {
        if value.len() == 2 {
            return Ok(value.to_vec());
        }
        let int_text = std::str::from_utf8(value)
            .map_err(|_| copy_out_error("COPY TO STDOUT INT2 field is not valid UTF-8"))?;
        let value = int_text
            .parse::<i16>()
            .map_err(|_| copy_out_error("COPY TO STDOUT INT2 field is invalid"))?;
        return Ok(value.to_be_bytes().to_vec());
    }

    if datatype == &Type::BOOL {
        if value.len() == 1 && (value[0] == 0 || value[0] == 1) {
            return Ok(value.to_vec());
        }
        let bool_text = std::str::from_utf8(value)
            .map_err(|_| copy_out_error("COPY TO STDOUT boolean field is not valid UTF-8"))?
            .to_ascii_lowercase();
        return match bool_text.as_str() {
            "true" | "t" | "1" => Ok(vec![1]),
            "false" | "f" | "0" => Ok(vec![0]),
            _ => Err(copy_out_error("COPY TO STDOUT boolean field is invalid")),
        };
    }

    Ok(value.to_vec())
}

fn split_copy_row_fields(
    row_data: &BytesMut,
    field_count: usize,
) -> PgWireResult<Vec<Option<Vec<u8>>>> {
    let mut offset = 0usize;
    let mut fields = Vec::with_capacity(field_count);
    for _ in 0..field_count {
        if row_data.len().saturating_sub(offset) < 4 {
            return Err(copy_out_error(
                "COPY TO STDOUT row field length is truncated",
            ));
        }
        let len = i32::from_be_bytes(
            row_data[offset..offset + 4]
                .try_into()
                .expect("slice length checked"),
        );
        offset += 4;
        if len == -1 {
            fields.push(None);
            continue;
        }
        if len < 0 {
            return Err(copy_out_error(
                "COPY TO STDOUT row contains invalid field length",
            ));
        }
        let len = len as usize;
        if row_data.len().saturating_sub(offset) < len {
            return Err(copy_out_error("COPY TO STDOUT row field data is truncated"));
        }
        fields.push(Some(row_data[offset..offset + len].to_vec()));
        offset += len;
    }
    if offset != row_data.len() {
        return Err(copy_out_error("COPY TO STDOUT row has trailing bytes"));
    }
    Ok(fields)
}

fn copy_out_error(message: &str) -> PgWireError {
    PgWireError::UserError(Box::new(ErrorInfo::new(
        "ERROR".to_string(),
        "XX000".to_string(),
        message.to_string(),
    )))
}

/// Constant-time byte slice equality comparison to resist timing attacks.
fn ct_bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Startup handler that enforces authentication when configured.
///
/// When `AuthConfig::is_enabled()` returns false, connections are accepted
/// without any credential check (noop). When it returns true, cleartext
/// password auth is required; username is verified before issuing the
/// password challenge, and the password is compared in constant time.
///
/// When `tls_required` is true and the client connects without TLS, the
/// connection is rejected immediately with a fatal error.
pub struct SlateDuckStartupHandler {
    auth: Arc<AuthConfig>,
    tls_required: bool,
}

impl SlateDuckStartupHandler {
    pub fn new(auth: Arc<AuthConfig>) -> Self {
        Self {
            auth,
            tls_required: false,
        }
    }

    pub fn new_with_tls_required(auth: Arc<AuthConfig>, tls_required: bool) -> Self {
        Self { auth, tls_required }
    }
}

#[async_trait]
impl pgwire::api::auth::StartupHandler for SlateDuckStartupHandler {
    async fn on_startup<C>(
        &self,
        client: &mut C,
        message: PgWireFrontendMessage,
    ) -> PgWireResult<()>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        match message {
            PgWireFrontendMessage::Startup(ref startup) => {
                // Reject plaintext connections when TLS is required.
                if self.tls_required && !client.is_secure() {
                    let error_info = ErrorInfo::new(
                        "FATAL".to_owned(),
                        "28000".to_owned(),
                        "SSL connection is required. Connect using SSL/TLS.".to_owned(),
                    );
                    client
                        .feed(PgWireBackendMessage::ErrorResponse(ErrorResponse::from(
                            error_info,
                        )))
                        .await?;
                    client.close().await?;
                    return Ok(());
                }
                save_startup_parameters_to_metadata(client, startup);
                if !self.auth.is_enabled() {
                    finish_authentication(client, &DefaultServerParameterProvider::default())
                        .await?;
                } else {
                    let expected_user = self.auth.username.as_deref().unwrap_or("").to_owned();
                    let provided_user = client
                        .metadata()
                        .get(METADATA_USER)
                        .cloned()
                        .unwrap_or_default();
                    if provided_user != expected_user {
                        let error_info = ErrorInfo::new(
                            "FATAL".to_owned(),
                            "28P01".to_owned(),
                            format!("Password authentication failed for user \"{provided_user}\""),
                        );
                        client
                            .feed(PgWireBackendMessage::ErrorResponse(ErrorResponse::from(
                                error_info,
                            )))
                            .await?;
                        client.close().await?;
                        return Ok(());
                    }
                    client.set_state(PgWireConnectionState::AuthenticationInProgress);
                    client
                        .send(PgWireBackendMessage::Authentication(
                            Authentication::CleartextPassword,
                        ))
                        .await?;
                }
            }
            PgWireFrontendMessage::PasswordMessageFamily(pwd) if self.auth.is_enabled() => {
                let pwd = pwd.into_password()?;
                let expected = self.auth.password.as_deref().unwrap_or("").as_bytes();
                if ct_bytes_eq(pwd.password.as_bytes(), expected) {
                    finish_authentication(client, &DefaultServerParameterProvider::default())
                        .await?;
                } else {
                    let error_info = ErrorInfo::new(
                        "FATAL".to_owned(),
                        "28P01".to_owned(),
                        "Password authentication failed".to_owned(),
                    );
                    client
                        .feed(PgWireBackendMessage::ErrorResponse(ErrorResponse::from(
                            error_info,
                        )))
                        .await?;
                    client.close().await?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[async_trait]
impl SimpleQueryHandler for SlateDuckHandler {
    async fn do_query<'a, 'b: 'a, C>(
        &'b self,
        _client: &mut C,
        query: &'a str,
    ) -> PgWireResult<Vec<Response<'a>>>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        if let Some(rows) = self.try_stream_copy_to_stdout(_client, query).await? {
            return Ok(vec![Response::Execution(
                pgwire::api::results::Tag::new("COPY").with_rows(rows),
            )]);
        }

        let params = ParamValues::default();
        let mut session = self.session.lock().await;
        match executor::execute_sql(
            query,
            &params,
            &self.catalog,
            &mut session,
            &self.notify_manager,
            &self.extension_schemas,
        )
        .await
        {
            Ok(responses) => Ok(responses),
            Err(e) => Err(e.into()),
        }
    }
}

/// Query parser that stores SQL strings.
#[derive(Debug, Clone)]
pub struct SlateDuckQueryParser;

#[async_trait]
impl QueryParser for SlateDuckQueryParser {
    type Statement = String;

    async fn parse_sql(&self, sql: &str, _types: &[Type]) -> PgWireResult<Self::Statement> {
        Ok(sql.to_owned())
    }
}

#[async_trait]
impl ExtendedQueryHandler for SlateDuckHandler {
    type Statement = String;
    type QueryParser = SlateDuckQueryParser;

    fn query_parser(&self) -> Arc<Self::QueryParser> {
        self.parser.clone()
    }

    async fn do_query<'a, 'b: 'a, C>(
        &'b self,
        _client: &mut C,
        portal: &'a Portal<Self::Statement>,
        _max_rows: usize,
    ) -> PgWireResult<Response<'a>>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore<Statement = Self::Statement>,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        let sql = &portal.statement.statement;

        if let Some(rows) = self.try_stream_copy_to_stdout(_client, sql).await? {
            return Ok(Response::Execution(
                pgwire::api::results::Tag::new("COPY").with_rows(rows),
            ));
        }

        // Extract parameters from portal, handling binary-encoded integers.
        // tokio-postgres (and DuckDB) always send parameters in binary format;
        // for integer types we must decode the big-endian bytes to decimal
        // strings so that the string-based ParamValues can parse them.
        //
        // The stored `portal.statement.parameter_types` reflects what the client
        // declared in its Parse message (usually UNKNOWN because tokio-postgres
        // relies on DescribeStatement to learn types). We use `describe_params_for_sql`
        // as an authoritative fallback so binary INT8 bytes are always decoded correctly.
        let inferred_types = describe_params_for_sql(sql);
        let param_values: Vec<Option<String>> = portal
            .parameters
            .iter()
            .enumerate()
            .map(|(i, p)| {
                p.as_ref().map(|b| {
                    if portal.parameter_format.is_binary(i) {
                        // Prefer a non-UNKNOWN stored type; fall back to inferred type.
                        let pg_type = portal
                            .statement
                            .parameter_types
                            .get(i)
                            .filter(|t| **t != Type::UNKNOWN)
                            .cloned()
                            .or_else(|| inferred_types.get(i).cloned())
                            .unwrap_or(Type::UNKNOWN);
                        match pg_type {
                            Type::INT8 if b.len() == 8 => {
                                let bytes: [u8; 8] = b[..8].try_into().unwrap_or([0; 8]);
                                return i64::from_be_bytes(bytes).to_string();
                            }
                            Type::INT4 if b.len() == 4 => {
                                let bytes: [u8; 4] = b[..4].try_into().unwrap_or([0; 4]);
                                return i32::from_be_bytes(bytes).to_string();
                            }
                            Type::INT2 if b.len() == 2 => {
                                let bytes: [u8; 2] = b[..2].try_into().unwrap_or([0; 2]);
                                return i16::from_be_bytes(bytes).to_string();
                            }
                            _ => {}
                        }
                    }
                    String::from_utf8_lossy(b).to_string()
                })
            })
            .collect();
        let params = ParamValues::new(param_values);

        let mut session = self.session.lock().await;
        match executor::execute_sql(
            sql,
            &params,
            &self.catalog,
            &mut session,
            &self.notify_manager,
            &self.extension_schemas,
        )
        .await
        {
            Ok(mut responses) => {
                if let Some(resp) = responses.pop() {
                    Ok(resp)
                } else {
                    Ok(Response::EmptyQuery)
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn do_describe_statement<C>(
        &self,
        _client: &mut C,
        stmt: &StoredStatement<Self::Statement>,
    ) -> PgWireResult<DescribeStatementResponse>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore<Statement = Self::Statement>,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        let sql = &stmt.statement;
        let fields = describe_fields_for_sql_with_catalog(sql, &self.catalog).await;

        // Return precise parameter types so the client can correctly serialize
        // typed values (e.g. i64 → INT8). When the client provided type hints in
        // its Parse message we respect those; otherwise we infer from the
        // StatementKind.
        let param_types = if !stmt.parameter_types.is_empty()
            && stmt.parameter_types.iter().any(|t| *t != Type::UNKNOWN)
        {
            stmt.parameter_types.clone()
        } else {
            describe_params_for_sql(sql)
        };

        Ok(DescribeStatementResponse::new(param_types, fields))
    }

    async fn do_describe_portal<C>(
        &self,
        _client: &mut C,
        portal: &Portal<Self::Statement>,
    ) -> PgWireResult<DescribePortalResponse>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore<Statement = Self::Statement>,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        let sql = &portal.statement.statement;
        let fields = describe_fields_for_sql_with_catalog(sql, &self.catalog).await;
        Ok(DescribePortalResponse::new(fields))
    }
}

/// Server handlers collection for SlateDuck.
pub struct SlateDuckServerHandlers {
    pub handler: Arc<SlateDuckHandler>,
    pub startup: Arc<SlateDuckStartupHandler>,
    pub copy_handler: Arc<SlateDuckCopyHandler>,
    pub error_handler: Arc<NoopErrorHandler>,
}

impl SlateDuckServerHandlers {
    pub fn new(catalog: Arc<Mutex<CatalogStore>>) -> Self {
        let auth = Arc::new(AuthConfig::default());
        let handler = Arc::new(SlateDuckHandler::new(catalog));
        let copy_handler = Arc::new(SlateDuckCopyHandler::new(handler.session.clone()));
        Self {
            handler,
            startup: Arc::new(SlateDuckStartupHandler::new(auth)),
            copy_handler,
            error_handler: Arc::new(NoopErrorHandler),
        }
    }

    pub fn new_with_auth(catalog: Arc<Mutex<CatalogStore>>, auth: Arc<AuthConfig>) -> Self {
        let handler = Arc::new(SlateDuckHandler::new_with_auth(catalog, auth.clone()));
        let copy_handler = Arc::new(SlateDuckCopyHandler::new(handler.session.clone()));
        Self {
            handler,
            startup: Arc::new(SlateDuckStartupHandler::new(auth)),
            copy_handler,
            error_handler: Arc::new(NoopErrorHandler),
        }
    }

    pub fn new_with_config(
        catalog: Arc<Mutex<CatalogStore>>,
        auth: Arc<AuthConfig>,
        tls_required: bool,
        notify_manager: Arc<NotifyManager>,
        extension_schemas: Arc<Vec<String>>,
    ) -> Self {
        let handler = Arc::new(SlateDuckHandler::new_with_config(
            catalog,
            auth.clone(),
            notify_manager,
            extension_schemas,
        ));
        let copy_handler = Arc::new(SlateDuckCopyHandler::new(handler.session.clone()));
        Self {
            handler,
            startup: Arc::new(SlateDuckStartupHandler::new_with_tls_required(
                auth,
                tls_required,
            )),
            copy_handler,
            error_handler: Arc::new(NoopErrorHandler),
        }
    }
}

impl PgWireServerHandlers for SlateDuckServerHandlers {
    type StartupHandler = SlateDuckStartupHandler;
    type SimpleQueryHandler = SlateDuckHandler;
    type ExtendedQueryHandler = SlateDuckHandler;
    type CopyHandler = SlateDuckCopyHandler;
    type ErrorHandler = NoopErrorHandler;

    fn simple_query_handler(&self) -> Arc<Self::SimpleQueryHandler> {
        self.handler.clone()
    }

    fn extended_query_handler(&self) -> Arc<Self::ExtendedQueryHandler> {
        self.handler.clone()
    }

    fn startup_handler(&self) -> Arc<Self::StartupHandler> {
        self.startup.clone()
    }

    fn copy_handler(&self) -> Arc<Self::CopyHandler> {
        self.copy_handler.clone()
    }

    fn error_handler(&self) -> Arc<Self::ErrorHandler> {
        self.error_handler.clone()
    }
}

/// Count the number of positional parameters (`$1`, `$2`, …) in a SQL string.
/// Returns the highest parameter index found, which equals the number of
/// parameters the client must bind.
fn count_sql_params(sql: &str) -> usize {
    let mut max = 0usize;
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > start {
                if let Ok(n) = sql[start..end].parse::<usize>() {
                    if n > max {
                        max = n;
                    }
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    max
}

/// Return the expected parameter types for a SQL statement.
/// Allows tokio-postgres to correctly serialize typed Rust values (e.g. i64→INT8)
/// even when the client sends no type hints in the Parse message.
fn describe_params_for_sql(sql: &str) -> Vec<Type> {
    use slateduck_sql::StatementKind;
    let kind =
        slateduck_sql::classify_statement(sql).unwrap_or(StatementKind::Unsupported(String::new()));
    match kind {
        // Snapshot-scoped catalog selects: $1 = snapshot_id (INT8)
        StatementKind::SelectSchemas
        | StatementKind::SelectTables
        | StatementKind::SelectDataFiles
        | StatementKind::SelectMaxSnapshotAfter => vec![Type::INT8],
        // Inserts whose first column is a numeric FK
        StatementKind::InsertTable => vec![Type::INT8, Type::TEXT, Type::TEXT],
        StatementKind::InsertDataFile => {
            // table_id, path, format, row_count, file_size_bytes
            vec![Type::INT8, Type::TEXT, Type::TEXT, Type::INT8, Type::INT8]
        }
        // Text-only inserts
        StatementKind::InsertSchema => vec![Type::TEXT],
        StatementKind::InsertSnapshot => vec![Type::TEXT, Type::TEXT],
        // table_changes(table_name TEXT, from_snapshot INT8, to_snapshot INT8)
        StatementKind::TableChanges { .. } => vec![Type::TEXT, Type::INT8, Type::INT8],
        // Everything else: fall back to UNKNOWN (works for &str / String)
        _ => {
            let count = count_sql_params(sql);
            vec![Type::UNKNOWN; count]
        }
    }
}

/// Return the result-set field descriptions for a SQL statement.
/// Used by both `do_describe_statement` and `do_describe_portal`.
async fn describe_fields_for_sql_with_catalog(
    sql: &str,
    catalog: &Arc<Mutex<CatalogStore>>,
) -> Vec<pgwire::api::results::FieldInfo> {
    let kind = slateduck_sql::classify_statement(sql)
        .unwrap_or(slateduck_sql::StatementKind::Unsupported(String::new()));
    if matches!(kind, slateduck_sql::StatementKind::SelectInlinedRows) {
        if let Some((table_id, _schema_version)) = parse_inlined_table_ids_from_sql(sql) {
            let reader = { catalog.lock().await.read_latest() };
            if let Ok(Some((_, columns))) = reader.describe_table(table_id).await {
                return describe_inlined_row_fields(sql, &columns);
            }
        }
    }
    describe_fields_for_sql(sql)
}

fn describe_fields_for_sql(sql: &str) -> Vec<pgwire::api::results::FieldInfo> {
    use pgwire::api::results::{FieldFormat, FieldInfo};

    let kind = slateduck_sql::classify_statement(sql)
        .unwrap_or(slateduck_sql::StatementKind::Unsupported(String::new()));

    /// Quick helper to build a FieldInfo for text-type metadata columns.
    macro_rules! text_col {
        ($name:expr) => {
            FieldInfo::new($name.to_string(), None, None, Type::TEXT, FieldFormat::Text)
        };
    }
    macro_rules! int8_col {
        ($name:expr) => {
            FieldInfo::new(
                $name.to_string(),
                None,
                None,
                Type::INT8,
                FieldFormat::Binary,
            )
        };
    }
    macro_rules! int8_text_col {
        ($name:expr) => {
            FieldInfo::new($name.to_string(), None, None, Type::INT8, FieldFormat::Text)
        };
    }
    macro_rules! bool_col {
        ($name:expr) => {
            FieldInfo::new($name.to_string(), None, None, Type::BOOL, FieldFormat::Text)
        };
    }

    match kind {
        slateduck_sql::StatementKind::SelectVersion => vec![text_col!("version")],
        slateduck_sql::StatementKind::SelectCurrentSchema => vec![text_col!("current_schema")],
        slateduck_sql::StatementKind::SelectCurrentDatabase => {
            vec![text_col!("current_database")]
        }
        slateduck_sql::StatementKind::SelectPgType => vec![
            FieldInfo::new("oid".to_string(), None, None, Type::INT4, FieldFormat::Text),
            text_col!("typname"),
        ],
        slateduck_sql::StatementKind::SelectMaxSnapshot
        | slateduck_sql::StatementKind::SelectMaxSnapshotAfter => {
            vec![int8_col!("max")]
        }
        slateduck_sql::StatementKind::SelectLatestSnapshotInfo => vec![
            int8_col!("snapshot_id"),
            int8_col!("schema_version"),
            int8_col!("next_catalog_id"),
            int8_col!("next_file_id"),
        ],
        slateduck_sql::StatementKind::ShowVariable(ref var) => {
            vec![text_col!(var.as_str())]
        }
        // Catalog table schemas — must match the executor's make_*_response column lists.
        slateduck_sql::StatementKind::SelectSchemas => project_described_fields(
            sql,
            vec![
                int8_col!("schema_id"),
                int8_col!("begin_snapshot"),
                int8_col!("end_snapshot"),
                text_col!("schema_uuid"),
                text_col!("schema_name"),
                text_col!("path"),
                bool_col!("path_is_relative"),
            ],
        ),
        slateduck_sql::StatementKind::SelectTables => project_described_fields(
            sql,
            vec![
                int8_col!("table_id"),
                int8_col!("begin_snapshot"),
                int8_col!("end_snapshot"),
                int8_col!("schema_id"),
                text_col!("table_name"),
                text_col!("table_uuid"),
                text_col!("path"),
                bool_col!("path_is_relative"),
            ],
        ),
        slateduck_sql::StatementKind::SelectColumns => project_described_fields(
            sql,
            vec![
                int8_text_col!("column_id"),
                int8_text_col!("begin_snapshot"),
                int8_text_col!("end_snapshot"),
                int8_text_col!("table_id"),
                int8_text_col!("column_order"),
                text_col!("column_name"),
                text_col!("column_type"),
                text_col!("initial_default"),
                text_col!("default_value"),
                bool_col!("nulls_allowed"),
                int8_text_col!("parent_column"),
                text_col!("default_value_type"),
                text_col!("default_value_dialect"),
            ],
        ),
        slateduck_sql::StatementKind::SelectDataFiles => project_described_fields(
            sql,
            vec![
                int8_text_col!("data_file_id"),
                int8_text_col!("table_id"),
                int8_text_col!("begin_snapshot"),
                int8_text_col!("end_snapshot"),
                int8_text_col!("file_order"),
                text_col!("path"),
                bool_col!("path_is_relative"),
                text_col!("file_format"),
                int8_text_col!("record_count"),
                int8_text_col!("file_size_bytes"),
                int8_text_col!("row_id_start"),
            ],
        ),
        slateduck_sql::StatementKind::SelectFileColumnStats => project_described_fields(
            sql,
            vec![
                int8_text_col!("data_file_id"),
                int8_text_col!("table_id"),
                int8_text_col!("column_id"),
                int8_text_col!("column_size_bytes"),
                int8_text_col!("value_count"),
                int8_text_col!("null_count"),
                text_col!("min_value"),
                text_col!("max_value"),
                bool_col!("contains_nan"),
                text_col!("extra_stats"),
            ],
        ),
        slateduck_sql::StatementKind::SelectTableStats
            if sql
                .to_ascii_lowercase()
                .contains("ducklake_table_column_stats") =>
        {
            project_described_fields(
                sql,
                vec![
                    int8_text_col!("table_id"),
                    int8_text_col!("column_id"),
                    int8_text_col!("record_count"),
                    int8_text_col!("next_row_id"),
                    int8_text_col!("file_size_bytes"),
                    bool_col!("contains_null"),
                    bool_col!("contains_nan"),
                    text_col!("min_value"),
                    text_col!("max_value"),
                    text_col!("extra_stats"),
                ],
            )
        }
        slateduck_sql::StatementKind::SelectTableStats => project_described_fields(
            sql,
            vec![
                int8_text_col!("table_id"),
                int8_text_col!("record_count"),
                int8_text_col!("next_row_id"),
                int8_text_col!("file_size_bytes"),
            ],
        ),
        slateduck_sql::StatementKind::SelectTableColumnStats => project_described_fields(
            sql,
            vec![
                int8_text_col!("table_id"),
                int8_text_col!("column_id"),
                bool_col!("contains_null"),
                bool_col!("contains_nan"),
                text_col!("min_value"),
                text_col!("max_value"),
                text_col!("extra_stats"),
            ],
        ),
        slateduck_sql::StatementKind::SelectInlinedData => project_described_fields(
            sql,
            vec![
                int8_text_col!("table_id"),
                text_col!("table_name"),
                int8_text_col!("schema_version"),
            ],
        ),
        _ => vec![],
    }
}

fn parse_inlined_table_ids_from_sql(sql: &str) -> Option<(u64, u64)> {
    let lower = sql.to_ascii_lowercase();
    let start = lower.find("ducklake_inlined_data_")?;
    let rest = &lower[start + "ducklake_inlined_data_".len()..];
    let mut parts = rest
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .next()?
        .split('_');
    let table_id = parts.next()?.parse().ok()?;
    let schema_version = parts.next()?.parse().ok()?;
    Some((table_id, schema_version))
}

fn describe_inlined_row_fields(
    sql: &str,
    columns: &[ColumnRow],
) -> Vec<pgwire::api::results::FieldInfo> {
    use pgwire::api::results::{FieldFormat, FieldInfo};

    let field_for_name = |name: &str| {
        let lower = name.trim_matches('"').to_ascii_lowercase();
        match lower.as_str() {
            "row_id" => Some(FieldInfo::new(
                name.to_string(),
                None,
                None,
                Type::INT8,
                FieldFormat::Binary,
            )),
            "begin_snapshot" => Some(FieldInfo::new(
                name.to_string(),
                None,
                None,
                Type::INT8,
                FieldFormat::Binary,
            )),
            "end_snapshot" => Some(FieldInfo::new(
                name.to_string(),
                None,
                None,
                Type::INT8,
                FieldFormat::Binary,
            )),
            _ => columns
                .iter()
                .find(|column| column.column_name.eq_ignore_ascii_case(&lower))
                .map(|column| {
                    FieldInfo::new(
                        name.to_string(),
                        None,
                        None,
                        inlined_storage_type(&column.data_type),
                        FieldFormat::Binary,
                    )
                }),
        }
    };

    let Some(names) = projection_names(sql) else {
        return columns
            .iter()
            .map(|column| {
                FieldInfo::new(
                    column.column_name.clone(),
                    None,
                    None,
                    inlined_storage_type(&column.data_type),
                    FieldFormat::Binary,
                )
            })
            .collect();
    };
    let fields = names
        .iter()
        .filter_map(|name| field_for_name(name))
        .collect::<Vec<_>>();
    if fields.is_empty() {
        describe_fields_for_sql(sql)
    } else {
        fields
    }
}

fn inlined_storage_type(logical_type: &str) -> Type {
    match logical_type.to_ascii_uppercase().as_str() {
        "BOOLEAN" | "BOOL" => Type::BOOL,
        "TINYINT" | "SMALLINT" | "INT2" | "INT16" => Type::INT2,
        "INTEGER" | "INT" | "INT4" | "INT32" => Type::INT4,
        "BIGINT" | "INT8" | "INT64" => Type::INT8,
        "VARCHAR" | "TEXT" | "STRING" | "BLOB" | "BYTEA" => Type::BYTEA,
        "TIMESTAMP" | "TIMESTAMP WITHOUT TIME ZONE" => Type::TIMESTAMP,
        "TIMESTAMP WITH TIME ZONE" | "TIMESTAMPTZ" => Type::TIMESTAMPTZ,
        "DATE" => Type::DATE,
        _ => Type::TEXT,
    }
}

fn project_described_fields(
    sql: &str,
    schema: Vec<pgwire::api::results::FieldInfo>,
) -> Vec<pgwire::api::results::FieldInfo> {
    let Some(names) = projection_names(sql) else {
        return schema;
    };
    let mut remaining = schema
        .into_iter()
        .map(|field| (field.name().to_ascii_lowercase(), field))
        .collect::<Vec<_>>();
    let mut projected = Vec::new();
    for name in names {
        let Some(index) = remaining
            .iter()
            .position(|(field_name, _)| field_name == &name)
        else {
            return remaining.into_iter().map(|(_, field)| field).collect();
        };
        projected.push(remaining.remove(index).1);
    }
    if projected.is_empty() {
        remaining.into_iter().map(|(_, field)| field).collect()
    } else {
        projected
    }
}
