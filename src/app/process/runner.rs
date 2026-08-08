//! App-server runner lifecycle, protocol transport, and telemetry.

mod child;
mod evaluator;
mod lazy;
mod spawn;
mod state;
mod transport;

use state::AppServerRunner;

pub(crate) use lazy::LazyAppServerRunner;
