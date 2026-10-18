/// A request to invoke one exported function of one pallet.
///
/// An actix message: send it to an `Addr<Runtime>` (which picks the
/// pallet by [`pallet_name`](Self::pallet_name)) or straight to an
/// `Addr<Pallet>`. `data` is the raw payload handed to the
/// guest's `function` export as a `(ptr, len)` pair — encoding/decoding it
/// is left to the pallet.
#[derive(Debug, Clone)]
pub struct Call {
    pub pallet: String,
    pub function: String,
    pub data: Vec<u8>,
    /// Most fuel the call may use; running out fails it with
    /// [`crate::RuntimeError::OutOfFuel`]. Most WASM instructions use 1
    /// fuel, and a host function call uses 1 whatever it does.
    /// [`Call::new`] sets [`Call::DEFAULT_FUEL_LIMIT`];
    /// [`with_fuel_limit`](Call::with_fuel_limit) changes it.
    pub fuel_limit: u64,
    pub context: CallContext,
}

/// What travels along with a [`Call`] from actor to actor, besides the
/// call itself.
///
/// Today that's only the tracing span the call was created in, so the
/// spans the SDK opens while running it (`call.execution`, `pallet.call`)
/// hang under the caller's span, even across mailboxes and threads.
#[derive(Debug, Clone)]
pub struct CallContext {
    /// Parent of the spans opened while running the call. [`Call::new`]
    /// fills it with [`tracing::Span::current()`]; with no subscriber
    /// installed it's a disabled span and costs nothing.
    pub parent_span: tracing::Span,
}

impl CallContext {
    /// A context whose parent span is whatever span is current right now.
    pub fn from_current_span() -> Self {
        Self {
            parent_span: tracing::Span::current(),
        }
    }
}

impl Call {
    /// Fuel limit [`Call::new`] sets: 100 million, about 5–9 ms of CPU on
    /// the machine measured in ADR 0002 and ~47 thousand times the fuel of
    /// a small call like `pallet_example`'s `sum`. A safety net against
    /// infinite loops, not a price.
    pub const DEFAULT_FUEL_LIMIT: u64 = 100_000_000;

    /// Creates a [`Call`] targeting `function` on `pallet` with the given
    /// raw payload and [`Call::DEFAULT_FUEL_LIMIT`].
    ///
    /// The call's [`CallContext`] captures the current tracing span, so
    /// creating the `Call` inside a span is all it takes for the SDK's
    /// spans to show up under it.
    ///
    /// # Example
    ///
    /// ```
    /// use ledger_me_sdk::Call;
    ///
    /// let call = Call::new("token", "transfer", b"alice,bob,10".to_vec());
    /// assert_eq!(call.pallet_name(), "token");
    /// ```
    pub fn new(
        pallet: impl Into<String>,
        function: impl Into<String>,
        data: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            pallet: pallet.into(),
            function: function.into(),
            data: data.into(),
            fuel_limit: Self::DEFAULT_FUEL_LIMIT,
            context: CallContext::from_current_span(),
        }
    }

    /// Sets the most fuel this call may use (see [`Call::fuel_limit`]).
    ///
    /// # Example
    ///
    /// ```
    /// use ledger_me_sdk::Call;
    ///
    /// let call = Call::new("token", "transfer", b"alice,bob,10".to_vec()).with_fuel_limit(10_000_000);
    /// assert_eq!(call.fuel_limit, 10_000_000);
    /// ```
    pub fn with_fuel_limit(mut self, fuel_limit: u64) -> Self {
        self.fuel_limit = fuel_limit;
        self
    }

    /// The name of the pallet this call is addressed to.
    pub fn pallet_name(&self) -> &str {
        &self.pallet
    }
}

/// The result of executing a [`Call`]: the raw bytes returned by the
/// guest's exported function, any events it emitted along the way (via
/// [`crate::PalletContext`]) and the fuel it used.
#[derive(Debug)]
pub struct CallResponse {
    pub data: Vec<u8>,
    pub events: Vec<Vec<u8>>,
    /// Fuel the call used, out of its [`Call::fuel_limit`]. Deterministic:
    /// the same on every node.
    pub fuel_used: u64,
}

impl CallResponse {
    /// A response with no data, no events and no fuel used.
    pub fn empty() -> Self {
        Self {
            data: Vec::new(),
            events: Vec::new(),
            fuel_used: 0,
        }
    }

    /// Creates a response from the guest's output bytes, the events
    /// collected during the call and the fuel it used.
    pub fn new(data: impl Into<Vec<u8>>, events: Vec<Vec<u8>>, fuel_used: u64) -> Self {
        Self {
            data: data.into(),
            events,
            fuel_used,
        }
    }
}
