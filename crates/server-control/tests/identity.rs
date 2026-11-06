//! Requires a disposable database. Never point this at an existing deployment.
#[tokio::test]
#[ignore = "requires MINERVA_TEST_POSTGRES_URL pointing to a disposable PostgreSQL database"]
async fn migration_rejects_cross_tenant_memberships_and_duplicate_identities() {
    use minerva_server_control::store::ControlStore;
    use tokio_postgres::NoTls;
    use uuid::Uuid;
    let url = std::env::var("MINERVA_TEST_POSTGRES_URL").expect("disposable database URL required");
    let (client, connection) = tokio_postgres::connect(&url, NoTls).await.unwrap();
    tokio::spawn(async move {
        connection.await.unwrap();
    });
    let mut store = ControlStore::new(client);
    store.migrate().await.unwrap();
    let mut client = store.into_client();
    let tx = client.transaction().await.unwrap();
    let user = Uuid::new_v4();
    let tenant = Uuid::new_v4();
    let foreign = Uuid::new_v4();
    let workspace = Uuid::new_v4();
    let subject = user.to_string();
    tx.execute(
        "INSERT INTO users(id,issuer,subject) VALUES($1,'https://fixture.invalid',$2)",
        &[&user, &subject],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO tenants(id) VALUES($1),($2)",
        &[&tenant, &foreign],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO workspaces(id,tenant_id) VALUES($1,$2)",
        &[&workspace, &foreign],
    )
    .await
    .unwrap();
    tx.batch_execute("SAVEPOINT invalid_membership")
        .await
        .unwrap();
    let error = tx.execute("INSERT INTO workspace_memberships(user_id,tenant_id,workspace_id,role) VALUES($1,$2,$3,'owner')", &[&user,&tenant,&workspace]).await.unwrap_err();
    assert_eq!(
        error.code(),
        Some(&tokio_postgres::error::SqlState::FOREIGN_KEY_VIOLATION)
    );
    tx.batch_execute("ROLLBACK TO SAVEPOINT invalid_membership")
        .await
        .unwrap();
    let error = tx
        .execute(
            "INSERT INTO users(id,issuer,subject) VALUES($1,'https://fixture.invalid',$2)",
            &[&Uuid::new_v4(), &subject],
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.code(),
        Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
    );
    tx.rollback().await.unwrap();
}
