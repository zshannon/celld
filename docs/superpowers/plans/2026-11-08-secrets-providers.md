# URI-addressed secrets providers

Approved design: Cloudflare-shaped `secrets_store_secrets` bindings expose
`get(): Promise<string>` in Workers and Durable Objects. `store_id` is a provider
URI (`op://production`); `secret_name` selects an item and field (`stripe/api-key`).
Provider credentials come from `CELLD_SECRET_STORES` on the celld host.

Implement directly in the existing checkout on `feat/secrets-providers`.

- [x] Add `celld-secrets`: binding types, provider trait, URI routing, errors,
  and a test proving an injected provider receives the configured reference.
- [x] Add `celld-secrets-1password`: Connect endpoint/token configuration,
  vault/item/field resolution, and tests against a local HTTP server.
- [x] Thread bindings through deploy metadata, feature gating, fleet loading,
  engine configuration and private async V8 operations. Load host providers at
  node startup, with an optional `secrets-1password` Cargo feature enabled by default.
- [x] Verify Worker and Durable Object binding access, promise rejection, config
  parsing and compilation with/without the provider feature. Document configuration.

Hypothetical issues (likelihood / severity, each 1–10):

| Issue | Rating | Decision |
| --- | --- | --- |
| Provider clients lack configured host credentials | 7 / 7 | Report a clear configuration error |
| A backend lookup fails or finds no value | 7 / 6 | Reject the binding promise |
| Credentials reach deployment metadata or Worker env | 6 / 9 | Keep provider credentials in host services only |
| Duplicate names within a 1Password vault/item | 3 / 6 | Defer ambiguity policy; use the first match |
| Read volume needs caching | 4 / 4 | Defer caching |
| A temporary backend outage needs retries | 4 / 6 | Defer retries |
| Credentials must reload without process restart | 3 / 4 | Defer live reconfiguration |
| Dynamic Worker Loader must transfer secret capabilities | 3 / 4 | Defer capability transfer |
| Binary providers must load dynamically | 2 / 4 | Defer dynamic loading; use Cargo features |
| A sectioned field is referenced without its section | 3 / 4 | Defer shorthand; explicit section paths work |
| A test's released listener port is taken before startup | 2 / 2 | Defer port-reservation machinery |
| Provider credentials are explicitly placed in project dotenv files | 4 / 8 | Defer changes to the existing dotenv import policy; configure host environment variables |
| A secret is read during module evaluation | 2 / 3 | Defer broader top-level I/O compatibility |

No storage engine, management API, custom retries, cache, or background refresh is part
of this implementation. Basic valid-input behavior and the approved API contract
are requirements, rather than hypothetical extensions.

Verification record:

- Baseline `cargo +1.99.1 check --locked -p celld` passed.
- Core/provider tests failed before implementation, then passed.
- Deployment/host configuration tests failed before implementation, then passed.
- Full CLI Worker/DO integration failed with a missing binding before runtime
  wiring, then passed for Worker/DO reads, empty values, missing-secret rejection,
  and host credential/op non-exposure.
- Provider-free compilation passed with `--no-default-features`.
- Fresh independent code review found no blocking issues; its one hypothetical
  compatibility concern (section shorthand, 3 / 4) is deferred above.
- Final `cargo +1.99.1 test --workspace` passed: 8 tests, no failures.
- `cargo +1.99.1 fmt --all -- --check` and `git diff --check` passed.
- Clippy completed successfully with warnings, including an async-trait-generated
  `double_must_use` warning and existing workspace warnings.

## SDK replacement — 2026-11-09

The provider now delegates vault and item operations to the existing
`connect-1password` 2.0.1 crate. Its handwritten HTTP client and response models
were removed. The SDK is vendored with the upstream MIT license and a documented
patch limited to removing secret response prints and correcting vault version
JSON field names. The host mapping and Worker/DO API remain the same.

- Published client prints plaintext secrets: likelihood 10 / severity 9.
  Demonstrated by the integration test's log assertion; fix in the vendored SDK.
- Published vault model rejects normal Connect camelCase fields: likelihood 10 /
  severity 7. Demonstrated by the integration test; apply upstream master's two
  serde field-name annotations.
- SDK network retries block a thread during an outage: likelihood 4 / severity 6.
  Defer changing upstream retry behavior under the user's threshold.
- The SDK's typed field model omits IDs: use the SDK's accompanying raw item
  result to preserve the already-supported field-ID references.

Verification: the unpatched SDK failed the vault-field decoding regression;
after correcting the field names, the log regression failed because a plaintext
secret was printed. With both patches applied, `cargo +1.99.1 test --workspace`
passes all 8 tests, including the Worker/DO integration and secret-log assertions.
Provider-disabled compilation and formatting checks also pass.

## Static production hardening — 2026-11-09

Completed the source/schema, runtime integration, dependency, and fresh
independent static review. Fixed normal URL response parsing, optional sections,
and existing empty-field handling. Modernized the SDK's HTTP/TLS dependencies,
redacted client Debug/Authorization output, propagated native trust-root failures,
and made retry delays asynchronous. Removed unused SDK dependencies and applied
compatible Tokio 1.53.3 and compression-library security patches.

- `cargo +1.99.1 test --workspace`: 11 passed, no failures.
- `cargo +1.99.1 build --release --locked -p celld`: passed.
- Provider-disabled build: passed.
- Clippy: completed with existing daemon/upstream warnings.
- Formatting and whitespace checks: passed.
- Final independent static review: no blocking findings in the changes.
- Known RustSec vulnerability/unsoundness findings in provider subtree: zero.
  Global baseline and inactive-package advisories are documented rather than
  presented as a clean whole-repository audit.

Full findings, scope and likelihood/severity ratings:
`docs/services/secrets-static-review.md`. No live-vault test is a completion gate.
