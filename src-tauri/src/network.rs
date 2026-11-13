//! Tells the engine when Windows reports a change of network connection (a connection lost or back).
//!
//! The handler only calls `on_change`, which must be cheap: the engine then decides what to do.

use windows::core::Result;
use windows::Networking::Connectivity::{NetworkInformation, NetworkStatusChangedEventHandler};

pub struct NetworkWatcher(i64);

impl NetworkWatcher {
    /// `on_change` is called from a WinRT thread. Call this on a thread that is already in the MTA.
    pub fn start(on_change: impl Fn() + Send + 'static) -> Result<NetworkWatcher> {
        let handler = NetworkStatusChangedEventHandler::new(move |_| {
            on_change();
            Ok(())
        });
        NetworkInformation::NetworkStatusChanged(&handler).map(NetworkWatcher)
    }
}

impl Drop for NetworkWatcher {
    fn drop(&mut self) {
        let _ = NetworkInformation::RemoveNetworkStatusChanged(self.0);
    }
}
