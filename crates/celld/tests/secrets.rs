#![cfg(feature = "secrets-1password")]
// This integration test drives real processes, files, sockets, and timers.
#![allow(clippy::disallowed_methods)]

use axum::{http::HeaderMap, routing::get, Json, Router};
use serde_json::json;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{
    fs,
    process::{Child, Command, Stdio},
    time::Duration,
};

struct Node(Child);

impl Drop for Node {
    fn drop(&mut self) {
        // `celld dev` supervises a serving node; stop the complete test group.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGTERM);
        }
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn secrets_work_in_a_deployed_worker_and_durable_object() {
    let api = Router::new()
        .route(
            "/v1/vaults",
            get(|headers: HeaderMap| async move {
                assert_eq!(headers["authorization"], "Bearer test-connect-token");
                Json(json!([{"id":"vault-id","name":"production","attributeVersion":1,"contentVersion":1,"type":"USER_CREATED"}]))
            }),
        )
        .route(
            "/v1/vaults/vault-id/items",
            get(|| async { Json(json!([{"id":"item-id","title":"stripe","vault":{"id":"vault-id"}}])) }),
        )
        .route(
            "/v1/vaults/vault-id/items/item-id",
            get(|| async {
                Json(json!({"title":"stripe", "vault":{"id":"vault-id"}, "sections":[], "fields":[
                    {"id":"key-id","label":"api-key","value":"test-secret"},
                    {"id":"empty-id","label":"empty","value":""}
                ]}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, api).await.unwrap();
    });
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("index.js"),
        r#"
        import { DurableObject } from 'cloudflare:workers';
        export class SecretDO extends DurableObject {
            async fetch() { return new Response(await this.env.API_KEY.get()); }
        }
        export default {
            async fetch(request, env) {
                const path = new URL(request.url).pathname;
                if (path === '/ready') return new Response('ready');
                if (path === '/do') return env.OBJECTS.getByName('one').fetch(request);
                if (path === '/empty') return new Response(await env.EMPTY.get());
                if (path === '/host-env') return new Response(JSON.stringify({
                    token: env.TEST_CONNECT_TOKEN, op: globalThis.__secret_get
                }));
                if (path === '/missing') {
                    try { await env.MISSING.get(); return new Response('unexpected'); }
                    catch (error) { return new Response(error.message); }
                }
                return new Response(await env.API_KEY.get());
            }
        };
    "#,
    )
    .unwrap();
    fs::write(
        project.path().join("wrangler.json"),
        json!({
            "name":"secrets-test", "main":"index.js", "no_bundle":true,
            "compatibility_date":"2026-11-08",
            "durable_objects":{"bindings":[{"name":"OBJECTS","class_name":"SecretDO"}]},
            "migrations":[{"tag":"v1","new_sqlite_classes":["SecretDO"]}],
            "secrets_store_secrets":[
                {"binding":"API_KEY","store_id":"op://production","secret_name":"stripe/api-key"},
                {"binding":"EMPTY","store_id":"op://production","secret_name":"stripe/empty"},
                {"binding":"MISSING","store_id":"op://production","secret_name":"stripe/missing"}
            ]
        })
        .to_string(),
    )
    .unwrap();
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let log_path = project.path().join("node.log");
    let log = fs::File::create(&log_path).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_celld"));
    command
        .arg("dev")
        .arg(project.path())
        .args(["--no-watch", "--port", &port.to_string()])
        .env(
            "CELLD_SECRET_STORES",
            json!({"op://production":{
                "endpoint":endpoint, "token_env":"TEST_CONNECT_TOKEN"
            }})
            .to_string(),
        )
        .env("TEST_CONNECT_TOKEN", "test-connect-token")
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log));
    #[cfg(unix)]
    command.process_group(0);
    let mut node = Node(command.spawn().unwrap());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let mut ready = false;
    for _ in 0..300 {
        if node.0.try_wait().unwrap().is_some() {
            panic!("celld exited: {}", fs::read_to_string(&log_path).unwrap());
        }
        if let Ok(response) = client.get(format!("{base}/ready")).send().await {
            if response.status().is_success() {
                ready = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        ready,
        "celld did not become ready: {}",
        fs::read_to_string(&log_path).unwrap()
    );
    for (path, expected) in [
        ("/worker", "test-secret"),
        ("/do", "test-secret"),
        ("/empty", ""),
        ("/missing", "Secret \"stripe/missing\" not found"),
        ("/host-env", "{}"),
    ] {
        let response = client.get(format!("{base}{path}")).send().await.unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert!(status.is_success(), "{path}: {status}: {body}");
        assert_eq!(body, expected, "{path}");
    }
    drop(node);
    let log = fs::read_to_string(&log_path).unwrap();
    assert!(
        !log.contains("test-secret"),
        "the Connect client printed a secret response"
    );
    assert!(
        !log.contains("test-connect-token"),
        "the Connect client printed its credential"
    );
    server.abort();
}
