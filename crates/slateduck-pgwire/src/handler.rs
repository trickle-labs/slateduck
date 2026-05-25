//! PG Wire protocol handler implementation.
//!
//! Implements SimpleQueryHandler and ExtendedQueryHandler for the pgwire crate.
//! Supports optional password authentication (cleartext with constant-time comparison).

use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use futures::sink::{Sink, SinkExt};
use pgwire::api::auth::{
    finish_authentication, save_startup_parameters_to_metadata, DefaultServerParameterProvider,
};
use pgwire::api::copy::NoopCopyHandler;
use pgwire::api::portal::Portal;
use pgwire::api::query::{ExtendedQueryHandler, SimpleQueryHandler};
use pgwire::api::results::{
    DescribePortalResponse, DescribeResponse, DescribeStatementResponse, Response,
};
use pgwire::api::stmt::{QueryParser, StoredStatement};
use pgwire::api::store::PortalStore;
use pgwire::api::{
    ClientInfo, ClientPortalStore, NoopErrorHandler, PgWireConnectionState, PgWireServerHandlers,
    Type, METADATA_USER,
};
use pgwire::error::{ErrorInfo, PgWireError, PgWireResult};
use pgwire::messages::response::ErrorResponse;
use pgwire::messages::startup::Authentication;
use pgwire::messages::{PgWireBackendMessage, PgWireFrontendMessage};
use tokio::sync::Mutex;

use slateduck_catalog::CatalogStore;
use slateduck_sql::ParamValues;

use crate::executor;
use crate::notify::NotifyManager;
use crate::server::AuthConfig;
use crate::session::SessionState;

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
        let params = ParamValues::default();
        let mut session = self.session.lock().await;
        match executor::execute_sql(query, &params, &self.catalog, &mut session, &self.notify_manager, &self.extension_schemas).await {
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
        match executor::execute_sql(sql, &params, &self.catalog, &mut session, &self.notify_manager, &self.extension_schemas).await {
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
    pub startup: Arc<SlateDuckStartupHandler>,
    pub copy_handler: Arc<NoopCopyHandler>,
    pub error_handler: Arc<NoopErrorHandler>,
}

impl SlateDuckServerHandlers {
    pub fn new(catalog: Arc<Mutex<CatalogStore>>) -> Self {
        let auth = Arc::new(AuthConfig::default());
        Self {
            handler: Arc::new(SlateDuckHandler::new(catalog)),
            startup: Arc::new(SlateDuckStartupHandler::new(auth)),
            copy_handler: Arc::new(NoopCopyHandler),
            error_handler: Arc::new(NoopErrorHandler),
        }
    }

    pub fn new_with_auth(catalog: Arc<Mutex<CatalogStore>>, auth: Arc<AuthConfig>) -> Self {
        Self {
            handler: Arc::new(SlateDuckHandler::new_with_auth(catalog, auth.clone())),
            startup: Arc::new(SlateDuckStartupHandler::new(auth)),
            copy_handler: Arc::new(NoopCopyHandler),
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
        Self {
            handler: Arc::new(SlateDuckHandler::new_with_config(
                catalog,
                auth.clone(),
                notify_manager,
                extension_schemas,
            )),
            startup: Arc::new(SlateDuckStartupHandler::new_with_tls_required(
                auth,
                tls_required,
            )),
            copy_handler: Arc::new(NoopCopyHandler),
            error_handler: Arc::new(NoopErrorHandler),
        }
    }
}

impl PgWireServerHandlers for SlateDuckServerHandlers {
    type StartupHandler = SlateDuckStartupHandler;
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
        self.startup.clone()
    }

    fn copy_handler(&self) -> Arc<Self::CopyHandler> {
        self.copy_handler.clone()
    }

    fn error_handler(&self) -> Arc<Self::ErrorHandler> {
        self.error_handler.clone()
    }
}
