pub mod discovery;
pub mod router;

#[cfg(feature = "fast")]
pub mod fast;

#[cfg(feature = "a2a")]
pub mod a2a;

#[cfg(feature = "transport-log")]
pub mod log;

pub use discovery::Discovery;
pub use router::{AgentEndpoint, Router};

#[cfg(feature = "transport-log")]
pub use log::TransportLogger;
