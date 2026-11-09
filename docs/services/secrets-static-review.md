# Secrets integration: static production review

Reviewed 2026-11-09. Scope: provider interface, 1Password adapter and vendored SDK,
deployment metadata, feature gating, host credentials, Worker/DO environment
construction, asynchronous host operations, and the compiled dependency graph.

The review used source inspection, the official Connect OpenAPI schema, a fresh
independent reviewer, RustSec analysis, compilation, and local synthetic
regressions. No live account is needed as a completion gate.

## Concrete defects addressed

- Correct Connect item URLs (`href`) and optional URL `primary` values. A normal
  login item elsewhere in the vault previously prevented all item lookups.
- Accept items without the optional `sections` array.
- Return an empty string for an existing field whose value is omitted, while
  retaining NotFound for an absent field.
- Remove the SDK's old h2/rustls/ring dependency families by updating its transport
  to Hyper 1, hyper-util 0.1, and hyper-rustls 0.27 with ring. The provider still
  calls the existing SDK's item and vault APIs.
- Redact the token from the SDK client's Debug implementation and mark bearer
  headers sensitive. Existing successful-response logging remains removed.
- Use the fallible SDK constructor from celld, so native trust-root errors are
  returned during startup rather than panicking.
- Replace the SDK's blocking retry sleep with an asynchronous Tokio sleep;
  preserve its existing retry schedule.
- Remove unused SDK logger/futures dependencies, including the unsound `atty`
  dependency they introduced.
- Update Tokio to 1.53.3 for RUSTSEC-2026-0386 and apply compatible zstd,
  zstd-safe, and zstd-sys patches for the compression advisories found by audit.

## Findings and rating rule

Likelihood and severity are each 1–10. Hypothetical changes are deferred unless
both exceed 5, as requested.

| Finding | Likelihood | Severity | Result |
| --- | ---: | ---: | --- |
| Connect URL objects fail SDK parsing | 8 | 7 | Fixed; regression covered |
| Sectionless items fail SDK parsing | 7 | 7 | Fixed; regression covered |
| Omitted empty values become NotFound | 6 | 6 | Fixed; regression covered |
| Backend stalls require a provider-specific timeout | 4 | 6 | Deferred; existing request lifetime still applies |
| Oversized backend responses need a new response-size policy | 3 | 7 | Deferred |
| Backend error responses might contain sensitive data | 3 | 8 | Deferred; no bearer-token exposure found in current error paths |
| Duplicate item/field names need an ambiguity policy | 3 | 6 | Deferred; names and stable IDs both work |
| Read volume needs caching | 4 | 4 | Deferred |
| Credentials need reloading without restart | 3 | 4 | Deferred |
| Secret capabilities need Worker Loader transfer | 3 | 4 | Deferred |

## Dependency audit scope

The compiled 1Password provider dependency subtree has **zero known RustSec
vulnerability findings and zero unsoundness findings** after these updates.
Its remaining maintenance notice is the SDK's legacy `dotenv` dependency
(RUSTSEC-2021-0141); celld uses explicit host configuration and `try_new`, not the
SDK's dotenv-loading default constructor. Replacing that unused legacy helper is
deferred (likelihood 1, severity 2).

The complete workspace lockfile audit is **not globally clean**. These existing
dependencies are outside the provider subtree:

- `crypto-bigint` 0.5.5: RUSTSEC-2026-0387.
- `rsa` 0.9.10: RUSTSEC-2023-0071.
- `quick-xml` 0.38.4: RUSTSEC-2026-0194 and RUSTSEC-2026-0195.
- `quinn-proto` 0.11.16: five HTTP/3 advisories; this package is in Cargo.lock but
  is absent from the compiled dependency graph for this configuration.

The cryptographic timing scenarios require an application exposing a relevant
private-key operation as an oracle (likelihood 2, severity 9). The XML scenarios
require attacker-controlled object-store responses (likelihood 3, severity 7).
Those broader daemon changes remain deferred under the requested threshold.
Enabling currently inactive HTTP/3 is a separate configuration change.

## Verification

- Workspace regression suite, including Worker/DO use and secret-log checks.
- Official-schema fixtures for URLs, optional sections, and omitted empty values.
- Direct SDK-client Debug redaction regression.
- Production release build with `--release --locked`.
- Provider-disabled build with `--no-default-features`.
- Clippy, formatting, whitespace checks, and a final fresh static review.

Clippy retains existing daemon and upstream SDK warnings. Build and regression
results are recorded in the implementation notes. No unresolved above-threshold
finding remains in the reviewed secrets integration.

Sources: [Connect OpenAPI schema](https://github.com/1Password/connect/blob/main/docs/openapi/spec.yaml),
[SDK patches](../../vendor/connect-1password/PATCHES.md),
[implementation notes](../superpowers/plans/2026-11-08-secrets-providers.md).
