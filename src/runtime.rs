use actix::prelude::{Actor, Addr, AtomicResponse, Context, Handler, WrapFuture};
use anyhow::Result;
use std::collections::HashMap;

use crate::{Call, CallResponse, Pallet, RuntimeError, call_execution::CallExecution};

/// Actor that routes [`Call`]s to one of several named, already-started
/// [`Pallet`] actors, keyed by [`Call::pallet_name`].
///
/// It handles the same [`Call`] message a [`Pallet`] does, so talking to a
/// `Runtime` or straight to a single pallet looks identical:
/// `addr.send(call).await??`. A program with just one pallet can skip the
/// `Runtime` entirely.
///
/// Calls are processed **one at a time, in arrival order** (serially): the
/// next [`Call`] isn't taken from the mailbox until the previous one has
/// been answered. That's what keeps state transitions deterministic across
/// pallets, which a block of calls needs.
///
/// Each call is run by a fresh, crate-private `CallExecution` actor, created
/// for that call and dropped once it's answered. It opens the call's root
/// tracing span (`call.execution`, under the span the [`Call`] was created
/// in) and collects the call's events.
///
/// Because of that, a pallet must never send a [`Call`] back to the
/// `Runtime` while executing one (it'd wait on a `Runtime` that's waiting
/// on it: deadlock). Cross-pallet calls (not implemented yet) will go
/// through the `CallExecution` of the call in progress instead.
///
/// # Example
///
/// ```
/// use ledger_me_sdk::actix::Actor;
/// use ledger_me_sdk::{Call, Runtime, RuntimeError};
///
/// actix_rt::System::new().block_on(async {
///     let runtime = Runtime::new().start();
///
///     let call = Call::new("missing", "noop", vec![]);
///     let err = runtime.send(call).await.unwrap().unwrap_err();
///     assert!(matches!(err.downcast_ref(), Some(RuntimeError::PalletNotFound(_))));
/// });
/// ```
#[derive(Default)]
pub struct Runtime {
    pallets_by_name: HashMap<String, Addr<Pallet>>,
}

impl Runtime {
    /// Creates a `Runtime` with no pallets registered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an already-started pallet under `name`, replacing any
    /// pallet already registered under that name.
    pub fn with_pallet(mut self, name: impl Into<String>, pallet: Addr<Pallet>) -> Self {
        self.pallets_by_name.insert(name.into(), pallet);

        self
    }
}

impl Actor for Runtime {
    type Context = Context<Self>;
}

impl Handler<Call> for Runtime {
    // ponytail: AtomicResponse serializes every call; switch to ResponseFuture if independent pallets need to run in parallel
    type Result = AtomicResponse<Self, Result<CallResponse>>;

    /// Hands `call` to a new `CallExecution` for its pallet and waits for
    /// the answer.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::PalletNotFound`] if no pallet is registered under
    /// `call.pallet_name()`, an error if the pallet's mailbox can't be
    /// reached (e.g. its thread panicked), or whatever error the pallet
    /// itself returns.
    fn handle(&mut self, call: Call, _ctx: &mut Self::Context) -> Self::Result {
        let pallet = self.pallets_by_name.get(call.pallet_name()).cloned();

        let run_call = async move {
            let pallet = pallet
                .ok_or_else(|| RuntimeError::PalletNotFound(call.pallet_name().to_string()))?;

            CallExecution::new(pallet).start().send(call).await?
        };

        AtomicResponse::new(Box::pin(run_call.into_actor(self)))
    }
}
