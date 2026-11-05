//! OpenID Connect login flow (authorization code + PKCE).

mod config;
mod handlers;
mod invite;
mod member;

pub use config::{OidcInitError, OidcRuntime, OidcSettings};
pub use handlers::{
    callback, login, logout, signup, signup_form, SESSION_CREATE, SESSION_DELETE, WORKSPACE_SIGNUP,
};
pub(crate) use handlers::{STARTER_TASK_DESCRIPTION, STARTER_TASK_MESSAGE, STARTER_TASK_TITLE};
pub use member::{resolve_member_for_login, LoginMember, NOT_PROVISIONED};
