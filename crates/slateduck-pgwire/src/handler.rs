//! PG Wire protocol handler implementation.
//!
//! Implements SimpleQueryHandler and ExtendedQueryHandler for the pgwire crate.

use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use futures::sink::Sink;
use pgwire::api::auth::noop::NoopStartupHandler;
use pgwire::api::copy::NoopCopyHandler;
use pgwire::api::portal::Portal;
use pgwire::api::query::{ExtendedQueryHandler, SimpleQueryHandler};
use pgwire::api::results::{
    DescribePortalResponse, DescribeResponse, DescribeStatementResponse, Response,
};
use pgwire::api::stmt::{QueryParser, StoredStatement};
use pgwire::api::store::PortalStore;
use pgwire::api::{ClientInfo, ClientPortalStore, NoopErrorHandler, PgWireServerHandlers, Type};
use pgwire::error::{PgWireError, PgWireResult};
use pgwire::messages::PgWireBackendMessage;
use tokio::sync::Mutex;

use slateduck_catalog::CatalogStore;
use slateduck_sql::ParamValues;

use crate::executor;
use crate::session::SessionState;

/// The main SlateDuck query handler.
pub struct SlateDuckHandler {
    pub catalog: Arc<Mutex<CatalogStore>>,
    pub session: Arc<Mutex<SessionState>>,
    pub parser: Arc<SlateDuckQueryParser>,
}

impl SlateDuckHandler {
    pub fn new(catalog: Arc<Mutex<CatalogStore>>) -> Self {
        Self {
            catalog,
            session: Arc::new(Mutex::new(SessionState::new())),
            parser: Arc::new(SlateDuckQueryParser),
        }
    }
}

#[async_trait]
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
        let params = ParamValues::default();
        let mut session = self.session.lock().await;
        match executor::execute_sql(query, &params, &self.catalog, &mut session).await {
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

        // Extract parameters from portal
        let param_values: Vec<Option<String>> = portal
            .parameters
            .iter()
            .map(|p| p.as_ref().map(|b| String::from_utf8_lossy(b).to_string()))
            .collect();
        let params = ParamValues::new(param_values);

        let mut session = self.session.lock().await;
        match executor::execute_sql(sql, &params, &self.catalog, &mut session).await {
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
        use pgwire::api::results::FieldFormat;
        use pgwire::api::results::FieldInfo;

        // Classify the statement to determine response schema
        let sql = &stmt.statement;
        let kind = slateduck_sql::classify_statement(sql)
            .unwrap_or(slateduck_sql::StatementKind::Unsupported(String::new()));

        let fields = match kind {
            slateduck_sql::StatementKind::SelectVersion => vec![FieldInfo::new(
                "version".to_string(),
                None,
                None,
                Type::TEXT,
                FieldFormat::Text,
            )],
            slateduck_sql::StatementKind::SelectCurrentSchema => vec![FieldInfo::new(
                "current_schema".to_string(),
                None,
                None,
                Type::TEXT,
                FieldFormat::Text,
            )],
            slateduck_sql::StatementKind::SelectCurrentDatabase => vec![FieldInfo::new(
                "current_database".to_string(),
                None,
                None,
                Type::TEXT,
                FieldFormat::Text,
            )],
            slateduck_sql::StatementKind::SelectPgType => vec![
                FieldInfo::new("oid".to_string(), None, None, Type::INT4, FieldFormat::Text),
                FieldInfo::new(
                    "typname".to_string(),
                    None,
                    None,
                    Type::TEXT,
                    FieldFormat::Text,
                ),
            ],
            slateduck_sql::StatementKind::SelectMaxSnapshot => vec![FieldInfo::new(
                "max".to_string(),
                None,
                None,
                Type::INT8,
                FieldFormat::Text,
            )],
            slateduck_sql::StatementKind::ShowVariable(ref var) => vec![FieldInfo::new(
                var.clone(),
                None,
                None,
                Type::TEXT,
                FieldFormat::Text,
            )],
            _ => vec![],
        };

        Ok(DescribeStatementResponse::new(vec![], fields))
    }

    async fn do_describe_portal<C>(
        &self,
        _client: &mut C,
        _portal: &Portal<Self::Statement>,
    ) -> PgWireResult<DescribePortalResponse>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore<Statement = Self::Statement>,
        C::Error: Debug,
        PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        Ok(DescribePortalResponse::no_data())
    }
}

/// Server handlers collection for SlateDuck.
pub struct SlateDuckServerHandlers {
    pub handler: Arc<SlateDuckHandler>,
    pub copy_handler: Arc<NoopCopyHandler>,
    pub error_handler: Arc<NoopErrorHandler>,
}

impl SlateDuckServerHandlers {
    pub fn new(catalog: Arc<Mutex<CatalogStore>>) -> Self {
        Self {
            handler: Arc::new(SlateDuckHandler::new(catalog)),
            copy_handler: Arc::new(NoopCopyHandler),
            error_handler: Arc::new(NoopErrorHandler),
        }
    }
}

impl PgWireServerHandlers for SlateDuckServerHandlers {
    type StartupHandler = SlateDuckHandler;
    type SimpleQueryHandler = SlateDuckHandler;
    type ExtendedQueryHandler = SlateDuckHandler;
    type CopyHandler = NoopCopyHandler;
    type ErrorHandler = NoopErrorHandler;

    fn simple_query_handler(&self) -> Arc<Self::SimpleQueryHandler> {
        self.handler.clone()
    }

    fn extended_query_handler(&self) -> Arc<Self::ExtendedQueryHandler> {
        self.handler.clone()
    }

    fn startup_handler(&self) -> Arc<Self::StartupHandler> {
        self.handler.clone()
    }

    fn copy_handler(&self) -> Arc<Self::CopyHandler> {
        self.copy_handler.clone()
    }

    fn error_handler(&self) -> Arc<Self::ErrorHandler> {
        self.error_handler.clone()
    }
}
