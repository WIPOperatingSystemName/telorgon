use super::*;

pub enum NetworkDispatch {
    /// The provider has read back the resulting state, not merely received a method reply.
    Complete(NetworkResult),
    /// Provider-owned token, valid until completion or abandon().
    Pending(u64),
}
/// Trusted embedding boundary. Calls run serially on one I/O worker, never the UI thread.
/// IDs must describe runtime incarnations; backend restarts invalidate all existing IDs.
/// Native calls must be bounded. Errors can follow partial changes and do not imply rollback.
pub trait NetworkProvider: Send + 'static {
    fn snapshot(&mut self) -> Result<NetworkSnapshot, NetworkError>;
    fn execute(&mut self, command: NetworkCommand) -> Result<NetworkDispatch, NetworkError>;
    fn poll(
        &mut self,
        token: u64,
        snapshot: &NetworkSnapshot,
    ) -> Result<Option<NetworkResult>, NetworkError>;
    /// Stops tracking completion; must not undo dispatched work or disconnect a network.
    fn abandon(&mut self, token: u64);
    /// Drain native change notifications without blocking; periodic refresh remains a fallback.
    fn has_changes(&mut self) -> bool {
        false
    }
}
