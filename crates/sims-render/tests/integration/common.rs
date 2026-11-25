//! Helpers the integration test modules share.

use sims_content::Install;
use std::path::PathBuf;

/// The install `SIMS_INSTALL` names, or `None` to skip when it is unset. A
/// set variable that is not an install fails the test.
pub fn install() -> Option<Install> {
    let root = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    Some(
        Install::detect(&root).expect("SIMS_INSTALL should be a valid Complete Collection install"),
    )
}
