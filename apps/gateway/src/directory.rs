use minerva_server_control::policy::Resource;
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio_postgres::Client;
use uuid::Uuid;
pub type DirectoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, DirectoryError>> + Send + 'a>>;
#[derive(Debug, thiserror::Error)]
#[error("no healthy isolated worker is assigned")]
pub struct DirectoryError;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerAssignment {
    pub assignment_id: Uuid,
    pub generation: u64,
    pub worker_id: Uuid,
    pub resource: Resource,
    pub endpoint: String,
}
impl WorkerAssignment {
    pub fn rpc_url(&self) -> String {
        format!("{}/api/v1/rpc", self.endpoint)
    }
    pub fn streams_url(&self) -> String {
        format!("{}/api/v1/streams", self.endpoint).replacen("https://", "wss://", 1)
    }
}
pub trait WorkerDirectory: Send + Sync {
    fn resolve(&self, resource: Resource) -> DirectoryFuture<'_, WorkerAssignment>;
    fn ready(&self) -> DirectoryFuture<'_, bool>;
}
/// Runner is an administrator-configured HTTPS reverse proxy; browser input never selects an endpoint.
pub struct PostgresWorkerDirectory {
    client: Arc<Client>,
    http: reqwest::Client,
    runner_origin: String,
}
pub fn validate_endpoint(
    endpoint: &str,
    runner_origin: &str,
    assignment_id: Uuid,
) -> Result<(), DirectoryError> {
    let url = url::Url::parse(endpoint).map_err(|_| DirectoryError)?;
    if url.origin().ascii_serialization() != runner_origin
        || url.scheme() != "https"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != format!("/workers/{assignment_id}")
    {
        return Err(DirectoryError);
    }
    Ok(())
}
impl PostgresWorkerDirectory {
    pub async fn new(mut client: Client, runner_origin: String) -> Result<Self, DirectoryError> {
        let tx = client.transaction().await.map_err(|_| DirectoryError)?;
        tx.batch_execute("SELECT pg_advisory_xact_lock(739418992); CREATE TABLE IF NOT EXISTS minerva_gateway_migrations(version integer PRIMARY KEY)").await.map_err(|_|DirectoryError)?;
        if tx
            .query_opt(
                "SELECT version FROM minerva_gateway_migrations WHERE version=1",
                &[],
            )
            .await
            .map_err(|_| DirectoryError)?
            .is_none()
        {
            tx.batch_execute(include_str!("../migrations/001_assignments.sql"))
                .await
                .map_err(|_| DirectoryError)?;
            tx.execute(
                "INSERT INTO minerva_gateway_migrations(version) VALUES(1)",
                &[],
            )
            .await
            .map_err(|_| DirectoryError)?;
        }
        tx.commit().await.map_err(|_| DirectoryError)?;
        Ok(Self {
            client: Arc::new(client),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(3))
                .build()
                .map_err(|_| DirectoryError)?,
            runner_origin,
        })
    }
    async fn runner_ready(&self) -> Result<bool, DirectoryError> {
        let response = self
            .http
            .get(format!("{}/health/ready", self.runner_origin))
            .send()
            .await
            .map_err(|_| DirectoryError)?;
        if !response.status().is_success() {
            return Ok(false);
        }
        let bytes = crate::bounded_body(response, 4096)
            .await
            .map_err(|_| DirectoryError)?;
        let data: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| DirectoryError)?;
        Ok(data.get("ready").and_then(|v| v.as_bool()) == Some(true)
            && data.get("sandbox").and_then(|v| v.as_str()) == Some("runsc")
            && data.get("isolationVerified").and_then(|v| v.as_bool()) == Some(true))
    }
}
impl WorkerDirectory for PostgresWorkerDirectory {
    fn resolve(&self, resource: Resource) -> DirectoryFuture<'_, WorkerAssignment> {
        Box::pin(async move {
            if !self.runner_ready().await? {
                return Err(DirectoryError);
            }
            let row=self.client.query_opt("SELECT assignment_id,generation,worker_id,endpoint FROM workspace_worker_assignments WHERE workspace_id=$1 AND tenant_id=$2 AND status='ready' AND sandbox_profile='runsc' AND lease_until>now() AND healthy_at>now()-interval '10 seconds'",&[&resource.workspace_id,&resource.tenant_id]).await.map_err(|_|DirectoryError)?.ok_or(DirectoryError)?;
            let generation: i64 = row.get(1);
            if generation <= 0 {
                return Err(DirectoryError);
            }
            let assignment = WorkerAssignment {
                assignment_id: row.get(0),
                generation: generation as u64,
                worker_id: row.get(2),
                resource,
                endpoint: row.get(3),
            };
            validate_endpoint(
                &assignment.endpoint,
                &self.runner_origin,
                assignment.assignment_id,
            )?;
            Ok(assignment)
        })
    }
    fn ready(&self) -> DirectoryFuture<'_, bool> {
        Box::pin(async move {
            self.client
                .simple_query("SELECT 1")
                .await
                .map_err(|_| DirectoryError)?;
            self.runner_ready().await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_accepts_only_configured_runner_origin_and_fixed_worker_prefix() {
        let id = Uuid::new_v4();
        assert!(validate_endpoint(
            &format!("https://runner.test/workers/{id}"),
            "https://runner.test",
            id
        )
        .is_ok());
        for endpoint in [
            format!("https://evil.test/workers/{id}"),
            format!("http://runner.test/workers/{id}"),
            format!("https://runner.test/workers/{id}?secret=1"),
            format!("https://runner.test/workers/{id}/../other"),
        ] {
            assert!(validate_endpoint(&endpoint, "https://runner.test", id).is_err());
        }
    }
}
#[cfg(test)]
mod postgres_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires MINERVA_TEST_POSTGRES_URL pointing to a disposable PostgreSQL database"]
    async fn assignments_enforce_tenant_constraints_and_unreachable_runner_never_routes() {
        use tokio_postgres::error::SqlState;
        let (client, connection) = tokio_postgres::connect(
            &std::env::var("MINERVA_TEST_POSTGRES_URL").unwrap(),
            tokio_postgres::NoTls,
        )
        .await
        .unwrap();
        tokio::spawn(async move {
            connection.await.unwrap();
        });
        let mut store = minerva_server_control::store::ControlStore::new(client);
        store.migrate().await.unwrap();
        let directory =
            PostgresWorkerDirectory::new(store.into_client(), "https://127.0.0.1:1".into())
                .await
                .unwrap();
        let (tenant, foreign, workspace, assignment, worker) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let client = &directory.client;
        client.batch_execute("BEGIN").await.unwrap();
        client
            .execute(
                "INSERT INTO tenants(id) VALUES($1),($2)",
                &[&tenant, &foreign],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO workspaces(id,tenant_id) VALUES($1,$2)",
                &[&workspace, &tenant],
            )
            .await
            .unwrap();
        let endpoint = format!("https://127.0.0.1:1/workers/{assignment}");
        client
            .batch_execute("SAVEPOINT foreign_assignment")
            .await
            .unwrap();
        let error=client.execute("INSERT INTO workspace_worker_assignments(workspace_id,tenant_id,assignment_id,worker_id,generation,endpoint,sandbox_profile,status,lease_until,healthy_at) VALUES($1,$2,$3,$4,1,$5,'runsc','ready',now()+interval '1 minute',now())",&[&workspace,&foreign,&assignment,&worker,&endpoint]).await.unwrap_err();
        assert_eq!(error.code(), Some(&SqlState::FOREIGN_KEY_VIOLATION));
        client
            .batch_execute("ROLLBACK TO SAVEPOINT foreign_assignment")
            .await
            .unwrap();
        client.execute("INSERT INTO workspace_worker_assignments(workspace_id,tenant_id,assignment_id,worker_id,generation,endpoint,sandbox_profile,status,lease_until,healthy_at) VALUES($1,$2,$3,$4,1,$5,'runsc','ready',now()+interval '1 minute',now())",&[&workspace,&tenant,&assignment,&worker,&endpoint]).await.unwrap();
        assert!(directory
            .resolve(Resource {
                tenant_id: tenant,
                workspace_id: workspace
            })
            .await
            .is_err());
        client.batch_execute("ROLLBACK").await.unwrap();
    }
}
