# Local patches to connect-1password 2.0.1

Source: the crates.io `connect-1password` 2.0.1 distribution. The upstream MIT
license is retained in `LICENSE`.

- Remove unconditional response-body `dbg!` calls in `src/client.rs`. They print
  plaintext secrets to stderr on every successful lookup.
- Map `VaultData.attribute_version` and `content_version` to the Connect API's
  `attributeVersion` and `contentVersion` JSON keys. These annotations are already
  present in upstream master, but absent from the published 2.0.1 crate.
- Map item URLs to Connect's `href` key, default an omitted `primary` flag to
  false, and default omitted item sections to an empty list, following the
  official Connect OpenAPI schema.
- Upgrade the SDK's HTTP transport to Hyper 1, hyper-util 0.1, and hyper-rustls
  0.27 with the existing ring provider. This removes its old h2, rustls, and ring
  dependency families reported by RustSec. Keep the SDK's public request API.
- Provide `Client::try_new` so celld can report native trust-root loading errors
  during startup. The original `Client::new` remains available.
- Redact the API token from `Client::Debug` and mark the Authorization header
  sensitive. Use Tokio's asynchronous delay for the existing retry schedule.
- Remove unused direct `env_logger` and `futures` dependencies; SDK operation
  does not depend on either. This also removes the old `atty` dependency.

The celld provider delegates vault and item requests to the SDK. It uses the
SDK's accompanying raw item result for field IDs, which the published SDK's
`FieldObject` model does not expose.
