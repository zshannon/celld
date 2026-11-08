//! Private V8 bridge to the host's URI-addressed secrets providers.
use super::*;

pub(super) fn get(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let name = args.get(0).to_rust_string_lossy(scope);
    let binding = actor_runtime_state(scope)
        .secret_bindings
        .get(&name)
        .cloned();
    let providers = asyncrt::services().secrets();
    let id = asyncrt::enqueue(async move {
        let binding = binding.ok_or_else(|| format!("unknown secret binding {name:?}"))?;
        providers
            .get(&binding.store_id, &binding.secret_name)
            .await
            .map_err(|error| error.to_string())
    });
    rv.set(promise_for(scope, id));
}
