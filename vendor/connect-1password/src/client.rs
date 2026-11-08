//! HTTP Client

use crate::error::{Error, RequestNotSuccessful};
use async_trait::async_trait;
use dotenv::dotenv;
use exponential_backoff::Backoff;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, header::HeaderValue, Method, Response, StatusCode};
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client as HyperClient},
    rt::TokioExecutor,
};
use hyper_rustls::HttpsConnector;
use log::{debug, error};
use serde_json::Value;
use std::{fmt, ops, time::Duration};

/// GET method
pub const GET: Method = Method::GET;
/// POST method
pub const POST: Method = Method::POST;
/// PUT method
pub const PUT: Method = Method::PUT;
/// DELETE method
pub const DELETE: Method = Method::DELETE;

const RETRY_ATTEMPTS: u32 = 5;

/// Represents a (Hyper) HTTP client.
pub struct Client {
    api_key: String,
    server_url: String,
    https_client: HyperClient<HttpsConnector<HttpConnector>, Full<Bytes>>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").field("server_url", &self.server_url).finish_non_exhaustive()
    }
}

/// Interface for any compatible HTTP client
#[async_trait]
pub trait HTTPClient {
    /// Send a request using the underlying HTTP client
    async fn send_request<T>(
        &self,
        method: &str,
        endpoint: &str,
        params: &[(&str, &str)],
        body: Option<String>,
    ) -> Result<(T, Value), Error>
    where
        T: serde::de::DeserializeOwned + std::fmt::Debug;
}

#[async_trait]
impl HTTPClient for Client {
    async fn send_request<T>(
        &self,
        method: &str,
        endpoint: &str,
        params: &[(&str, &str)],
        body: Option<String>,
    ) -> Result<(T, Value), Error>
    where
        T: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let api_key: &String = &self.api_key;

        let method = match method {
            "GET" => GET,
            "POST" => POST,
            "PUT" => PUT,
            "DELETE" => DELETE,
            &_ => GET,
        };

        let resp = retry_with_backoff(self, &method, &api_key[..], endpoint, params, body).await?;
        let status = resp.status();

        let data: (Result<T, Error>, Value) = resp.into_body().collect()
            .await
            .map_err(Error::new_network_error)
            .map(|collected| {
                let mut bytes = collected.to_bytes();
                if &bytes.len() == &0 {
                    bytes = Bytes::from_static(b"{}");
                }
                let json = serde_json::from_slice(&bytes).map_err(Error::new_parsing_error);

                (json, bytes)
            })
            .and_then(|data| {
                let json = data.0;
                let bytes = std::str::from_utf8(&data.1)?;
                let json_raw: Value = serde_json::from_str(bytes)?;


                match status {
                    StatusCode::OK => {}
                    StatusCode::NO_CONTENT => {}
                    _ => {
                        debug!(
                            "Client error! Status: {}, JSON: {}",
                            status,
                            &bytes.to_string()
                        );

                        return Err(RequestNotSuccessful::new(status, bytes.to_string()).into());
                    }
                };

                Ok((json, json_raw))
            })?;
        let decoded = data.0?;
        let raw_json = data.1;


        Ok((decoded, raw_json))
    }
}

impl Client {
    /// Create a new instance
    ///
    /// # Fields
    ///
    /// - `token`: provide the 1Password Connect API token.
    /// - `server_url`: provide full URL to the host server, i.e. `http://localhost:8080`
    pub fn new(token: &str, server_url: &str) -> Self {
        Self::try_new(token, server_url).expect("load Connect TLS trust roots")
    }

    /// Create a client while reporting TLS trust-root errors to the caller.
    pub fn try_new(token: &str, server_url: &str) -> Result<Self, Error> {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots().map_err(Error::new_network_error)?
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build();

        Ok(Self {
            api_key: token.to_string(),
            server_url: server_url.to_string(),
            https_client: HyperClient::builder(TokioExecutor::new()).build(https),
        })
    }

    /// Create an instance by fetching defaults from the host ENV.
    ///
    /// # Fields
    ///
    /// - `OP_API_TOKEN`: provide the 1Password Connect API token.
    /// - `OP_SERVER_URL`: provide full URL to the host server, i.e. `http://localhost:8080`
    pub fn default() -> Self {
        let token = std::env::var("OP_API_TOKEN").expect("1Password API token expected!");
        let host = std::env::var("OP_SERVER_URL").expect("1Password Connect server URL expected!");

        // .env to override settings in ENV
        dotenv().ok();

        Client::new(&token, &host)
    }

    /// Returns the 1Password Connect API token.
    pub fn token(&self) -> String {
        self.api_key.clone()
    }
}

struct RetryErrors<'a>(pub(crate) &'a mut Vec<String>);

impl<'a> fmt::Display for RetryErrors<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.iter().fold(Ok(()), |result, error_msg| {
            result.and_then(|_| writeln!(f, "{}", error_msg))
        })
    }
}

impl ops::Deref for RetryErrors<'_> {
    type Target = Vec<String>;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

/// Attempt exponential backoff when re-attempting requests to the Melissa service.
async fn retry_with_backoff(
    client: &Client,
    method: &hyper::Method,
    api_key: &str,
    endpoint: &str,
    params: &[(&str, &str)],
    body: Option<String>,
) -> Result<Response<Incoming>, Error> {
    let retries = RETRY_ATTEMPTS;
    let min = Duration::from_millis(100);
    let max = Duration::from_secs(20);
    let backoff = Backoff::new(retries, min, max);
    let mut retry_error_messages: Vec<String> = vec![];
    let mut retry_errors = vec![];

    for duration in &backoff {
        let url = format!("{}/{}?{}", client.server_url, endpoint, url_encode(params));

        let body_data = match body {
            Some(ref value) => Full::new(Bytes::from(value.clone())),
            None => Full::new(Bytes::new()),
        };
        let mut req = hyper::Request::builder()
            .method(method)
            .uri(&*url)
            .body(body_data)?;

        let auth = String::from("Bearer ") + api_key;
        req.headers_mut()
            .insert("Accept", HeaderValue::from_str("application/json")?);
        let mut authorization = HeaderValue::from_str(&auth)?;
        authorization.set_sensitive(true);
        req.headers_mut().insert("Authorization", authorization);

        match client.https_client.request(req).await {
            Ok(value) => return Ok(value),
            Err(err) => {
                let error_message = format!("[ Retrying ]: Client error: {}", err);
                retry_error_messages.push(error_message);
                retry_errors.push(err);

                tokio::time::sleep(duration).await
            }
        }
    }

    let err = if let Some(val) = retry_errors.pop() {
        val
    } else {
        error!("Unable to unwrap error");
        return Err(Error::new_internal_error());
    };

    Err(Error::new_retry_error(err))
}

fn url_encode(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|&t| {
            let (k, v) = t;
            format!("{}={}", k, v)
        })
        .fold("".to_string(), |mut acc, item| {
            acc.push_str(&item);
            acc.push('&');
            acc.replace('+', "%2B")
        })
}
