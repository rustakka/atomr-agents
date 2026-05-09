//! Process-wide tokio runtime used for `future_into_py` async
//! adapters. Initialized lazily on first access — most callers go
//! through `pyo3_async_runtimes::tokio::future_into_py`, but the
//! shared runtime is also needed for direct `block_on` usage from
//! sync constructors that internally await small async setup steps.

use once_cell::sync::OnceCell;
use tokio::runtime::Runtime;

static RT: OnceCell<Runtime> = OnceCell::new();

pub fn shared() -> &'static Runtime {
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("atomr-agents-py")
            .build()
            .expect("failed to start atomr-agents-py tokio runtime")
    })
}
