//! Phantom path handlers for `utoipa` (not called at runtime).

#![allow(dead_code, unused_imports)]

mod api;
mod auth;
mod extensions;
mod health;
mod room;

pub use api::*;
pub use auth::*;
pub use extensions::*;
pub use health::*;
pub use room::*;
