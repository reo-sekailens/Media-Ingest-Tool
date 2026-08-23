//! Fail-closed format authorization. This module never formats media itself.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatAuthorization {
    pub token: String,
    pub medium_key: String,
    pub generation: u64,
    pub expires_at: SystemTime,
    pub run_id: String,
    pub profile_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatGateError {
    UnverifiedRun,
    WeakIdentity,
    DeviceChanged,
    Expired,
    AlreadyConsumed,
}

pub fn issue_authorization(
    medium_key: &str,
    generation: u64,
    run_id: &str,
    profile_id: &str,
    verified: bool,
    strong_identity: bool,
    now: SystemTime,
) -> Result<FormatAuthorization, FormatGateError> {
    if !verified {
        return Err(FormatGateError::UnverifiedRun);
    }
    if !strong_identity {
        return Err(FormatGateError::WeakIdentity);
    }
    if run_id.trim().is_empty() || profile_id.trim().is_empty() {
        return Err(FormatGateError::UnverifiedRun);
    }
    Ok(FormatAuthorization {
        token: Uuid::new_v4().to_string(),
        medium_key: medium_key.into(),
        generation,
        expires_at: now + Duration::from_secs(60),
        run_id: run_id.into(),
        profile_id: profile_id.into(),
    })
}

pub fn validate_authorization(
    authorization: &FormatAuthorization,
    current_medium_key: &str,
    current_generation: u64,
    now: SystemTime,
    consumed: bool,
) -> Result<(), FormatGateError> {
    if consumed {
        return Err(FormatGateError::AlreadyConsumed);
    }
    if now > authorization.expires_at {
        return Err(FormatGateError::Expired);
    }
    if authorization.medium_key != current_medium_key
        || authorization.generation != current_generation
    {
        return Err(FormatGateError::DeviceChanged);
    }
    Ok(())
}

/// Removes one authorization before handing it to a platform formatter. A
/// failed revalidation consumes the token too: confirmation is never
/// replayable after a device-generation mismatch or timeout.
pub fn consume_authorization(
    authorizations: &mut HashMap<String, FormatAuthorization>,
    token: &str,
    current_medium_key: &str,
    current_generation: u64,
    now: SystemTime,
) -> Result<FormatAuthorization, FormatGateError> {
    let authorization = authorizations
        .remove(token)
        .ok_or(FormatGateError::AlreadyConsumed)?;
    validate_authorization(
        &authorization,
        current_medium_key,
        current_generation,
        now,
        false,
    )?;
    Ok(authorization)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_authorization_is_short_lived_and_generation_bound() {
        let now = SystemTime::now();
        let authorization = issue_authorization(
            "v1:strong-card",
            7,
            "run-id",
            "sdxc-default",
            true,
            true,
            now,
        )
        .expect("authorization");
        assert!(validate_authorization(&authorization, "v1:strong-card", 7, now, false).is_ok());
        assert_eq!(
            validate_authorization(&authorization, "v1:strong-card", 8, now, false),
            Err(FormatGateError::DeviceChanged)
        );
        assert_eq!(
            validate_authorization(
                &authorization,
                "v1:strong-card",
                7,
                now + Duration::from_secs(61),
                false
            ),
            Err(FormatGateError::Expired)
        );
    }

    #[test]
    fn consumption_is_single_use_even_when_revalidation_rejects_the_device() {
        let now = SystemTime::now();
        let authorization = issue_authorization(
            "v1:strong-card",
            7,
            "run-id",
            "sdxc-default",
            true,
            true,
            now,
        )
        .expect("authorization");
        let token = authorization.token.clone();
        let mut authorizations = HashMap::from([(token.clone(), authorization)]);
        assert_eq!(
            consume_authorization(&mut authorizations, &token, "v1:strong-card", 8, now),
            Err(FormatGateError::DeviceChanged)
        );
        assert_eq!(
            consume_authorization(&mut authorizations, &token, "v1:strong-card", 7, now),
            Err(FormatGateError::AlreadyConsumed)
        );
    }

    #[test]
    fn consumption_returns_only_the_exact_current_authorization_once() {
        let now = SystemTime::now();
        let authorization = issue_authorization(
            "v1:strong-card",
            7,
            "run-id",
            "sdxc-default",
            true,
            true,
            now,
        )
        .expect("authorization");
        let token = authorization.token.clone();
        let mut authorizations = HashMap::from([(token.clone(), authorization.clone())]);
        assert_eq!(
            consume_authorization(&mut authorizations, &token, "v1:strong-card", 7, now),
            Ok(authorization)
        );
        assert!(authorizations.is_empty());
    }
}
