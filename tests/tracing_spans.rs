//! The SDK's spans must hang under the span the `Call` was created in, even
//! though the call crosses mailboxes and runs the WASM on another thread.

use std::sync::{Arc, Mutex};

use ledger_me_sdk::actix::Actor;
use ledger_me_sdk::adapters::InMemoryStorage;
use ledger_me_sdk::{Call, PalletBuilder, Runtime};
use tracing::Instrument;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

/// Smallest guest the SDK accepts: `memory`, `alloc`, and a `noop` call
/// returning an empty `(ptr, len)`.
const NOOP_PALLET: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "alloc") (param i32) (result i32) i32.const 1024)
  (func (export "noop") (param i32 i32) (result i64) i64.const 0))
"#;

/// `(span name, parent span name)`.
type SpanAndParent = (String, Option<String>);

/// Records a [`SpanAndParent`] for every span created.
#[derive(Clone, Default)]
struct SpanParents(Arc<Mutex<Vec<SpanAndParent>>>);

impl<S: tracing::Subscriber + for<'a> LookupSpan<'a>> Layer<S> for SpanParents {
    fn on_new_span(&self, _: &tracing::span::Attributes<'_>, id: &tracing::span::Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("span was just created");
        let parent = span.parent().map(|parent| parent.name().to_string());

        self.0.lock().unwrap().push((span.name().to_string(), parent));
    }
}

#[actix_rt::test]
async fn sdk_spans_nest_under_the_callers_span() -> anyhow::Result<()> {
    let span_parents = SpanParents::default();
    // Global, not scoped: the pallet runs on its own thread.
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(span_parents.clone()))?;

    let pallet = PalletBuilder::new()
        .with_wasm(NOOP_PALLET)
        .with_storage(Arc::new(InMemoryStorage::new()))
        .build()?
        .start();
    let runtime = Runtime::new().with_pallet("noop_pallet", pallet).start();

    async {
        runtime.send(Call::new("noop_pallet", "noop", vec![])).await?
    }
    .instrument(tracing::info_span!("caller"))
    .await?;

    let span_parents = span_parents.0.lock().unwrap().clone();
    let parent_of = |name: &str| {
        span_parents
            .iter()
            .find(|(span, _)| span == name)
            .unwrap_or_else(|| panic!("no `{name}` span in {span_parents:?}"))
            .1
            .clone()
    };

    assert_eq!(parent_of("call.execution").as_deref(), Some("caller"));
    assert_eq!(parent_of("pallet.call").as_deref(), Some("call.execution"));

    Ok(())
}
