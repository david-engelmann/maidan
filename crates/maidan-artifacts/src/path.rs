//! Shared object-key layout for content-addressed backends.

use crate::sha::Sha256;

/// S3/LocalFs key: `<sha[0:2]>/<sha[2:4]>/<sha[4:]>`.
pub fn object_key(sha: &Sha256) -> String {
    let hex = sha.to_hex();
    format!("{}/{}/{}", &hex[0..2], &hex[2..4], &hex[4..])
}
