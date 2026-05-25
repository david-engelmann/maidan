//! Phantom path handlers for `utoipa` (not called at runtime).

#![allow(dead_code, unused_imports)]

mod api;
mod health;

pub use api::*;
pub use health::*;
