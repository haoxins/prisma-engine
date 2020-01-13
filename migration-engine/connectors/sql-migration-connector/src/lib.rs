mod component;
mod database_info;
mod error;
mod sql_database_migration_inferrer;
mod sql_database_step_applier;
mod sql_destructive_changes_checker;
mod sql_migration;
mod sql_migration_persistence;
mod sql_renderer;
mod sql_schema_calculator;
mod sql_schema_differ;

pub use error::*;
pub use sql_migration::*;

use component::Component;
use database_info::DatabaseInfo;
use migration_connector::*;
use quaint::{
    prelude::{ConnectionInfo, Queryable, SqlFamily},
    single::Quaint,
};
use sql_database_migration_inferrer::*;
use sql_database_step_applier::*;
use sql_destructive_changes_checker::*;
use sql_migration_persistence::*;
use sql_schema_describer::SqlSchemaDescriberBackend;
use std::{fs, path::PathBuf, sync::Arc, time::Duration};
use tracing::debug;

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

pub struct SqlMigrationConnector {
    pub schema_name: String,
    pub database: Arc<dyn Queryable + Send + Sync + 'static>,
    pub database_info: DatabaseInfo,
    pub database_describer: Arc<dyn SqlSchemaDescriberBackend + Send + Sync + 'static>,
}

impl SqlMigrationConnector {
    pub async fn new(database_str: &str, provider: &str) -> ConnectorResult<Self> {
        validate_database_str(database_str, provider)?;

        let connection_info =
            ConnectionInfo::from_url(database_str).map_err(|err| ConnectorError::url_parse_error(err, database_str))?;

        let connection_fut = async {
            let connection = Quaint::new(database_str)
                .await
                .map_err(SqlError::from)
                .map_err(|err| err.into_connector_error(&connection_info))?;

            // async connections can be lazy, so we issue a simple query to fail early if the database
            // is not reachable.
            connection
                .query_raw("SELECT 1", &[])
                .await
                .map_err(SqlError::from)
                .map_err(|err| err.into_connector_error(&connection.connection_info()))?;

            Ok(connection)
        };

        let connection = tokio::time::timeout(CONNECTION_TIMEOUT, connection_fut)
            .await
            .map_err(|_elapsed| {
                SqlError::from(quaint::error::Error::ConnectTimeout).into_connector_error(&connection_info)
            })??;

        let database_info = DatabaseInfo::new(&connection, connection.connection_info().clone())
            .await
            .map_err(|sql_error| sql_error.into_connector_error(&connection_info))?;

        let schema_name = connection.connection_info().schema_name().to_owned();

        let conn = Arc::new(connection) as Arc<dyn Queryable + Send + Sync>;

        let describer: Arc<dyn SqlSchemaDescriberBackend + Send + Sync + 'static> = match database_info.sql_family() {
            SqlFamily::Mysql => Arc::new(sql_schema_describer::mysql::SqlSchemaDescriber::new(Arc::clone(&conn))),
            SqlFamily::Postgres => Arc::new(sql_schema_describer::postgres::SqlSchemaDescriber::new(Arc::clone(
                &conn,
            ))),
            SqlFamily::Sqlite => Arc::new(sql_schema_describer::sqlite::SqlSchemaDescriber::new(Arc::clone(&conn))),
        };

        Ok(Self {
            database_info,
            schema_name,
            database: conn,
            database_describer: Arc::clone(&describer),
        })
    }

    async fn create_database_impl(&self, db_name: &str) -> SqlResult<()> {
        match self.database_info.sql_family() {
            SqlFamily::Postgres => {
                let query = format!("CREATE DATABASE \"{}\"", db_name);
                self.database.query_raw(&query, &[]).await?;

                Ok(())
            }
            SqlFamily::Sqlite => Ok(()),
            SqlFamily::Mysql => {
                let query = format!("CREATE DATABASE `{}`", db_name);
                self.database.query_raw(&query, &[]).await?;

                Ok(())
            }
        }
    }

    async fn initialize_impl(&self) -> SqlResult<()> {
        // TODO: this code probably does not ever do anything. The schema/db creation happens already in the helper functions above.
        match self.database_info.connection_info() {
            ConnectionInfo::Sqlite { file_path, .. } => {
                let path_buf = PathBuf::from(&file_path);
                match path_buf.parent() {
                    Some(parent_directory) => {
                        fs::create_dir_all(parent_directory).expect("creating the database folders failed")
                    }
                    None => {}
                }
            }
            ConnectionInfo::Postgres(_) => {
                let schema_sql = format!("CREATE SCHEMA IF NOT EXISTS \"{}\";", &self.schema_name);

                debug!("{}", schema_sql);

                self.database.query_raw(&schema_sql, &[]).await?;
            }
            ConnectionInfo::Mysql(_) => {
                let schema_sql = format!(
                    "CREATE SCHEMA IF NOT EXISTS `{}` DEFAULT CHARACTER SET latin1;",
                    &self.schema_name
                );

                debug!("{}", schema_sql);

                self.database.query_raw(&schema_sql, &[]).await?;
            }
        }

        Ok(())
    }

    fn connection_info(&self) -> &ConnectionInfo {
        self.database_info.connection_info()
    }
}

#[async_trait::async_trait]
impl MigrationConnector for SqlMigrationConnector {
    type DatabaseMigration = SqlMigration;

    fn connector_type(&self) -> &'static str {
        self.connection_info().sql_family().as_str()
    }

    async fn create_database(&self, db_name: &str) -> ConnectorResult<()> {
        catch(self.connection_info(), self.create_database_impl(db_name)).await
    }

    async fn initialize(&self) -> ConnectorResult<()> {
        catch(self.connection_info(), self.initialize_impl()).await?;

        self.migration_persistence().init().await?;

        Ok(())
    }

    async fn reset(&self) -> ConnectorResult<()> {
        self.migration_persistence().reset().await?;
        Ok(())
    }

    fn migration_persistence<'a>(&'a self) -> Box<dyn MigrationPersistence + 'a> {
        Box::new(SqlMigrationPersistence { connector: self })
    }

    fn database_migration_inferrer<'a>(&'a self) -> Box<dyn DatabaseMigrationInferrer<SqlMigration> + 'a> {
        Box::new(SqlDatabaseMigrationInferrer { connector: self })
    }

    fn database_migration_step_applier<'a>(&'a self) -> Box<dyn DatabaseMigrationStepApplier<SqlMigration> + 'a> {
        Box::new(SqlDatabaseStepApplier { connector: self })
    }

    fn destructive_changes_checker<'a>(&'a self) -> Box<dyn DestructiveChangesChecker<SqlMigration> + 'a> {
        Box::new(SqlDestructiveChangesChecker { connector: self })
    }

    fn deserialize_database_migration(&self, json: serde_json::Value) -> SqlMigration {
        serde_json::from_value(json).expect("Deserializing the database migration failed.")
    }
}

pub(crate) async fn catch<O>(
    connection_info: &ConnectionInfo,
    fut: impl std::future::Future<Output = Result<O, SqlError>>,
) -> Result<O, ConnectorError> {
    match fut.await {
        Ok(o) => Ok(o),
        Err(sql_error) => Err(sql_error.into_connector_error(connection_info)),
    }
}

fn validate_database_str(database_str: &str, provider: &str) -> ConnectorResult<()> {
    let scheme = database_str.split(":").next();

    match (provider, scheme) {
        ("mysql", Some("mysql")) => Ok(()),
        ("postgresql", Some(scheme)) if scheme.starts_with("postgres") => Ok(()),
        ("postgres", Some(scheme)) if scheme.starts_with("postgres") => Ok(()),
        ("sqlite", Some("file")) | ("sqlite", Some("sqlite")) => Ok(()),
        _ => {
            let error = ConnectorError {
                kind: migration_connector::ErrorKind::InvalidDatabaseUrl,
                user_facing_error: None,
            };

            Err(error)
        }
    }
}
