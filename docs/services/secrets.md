# Secrets Store bindings

Secrets Store bindings expose a read-only `get(): Promise<string>` in Workers
and Durable Objects. celld resolves references through provider crates configured
on each serving node. URI-valued `store_id` fields are a celld extension.

## Configure a binding

```json
{
  "secrets_store_secrets": [
    {
      "binding": "STRIPE_KEY",
      "store_id": "op://production",
      "secret_name": "stripe/api-key"
    }
  ]
}
```

Use `await env.STRIPE_KEY.get()` in a Worker or
`await this.env.STRIPE_KEY.get()` in a Durable Object. A missing secret or provider
failure rejects the promise; an existing empty value returns `""`.

## 1Password Connect

The `celld-secrets-1password` adapter uses the `connect-1password` SDK and provides
the `op` scheme. Set these variables on every serving celld node, including when
running `celld dev`:

```sh
export PRODUCTION_CONNECT_TOKEN='<your-connect-token>'
export CELLD_SECRET_STORES='{
  "op://production": {
    "endpoint": "http://connect:8080",
    "token_env": "PRODUCTION_CONNECT_TOKEN"
  }
}'
```

The URI selects the vault by name or ID. `secret_name` selects
`<item>/<field>` or `<item>/<section>/<field>`, with names or IDs accepted for
each component. This example resolves `op://production/stripe/api-key` through
the configured Connect server. Each configured vault can use a different
endpoint and token variable.

Host credentials never become Worker environment bindings or deployment
metadata. The daemon reads the mapping at startup; restart it after changing
that mapping or its token variables. Each `get()` makes a fresh provider lookup.

The standard build enables the `secrets-1password` Cargo feature. Build without
it using `cargo build -p celld --no-default-features`; the generic binding and
provider interface remain available.

The SDK is vendored with current HTTP/TLS dependencies, credential redaction,
and fixes for Connect API response shapes. See
[`PATCHES.md`](../../vendor/connect-1password/PATCHES.md).
The [static review](secrets-static-review.md) records the fixes, dependency-audit
scope, and deferred concerns with their likelihood/severity ratings.

## Add another provider

Create a crate depending on `celld-secrets` and implement
`SecretProvider::get(store_id, secret_name)`. Register its URI scheme with
`Registry::register`, and add its host configuration construction in
`crates/celld/secrets.rs`, behind an optional Cargo feature. The provider crate
owns its API calls; the Worker/DO bridge needs no provider-specific changes.

See the [1Password Connect API](https://developer.1password.com/docs/connect/api-reference/)
and [secret reference syntax](https://developer.1password.com/docs/cli/secret-reference-syntax/).
