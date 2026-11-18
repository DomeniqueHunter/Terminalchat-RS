//! Protocol library for the F-Chat chat server of f-list.net.

pub mod connection;
pub mod error;
pub mod login;
pub mod opcode;
pub mod profile;

pub use connection::{Connection, Event, Identity, ServerMessage};
pub use error::{Error, Result};
pub use login::{fetch_ticket, LoginResult};
pub use profile::{fetch_profile, Profile};
