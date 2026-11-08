//! The WebAssembly runtime (FR-010 to FR-020, contract `host-abi.md`): one engine and one linker
//! for every plugin, modules checked against the ABI and the granted capabilities, instances with
//! their own store, memory limiter and fuel, and calls run in resumable slices.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bientot_plugin_api::{ABI_VERSION, Capability};
use wasmi::{
    Config, Engine, Error, ExternType, Func, FuncType, Linker, Memory, Module, ResourceLimiter,
    ResumableCall, Store, TrapCode, Val, ValType,
};
use wasmi_core::LimiterError;

use crate::host::{self, HostError, HostState, PluginContext, UiChange};

/// One slice of fuel; the thread can look at its queue between two (FR-013).
pub const SLICE: u64 = 2_000_000;
pub const BASE_ALLOWANCE: u64 = 20_000_000;
pub const ALLOWANCE_PER_BYTE: u64 = 250;
pub const BACKGROUND_CAP: u64 = 100_000_000;
pub const USER_CAP: u64 = 400_000_000;
pub const BACKGROUND_SAFETY_NET: Duration = Duration::from_millis(500);
pub const USER_SAFETY_NET: Duration = Duration::from_secs(2);
/// For all plugins together (FR-015).
pub const POOL_BYTES: usize = 32 << 20;
/// A response is read only up to this size.
pub const MAX_RESPONSE_BYTES: usize = 64 << 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    /// `day.refreshed`, `activate`, `installed`, `upgraded`, `language.changed`.
    Background,
    /// `command.invoked`.
    User,
}

/// The fuel and time one event may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub allowance: u64,
    /// Time spent in the plugin, not counting `between`.
    pub safety_net: Duration,
}

impl Limits {
    /// 20 M plus 250 per byte of input, capped by the kind of call (FR-013).
    pub fn for_event(kind: CallKind, input_bytes: usize) -> Limits {
        let (cap, safety_net) = match kind {
            CallKind::Background => (BACKGROUND_CAP, BACKGROUND_SAFETY_NET),
            CallKind::User => (USER_CAP, USER_SAFETY_NET),
        };
        Limits {
            allowance: (BASE_ALLOWANCE + ALLOWANCE_PER_BYTE * input_bytes as u64).min(cap),
            safety_net,
        }
    }
}

/// Why a module is refused at install or load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleError {
    Invalid(String),
    MissingExport(&'static str),
    WrongExport(&'static str),
    UnknownImport(String),
    NotGranted(Capability),
    BadAbi(i32),
}

impl std::fmt::Display for ModuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModuleError::Invalid(detail) => {
                write!(f, "plugin.wasm is not a valid module: {detail}")
            }
            ModuleError::MissingExport(name) => write!(f, "plugin.wasm does not export `{name}`"),
            ModuleError::WrongExport(name) => {
                write!(f, "plugin.wasm exports `{name}` with the wrong type")
            }
            ModuleError::UnknownImport(name) => {
                write!(
                    f,
                    "plugin.wasm imports `{name}`, which Bientôt does not offer"
                )
            }
            ModuleError::NotGranted(capability) => write!(
                f,
                "plugin.wasm uses `{}` without the capability `{}`",
                capability.interface(),
                capability.name()
            ),
            ModuleError::BadAbi(abi) => {
                write!(f, "plugin.wasm speaks ABI {abi}, not {ABI_VERSION}")
            }
        }
    }
}

impl std::error::Error for ModuleError {}

/// Why a call stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// A trap, including `unreachable` and out-of-bounds access.
    Trap(String),
    OutOfFuel,
    /// The response or an import request was not what the ABI allows.
    BadData(String),
    /// The wall-clock safety net; not a failure (FR-013).
    TimedOut,
    /// The memory pool is full; not a failure (FR-015).
    PoolFull,
    BadAbi(i32),
}

impl CallError {
    /// Whether this call counts toward the three-traps rule (contract `host-abi.md`).
    pub fn is_failure(&self) -> bool {
        !matches!(self, CallError::TimedOut | CallError::PoolFull)
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Trap(detail) => write!(f, "trap: {detail}"),
            CallError::OutOfFuel => write!(f, "the call used up its fuel"),
            CallError::BadData(detail) => write!(f, "bad data: {detail}"),
            CallError::TimedOut => write!(f, "the call ran past its time limit"),
            CallError::PoolFull => write!(f, "the plugin memory pool is full"),
            CallError::BadAbi(abi) => write!(f, "the module speaks ABI {abi}"),
        }
    }
}

impl std::error::Error for CallError {}

/// The memory shared by every instance.
#[derive(Debug)]
pub struct Pool {
    capacity: usize,
    used: AtomicUsize,
}

impl Pool {
    pub fn new(capacity: usize) -> Arc<Pool> {
        Arc::new(Pool {
            capacity,
            used: AtomicUsize::new(0),
        })
    }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    fn take(&self, bytes: usize) -> bool {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|total| *total <= self.capacity)
            })
            .is_ok()
    }

    fn give(&self, bytes: usize) {
        self.used.fetch_sub(bytes, Ordering::AcqRel);
    }
}

/// Holds an instance to its declared memory and its share of the pool. The declared minimum is
/// taken from the pool when the instance is created, the rest as it grows.
#[derive(Debug)]
pub struct Limiter {
    pool: Arc<Pool>,
    max: usize,
    min: usize,
    charged: usize,
    pub(crate) denied_by_pool: bool,
}

impl Limiter {
    fn new(pool: Arc<Pool>, min: usize, max: usize) -> Option<Limiter> {
        // `then`, not `then_some`: a limiter built for nothing would give `min` back on drop.
        pool.take(min).then(|| Limiter {
            pool,
            max,
            min,
            charged: min,
            denied_by_pool: false,
        })
    }
}

impl Drop for Limiter {
    fn drop(&mut self) {
        self.pool.give(self.charged);
    }
}

impl ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, LimiterError> {
        if desired > self.max {
            return Ok(false);
        }
        let wanted = desired.max(self.min);
        let more = wanted.saturating_sub(self.charged);
        if !self.pool.take(more) {
            self.denied_by_pool = true;
            return Ok(false);
        }
        self.charged = self.charged.max(wanted);
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, LimiterError> {
        Ok(desired <= 10_000)
    }

    fn instances(&self) -> usize {
        1
    }

    fn tables(&self) -> usize {
        4
    }

    fn memories(&self) -> usize {
        1
    }
}

/// The engine, the linker and the pool. Created only when a plugin is turned on (FR-018).
pub struct Runtime {
    engine: Engine,
    linker: Linker<HostState>,
    pool: Arc<Pool>,
}

impl Default for Runtime {
    fn default() -> Runtime {
        Runtime::new(Pool::new(POOL_BYTES))
    }
}

impl Runtime {
    pub fn new(pool: Arc<Pool>) -> Runtime {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let mut linker = Linker::new(&engine);
        host::define(&mut linker).expect("the host imports are defined once");
        Runtime {
            engine,
            linker,
            pool,
        }
    }

    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }

    /// Validates a module and checks its exports and imports (FR-020).
    pub fn compile(
        &self,
        wasm: &[u8],
        granted: &BTreeSet<Capability>,
    ) -> Result<Module, ModuleError> {
        let module =
            Module::new(&self.engine, wasm).map_err(|e| ModuleError::Invalid(e.to_string()))?;
        let i32_ = ValType::I32;
        let exports: [(&'static str, Option<FuncType>); 5] = [
            ("memory", None),
            ("bt_abi", Some(FuncType::new([], [i32_]))),
            ("bt_alloc", Some(FuncType::new([i32_], [i32_]))),
            ("bt_free", Some(FuncType::new([i32_, i32_], []))),
            (
                "bt_handle",
                Some(FuncType::new([i32_, i32_], [ValType::I64])),
            ),
        ];
        for (name, signature) in exports {
            let found = module
                .exports()
                .find(|e| e.name() == name)
                .ok_or(ModuleError::MissingExport(name))?;
            let matches = match (found.ty(), &signature) {
                (ExternType::Memory(_), None) => true,
                (ExternType::Func(ty), Some(expected)) => ty == expected,
                _ => false,
            };
            if !matches {
                return Err(ModuleError::WrongExport(name));
            }
        }
        for import in module.imports() {
            let name = format!("{}.{}", import.module(), import.name());
            let ExternType::Func(ty) = import.ty() else {
                return Err(ModuleError::UnknownImport(name));
            };
            let Some((needs, expected)) = host::import(import.module(), import.name()) else {
                return Err(ModuleError::UnknownImport(name));
            };
            if *ty != expected {
                return Err(ModuleError::UnknownImport(name));
            }
            if let Some(capability) = needs
                && !granted.contains(&capability)
            {
                return Err(ModuleError::NotGranted(capability));
            }
        }
        Ok(module)
    }

    /// Creates an instance: reserves its minimum memory, runs its start function under the
    /// background limits and checks `bt_abi`.
    pub fn instantiate(
        &self,
        module: &Module,
        context: PluginContext,
    ) -> Result<PluginInstance, CallError> {
        let memory_min = context.manifest.memory_min as usize;
        let memory_max = context.manifest.memory as usize;
        let limiter =
            Limiter::new(self.pool.clone(), memory_min, memory_max).ok_or(CallError::PoolFull)?;
        let mut store = Store::new(&self.engine, HostState::new(context, limiter));
        store.limiter(|state| &mut state.limiter);
        store
            .set_fuel(BACKGROUND_CAP)
            .map_err(|e| CallError::Trap(e.to_string()))?;
        let started = Instant::now();
        let instance = self
            .linker
            .instantiate_and_start(&mut store, module)
            .map_err(|e| classify(&store, e))?;
        let get = |name: &str| {
            instance
                .get_func(&store, name)
                .ok_or_else(|| CallError::BadData(format!("no `{name}`")))
        };
        let (abi, alloc, free, handle) = (
            get("bt_abi")?,
            get("bt_alloc")?,
            get("bt_free")?,
            get("bt_handle")?,
        );
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| CallError::BadData("no memory".into()))?;
        let mut out = [Val::I32(0)];
        abi.call(&mut store, &[], &mut out)
            .map_err(|e| classify(&store, e))?;
        let version = out[0].i32().unwrap_or(-1);
        if version != ABI_VERSION {
            return Err(CallError::BadAbi(version));
        }
        let mut plugin = PluginInstance {
            store,
            memory,
            alloc,
            free,
            handle,
            busy: started.elapsed(),
        };
        plugin.store.data_mut().take_changes();
        Ok(plugin)
    }
}

/// One live instance. Dropping it gives its memory back to the pool.
pub struct PluginInstance {
    store: Store<HostState>,
    memory: Memory,
    alloc: Func,
    free: Func,
    handle: Func,
    /// Time spent in this instance since the last [`PluginInstance::take_busy`].
    busy: Duration,
}

/// What one event produced.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outcome {
    pub ui: Vec<UiChange>,
    /// Trees `bt:ui@1 set` refused; each counts as a failure but keeps the instance.
    pub rejected: Vec<String>,
    pub fuel: u64,
    pub busy: Duration,
}

struct Meter {
    allowance: u64,
    used: u64,
    busy: Duration,
    safety_net: Duration,
}

impl PluginInstance {
    pub fn state(&self) -> &HostState {
        self.store.data()
    }

    pub fn state_mut(&mut self) -> &mut HostState {
        self.store.data_mut()
    }

    /// The instance's linear memory in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.memory.data_size(&self.store)
    }

    /// Time spent in this instance since the last call, including its start.
    pub fn take_busy(&mut self) -> Duration {
        std::mem::take(&mut self.busy)
    }

    /// Delivers one event. `between` runs between two slices; the thread serves user actions of
    /// other plugins there. After an error the instance must be dropped.
    pub fn deliver(
        &mut self,
        event: &[u8],
        limits: Limits,
        between: &mut dyn FnMut(),
    ) -> Result<Outcome, CallError> {
        let mut meter = Meter {
            allowance: limits.allowance,
            used: 0,
            busy: Duration::ZERO,
            safety_net: limits.safety_net,
        };
        self.store.data_mut().begin_event();
        let result = self.deliver_metered(event, &mut meter, between);
        self.busy += meter.busy;
        let changes = self.store.data_mut().take_changes();
        result.map(|()| Outcome {
            ui: changes.ui,
            rejected: changes.rejected,
            fuel: meter.used,
            busy: meter.busy,
        })
    }

    fn deliver_metered(
        &mut self,
        event: &[u8],
        meter: &mut Meter,
        between: &mut dyn FnMut(),
    ) -> Result<(), CallError> {
        let len = i32::try_from(event.len()).map_err(|_| CallError::BadData("event".into()))?;
        let mut out = [Val::I32(0)];
        self.run(self.alloc, &[Val::I32(len)], &mut out, meter, between)?;
        let ptr = out[0].i32().unwrap_or(0);
        if ptr == 0 {
            return Err(CallError::Trap("bt_alloc returned 0".into()));
        }
        self.memory
            .write(&mut self.store, ptr as u32 as usize, event)
            .map_err(|_| CallError::BadData("bt_alloc returned a buffer outside memory".into()))?;
        let mut out = [Val::I64(0)];
        self.run(
            self.handle,
            &[Val::I32(ptr), Val::I32(len)],
            &mut out,
            meter,
            between,
        )?;
        let packed = out[0].i64().unwrap_or(0) as u64;
        if packed == 0 {
            return Ok(());
        }
        let (ptr, len) = ((packed >> 32) as u32, packed as u32);
        if len as usize > MAX_RESPONSE_BYTES {
            return Err(CallError::BadData(
                "the response is larger than 64 KiB".into(),
            ));
        }
        let mut response = vec![0; len as usize];
        self.memory
            .read(&self.store, ptr as usize, &mut response)
            .map_err(|_| CallError::BadData("the response is outside memory".into()))?;
        if !response.is_empty()
            && !serde_json::from_slice::<serde_json::Value>(&response).is_ok_and(|v| v.is_object())
        {
            return Err(CallError::BadData(
                "the response is not a JSON object".into(),
            ));
        }
        let mut none = [];
        self.run(
            self.free,
            &[Val::I32(ptr as i32), Val::I32(len as i32)],
            &mut none,
            meter,
            between,
        )
    }

    /// Runs `func` in slices of [`SLICE`] fuel until it returns, its allowance is spent or its own
    /// time passes the safety net.
    fn run(
        &mut self,
        func: Func,
        inputs: &[Val],
        outputs: &mut [Val],
        meter: &mut Meter,
        between: &mut dyn FnMut(),
    ) -> Result<(), CallError> {
        let mut slice = self.start_slice(meter)?;
        let mut started = Instant::now();
        let mut call = func.call_resumable(&mut self.store, inputs, outputs);
        loop {
            meter.busy += started.elapsed();
            let left = self.store.get_fuel().unwrap_or(0);
            meter.used += slice.saturating_sub(left) + self.store.data_mut().take_charge();
            match call {
                Ok(ResumableCall::Finished) => return Ok(()),
                Ok(ResumableCall::OutOfFuel(invocation)) => {
                    if meter.used >= meter.allowance {
                        return Err(CallError::OutOfFuel);
                    }
                    if meter.busy >= meter.safety_net {
                        return Err(CallError::TimedOut);
                    }
                    between();
                    slice = self.start_slice(meter)?;
                    started = Instant::now();
                    call = invocation.resume(&mut self.store, outputs);
                }
                Ok(ResumableCall::HostTrap(trap)) => {
                    return Err(classify(&self.store, trap.into_host_error()));
                }
                Err(error) => return Err(classify(&self.store, error)),
            }
        }
    }

    fn start_slice(&mut self, meter: &Meter) -> Result<u64, CallError> {
        let slice = SLICE.min(meter.allowance.saturating_sub(meter.used));
        if slice == 0 {
            return Err(CallError::OutOfFuel);
        }
        self.store
            .set_fuel(slice)
            .map_err(|e| CallError::Trap(e.to_string()))?;
        self.store
            .data_mut()
            .begin_slice(meter.allowance, meter.used, slice);
        Ok(slice)
    }
}

/// The real sandbox: modules compiled on first use and kept while the plugin is loaded.
pub struct WasmSandbox {
    runtime: Runtime,
    modules: std::collections::HashMap<bientot_plugin_api::PluginId, Module>,
}

impl WasmSandbox {
    pub fn new(runtime: Runtime) -> WasmSandbox {
        WasmSandbox {
            runtime,
            modules: Default::default(),
        }
    }
}

impl crate::thread::Sandbox for WasmSandbox {
    type Instance = PluginInstance;

    fn activate(
        &mut self,
        spec: &crate::thread::PluginSpec,
        context: PluginContext,
    ) -> Result<PluginInstance, CallError> {
        let id = &spec.manifest.id;
        if !self.modules.contains_key(id) {
            let module = self
                .runtime
                .compile(&spec.wasm, &spec.granted)
                .map_err(|e| CallError::BadData(e.to_string()))?;
            self.modules.insert(id.clone(), module);
        }
        self.runtime.instantiate(&self.modules[id], context)
    }

    fn forget(&mut self, id: &bientot_plugin_api::PluginId) {
        self.modules.remove(id);
    }
}

impl crate::thread::Sandboxed for PluginInstance {
    fn deliver(
        &mut self,
        event: &[u8],
        limits: Limits,
        between: &mut dyn FnMut(),
    ) -> Result<Outcome, CallError> {
        PluginInstance::deliver(self, event, limits, between)
    }

    fn memory_bytes(&self) -> usize {
        PluginInstance::memory_bytes(self)
    }

    fn context_mut(&mut self) -> &mut PluginContext {
        &mut self.state_mut().context
    }
}

fn classify(store: &Store<HostState>, error: Error) -> CallError {
    if let Some(host) = error.downcast_ref::<HostError>() {
        return match host {
            HostError::OutOfFuel => CallError::OutOfFuel,
            HostError::BadRequest(detail) => CallError::BadData(detail.clone()),
            HostError::Failed(detail) => CallError::Trap(detail.clone()),
        };
    }
    if store.data().limiter.denied_by_pool {
        return CallError::PoolFull;
    }
    match error.as_trap_code() {
        Some(TrapCode::OutOfFuel) => CallError::OutOfFuel,
        _ => CallError::Trap(error.to_string()),
    }
}

#[cfg(test)]
pub(crate) mod tests;
