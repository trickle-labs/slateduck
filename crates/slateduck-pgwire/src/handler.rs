//! PG-Wire handler — implements pgwire's SimpleQueryHandler and ExtendedQueryHandler.

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream;
use futures::Sink;
use pgwire::api::auth::noop::NoopStartupHandler;
use pgwire::api::copy::CopyHandler;
use pgwire::api::portal::Portal;
use pgwire::api::query::{ExtendedQueryHandler, SimpleQueryHandler};
use pgwire::api::results::{
    DataRowEncoder, DescribePortalResponse, DescribeStatementResponse, FieldFormat, FieldInfo,
    QueryResponse, Response, Tag,
};
use pgwire::api::stmt::{NoopQueryParser, StoredStatement};
use pgwire::api::store::PortalStore;
use pgwire::api::{ClientInfo, ClientPortalStore, ErrorHandler, PgWireServerHandlers};
use pgwire::error::{ErrorInfo, PgWireError, PgWireResult};
use pgwire::messages::PgWireBackendMessage;
use postgres_types::Type;
use std::fmt::Debug;
use tokio::sync::Mutex;

use slateduck_catalog::CatalogStore;
use slateduck_sql::dispatch;

use crate::error_mapping::{dispatch_to_pg_error, to_pg_error};
use crate::executor::{self, QueryResult};
use crate::pg_types::type_for_oid;
use crate::session::Session;

/// The main SlateDuck pgwire handler.
pub struct SlateDuckHandler {
    store: Arc<CatalogStore>,
    session: Arc<Mutex<Session>>,
}

impl SlateDuckHandler {
    pub fn new(store: Arc<CatalogStore>) -> Self {
        Self {
            store,
            session: Arc::new(Mutex::new(Session::new())),
        }
    }

    async fn execute_sql<'a>(&self, sql: &str) -> PgWireResult<Vec<Response<'a>>> {
        // Handle multiple statements separated by semicolons
        let statements: Vec<&str> = sql
            .split(';')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        let mut responses = Vec::new();
        for stmt_sql in statements {
            let response = self.execute_single(stmt_sql).await?;
            responses.push(response);
        }

        if responses.is_empty() {
            Ok(vec![Response::EmptyQuery])
        } else {
            Ok(responses)
        }
    }

    async fn execute_single<'a>(&self, sql: &str) -> PgWireResult<Response<'a>> {
        let op = match dispatch(sql) {
            Ok(op) => op,
            Err(err) => {
                return Ok(Response::Error(Box::new(dispatch_to_pg_error(&err))));
            }
        };

        let mut session = self.session.lock().await;
        let result = match executor::execute(&op, &self.store, &mut session).await {
            Ok(r) => r,
            Err(err) => {
                return Ok(Response::Error(Box::new(to_pg_error(&err))));
            }
        };

        // Convert QueryResult to Response
        if result.columns.is_empty() {
            // Execution response (INSERT, UPDATE, BEGIN, COMMIT, etc.)
            let tag = result.command_tag.clone();
            match tag.as_str() {
                "BEGIN" => Ok(Response::TransactionStart(Tag::new("BEGIN"))),
                "COMMIT" => Ok(Response::TransactionEnd(Tag::new("COMMIT"))),
                "ROLLBACK" => Ok(Response::TransactionEnd(Tag::new("ROLLBACK"))),
                _ => Ok(Response::Execution(parse_tag(&tag))),
            }
        } else {
            // Query response with rows
            Ok(build_query_response(result))
        }
    }
}

impl NoopStartupHandler for SlateDuckHandler {}

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
        self.execute_sql(query).await
    }
}

#[async_trait]
impl ExtendedQueryHandler for SlateDuckHandler {
    type Statement = String;
    type QueryParser = NoopQueryParser;

    fn query_parser(&self) -> Arc<Self::QueryParser> {
        Arc::new(NoopQueryParser)
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
        // Substitute parameters ($1, $2, ...) with bound values
        let mut resolved_sql = sql.clone();
        for (i, param) in portal.parameters.iter().enumerate() {
            let placeholder = format!("${}", i + 1);
            let value = match param {
                Some(bytes) => {
                    let s = String::from_utf8_lossy(bytes);
                    format!("'{}'", s.replace('\'', "''"))
                }
                None => "NULL".to_string(),
            };
            resolved_sql = resolved_sql.replace(&placeholder, &value);
        }

        self.execute_single(&resolved_sql).await
    }

    async fn do_describe_statement<C>(
        &self,
        _client: &mut C,
        _target: &StoredStatement<Self::Statement>,
    ) -> PgWireResult<DescribeStatementResponse>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore<Statement = Self::Statement>,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        // Return empty description — parameters will be inferred
        Ok(DescribeStatementResponse::new(vec![], vec![]))
    }

    async fn do_describe_portal<C>(
        &self,
        _client: &mut C,
        _target: &Portal<Self::Statement>,
    ) -> PgWireResult<DescribePortalResponse>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore<Statement = Self::Statement>,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        Ok(DescribePortalResponse::new(vec![]))
    }
}

#[async_trait]
impl CopyHandler for SlateDuckHandler {
    async fn on_copy_data<C>(
        &self,
        _client: &mut C,
        _copy_data: pgwire::messages::copy::CopyData,
    ) -> PgWireResult<()>
    where
        C: ClientInfo + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        Err(PgWireError::UserError(Box::new(ErrorInfo::new(
            "ERROR".to_string(),
            "0A000".to_string(),
            "COPY not supported".to_string(),
        ))))
    }
}

impl ErrorHandler for SlateDuckHandler {}

impl PgWireServerHandlers for SlateDuckHandler {
    type StartupHandler = SlateDuckHandler;
    type SimpleQueryHandler = SlateDuckHandler;
    type ExtendedQueryHandler = SlateDuckHandler;
    type CopyHandler = SlateDuckHandler;
    type ErrorHandler = SlateDuckHandler;

    fn simple_query_handler(&self) -> Arc<Self::SimpleQueryHandler> {
        // We need to share state, so this is a trick — use Arc wrapping
        // In practice, the handler is already behind Arc in the server
        Arc::new(SlateDuckHandler {
            store: self.store.clone(),
            session: self.session.clone(),
        })
    }

    fn extended_query_handler(&self) -> Arc<Self::ExtendedQueryHandler> {
        Arc::new(SlateDuckHandler {
            store: self.store.clone(),
            session: self.session.clone(),
        })
    }

    fn startup_handler(&self) -> Arc<Self::StartupHandler> {
        Arc::new(SlateDuckHandler {
            store: self.store.clone(),
            session: self.session.clone(),
        })
    }

    fn copy_handler(&self) -> Arc<Self::CopyHandler> {
        Arc::new(SlateDuckHandler {
            store: self.store.clone(),
            session: self.session.clone(),
        })
    }

    fn error_handler(&self) -> Arc<Self::ErrorHandler> {
        Arc::new(SlateDuckHandler {
            store: self.store.clone(),
            session: self.session.clone(),
        })
    }
}

// -- Helper functions --

fn build_query_response(result: QueryResult) -> Response<'static> {
    let fields: Vec<FieldInfo> = result
        .columns
        .iter()
        .map(|col| {
            FieldInfo::new(
                col.name.clone(),
                None,
                None,
                type_for_oid(col.type_oid),
                FieldFormat::Text,
            )
        })
        .collect();

    let fields_arc = Arc::new(fields.clone());
    let fields_for_encoding = fields_arc.clone();

    let data_rows: Vec<PgWireResult<pgwire::messages::data::DataRow>> = result
        .rows
        .into_iter()
        .map(move |row| {
            let mut encoder = DataRowEncoder::new(fields_for_encoding.clone());
            for (i, val) in row.iter().enumerate() {
                let field_type = &fields_for_encoding[i];
                match val {
                    Some(v) => {
                        encoder.encode_field_with_type_and_format(
                            v,
                            field_type.datatype(),
                            FieldFormat::Text,
                        )?;
                    }
                    None => {
                        encoder.encode_field_with_type_and_format(
                            &None::<&str>,
                            &Type::TEXT,
                            FieldFormat::Text,
                        )?;
                    }
                }
            }
            encoder.finish()
        })
        .collect();

    let mut response = QueryResponse::new(fields_arc, stream::iter(data_rows));
    response.set_command_tag(&result.command_tag);
    Response::Query(response)
}

fn parse_tag(tag: &str) -> Tag {
    // Parse "INSERT 0 1" or "UPDATE 1" style tags
    let parts: Vec<&str> = tag.split_whitespace().collect();
    match parts.as_slice() {
        ["INSERT", _, count] => Tag::new("INSERT").with_rows(count.parse().unwrap_or(0)),
        ["UPDATE", count] => Tag::new("UPDATE").with_rows(count.parse().unwrap_or(0)),
        ["DELETE", count] => Tag::new("DELETE").with_rows(count.parse().unwrap_or(0)),
        [cmd] => Tag::new(cmd),
        _ => Tag::new(tag),
    }
}
