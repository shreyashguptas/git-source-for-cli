pub mod client;
pub mod detect;

pub use client::{generate_stream, list_models};
pub use detect::{detect, Availability};
