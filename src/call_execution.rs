use std::time::Instant;

use actix::prelude::{
    Actor, ActorFutureExt, Addr, Context, Handler, ResponseActFuture, WrapFuture,
};
use anyhow::Result;
use tracing::{Instrument, Span};

use crate::{Call, CallResponse, Pallet};

/// Actor that coordinates the execution of one top-level [`Call`]: the
/// [`crate::Runtime`] creates a new one per call and drops it once the call
/// is answered, so nothing can leak from one execution into the next (see
/// ADR 0001).
///
/// For now it opens the execution's root span (`call.execution`) and
/// collects the events of everything the call ran. Cross-pallet calls and
/// rollback will build on it.
///
/// It stops by itself: once the `Runtime` drops its `Addr` and the mailbox
/// is empty, actix stops the actor.
pub(crate) struct CallExecution {
    pallet: Addr<Pallet>,
    events: Vec<Vec<u8>>,
}

impl CallExecution {
    pub(crate) fn new(pallet: Addr<Pallet>) -> Self {
        Self {
            pallet,
            events: Vec::new(),
        }
    }
}

impl Actor for CallExecution {
    type Context = Context<Self>;
}

impl Handler<Call> for CallExecution {
    // Not serial on purpose: the mailbox stays free while waiting on the
    // pallet, which cross-pallet calls will need.
    type Result = ResponseActFuture<Self, Result<CallResponse>>;

    fn handle(&mut self, mut call: Call, _ctx: &mut Self::Context) -> Self::Result {
        let span = tracing::info_span!(
            parent: &call.context.parent_span,
            "call.execution",
            pallet = %call.pallet,
            function = %call.function,
        );
        call.context.parent_span = span.clone();
        let started_at = Instant::now();

        let pallet = self.pallet.clone();
        let run_on_pallet = async move { pallet.send(call).await? }.instrument(span.clone());

        // actix only hands back `&mut self` inside `map`, so the closure just
        // forwards to `finish`.
        Box::pin(
            run_on_pallet
                .into_actor(self)
                .map(move |result, execution, _ctx| execution.finish(result, &span, started_at)),
        )
    }
}

impl CallExecution {
    /// Runs after the pallet answers: logs the outcome and returns the
    /// response with every event collected by this execution.
    fn finish(
        &mut self,
        result: Result<CallResponse>,
        span: &Span,
        started_at: Instant,
    ) -> Result<CallResponse> {
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(parent: span, elapsed = ?started_at.elapsed(), error = %error, "call failed");
                return Err(error);
            }
        };

        self.events.extend(response.events);

        tracing::debug!(
            parent: span,
            elapsed = ?started_at.elapsed(),
            output_bytes = response.data.len(),
            events = self.events.len(),
            fuel_used = response.fuel_used,
            "call succeeded",
        );

        Ok(CallResponse::new(
            response.data,
            std::mem::take(&mut self.events),
            response.fuel_used,
        ))
    }
}
