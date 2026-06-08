pub mod codec;
pub mod unix;

pub use codec::{FastCodec, FastMessage};
pub use unix::{serve, FastClient};
