//! Client for Maidan. 0.0.1 is a name reservation; the API is not stable.

#[derive(Clone, Debug)]
pub struct Client {
    pub base_url: String,
    pub token: String,
}

impl Client {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: token.into(),
        }
    }
}
