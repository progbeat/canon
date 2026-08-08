use super::thread::ThreadState;
use crate::platform::filesystem::PrivateTemporaryDirectoryAllocator;

pub(crate) struct InterrogationSession {
    thread_state: ThreadState,
}

impl InterrogationSession {
    pub(crate) fn new(
        disable_session_isolation: bool,
        temporary_directory_allocator: PrivateTemporaryDirectoryAllocator,
    ) -> Result<Self, String> {
        Ok(Self {
            thread_state: ThreadState::new(
                disable_session_isolation,
                temporary_directory_allocator,
            )?,
        })
    }

    pub(crate) fn clear_threads(&mut self) {
        self.thread_state.clear_threads();
    }

    pub(in crate::check::interrogation::session) fn discard_thread(&mut self, thread_id: &str) {
        self.thread_state.discard_thread(thread_id);
    }

    pub(in crate::check::interrogation::session) fn thread_state(&self) -> &ThreadState {
        &self.thread_state
    }

    pub(in crate::check::interrogation::session) fn thread_state_mut(
        &mut self,
    ) -> &mut ThreadState {
        &mut self.thread_state
    }
}
