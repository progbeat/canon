//! One evaluator-thread interaction lifecycle.
//!
//! `lifecycle` owns selecting, starting, asking, and recovery. `state` owns
//! invocation-local thread state and its reuse indexes. `answer` assembles an
//! expectation answer, while `model` defines their shared request contracts.

mod answer;
mod lifecycle;
mod model;
mod state;

pub(crate) use answer::{interrogate_expectation_answer_with_model, resolve_diff_from};
pub(crate) use lifecycle::ask_thread_turn;
use model::ThreadSelection;
pub(crate) use model::{ThreadTurnContext, ThreadTurnRequest, ThreadTurnResponseContract};
pub(super) use state::ThreadState;
