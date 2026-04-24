//! GrowthBook SDK keys. Port of `src/constants/keys.ts`.
//!
//! TS picks the key from `USER_TYPE` and `ENABLE_GROWTHBOOK_DEV` at
//! runtime. We do the same so developer and external builds hit
//! separate GrowthBook environments.

use crate::errors_util::is_env_truthy;
use crate::user_type;

/// Return the GrowthBook client SDK key for the current user type.
pub fn get_growthbook_client_key() -> &'static str {
    if user_type::is_ant() {
        if is_env_truthy("ENABLE_GROWTHBOOK_DEV") {
            "sdk-yZQvlplybuXjYh6L"
        } else {
            "sdk-xRVcrliHIlrg4og4"
        }
    } else {
        "sdk-zAZezfDKGoZuXXKe"
    }
}

#[cfg(test)]
mod tests {
    use super::super::ENV_LOCK;
    use super::*;

    #[test]
    fn external_user_gets_external_key() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("USER_TYPE");
        std::env::remove_var("ENABLE_GROWTHBOOK_DEV");
        assert_eq!(get_growthbook_client_key(), "sdk-zAZezfDKGoZuXXKe");
    }

    #[test]
    fn ant_user_gets_prod_key_by_default() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("USER_TYPE", "ant");
        std::env::remove_var("ENABLE_GROWTHBOOK_DEV");
        assert_eq!(get_growthbook_client_key(), "sdk-xRVcrliHIlrg4og4");
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn ant_dev_gets_dev_key() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("USER_TYPE", "ant");
        std::env::set_var("ENABLE_GROWTHBOOK_DEV", "1");
        assert_eq!(get_growthbook_client_key(), "sdk-yZQvlplybuXjYh6L");
        std::env::remove_var("USER_TYPE");
        std::env::remove_var("ENABLE_GROWTHBOOK_DEV");
    }
}
