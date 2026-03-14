pub mod client;
mod connection;
mod error;

#[cfg(test)]
pub mod integration_tests;

pub use client::{DocumentIdentifier, NeovimClient, NeovimClientTrait, Position, string_or_struct};

pub use error::NeovimError;
