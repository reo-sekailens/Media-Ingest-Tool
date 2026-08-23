//! Conservative identity normalization and key derivation.
//!
//! A derived key is an opaque comparison key, not proof that its input is
//! immutable. Callers must retain the evidence and its scope alongside it.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const IDENTITY_ENCODING_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityScope {
    Reader,
    Medium,
    Partition,
    Filesystem,
    Topology,
    Session,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityStrength {
    HardwareStrong,
    HardwareReported,
    Filesystem,
    Topology,
    Session,
    Ambiguous,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityCandidate {
    pub source: String,
    pub scope: IdentityScope,
    pub original_value: String,
    pub normalized_value: String,
    pub strength: IdentityStrength,
}

impl IdentityCandidate {
    pub fn new(
        source: impl Into<String>,
        scope: IdentityScope,
        value: impl Into<String>,
        strength: IdentityStrength,
    ) -> Option<Self> {
        let original_value = value.into();
        let normalized_value = normalize_identifier(&original_value);
        (!normalized_value.is_empty()).then(|| Self {
            source: source.into(),
            scope,
            original_value,
            normalized_value,
            strength,
        })
    }
}

pub fn normalize_identifier(value: &str) -> String {
    value
        .trim_matches(|character: char| character.is_whitespace() || character == '\0')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

pub fn derive_key(scope: IdentityScope, candidate: &IdentityCandidate) -> String {
    let canonical = format!(
        "v{IDENTITY_ENCODING_VERSION}\u{001f}{scope:?}\u{001f}{}\u{001f}{}",
        candidate.source.to_uppercase(),
        candidate.normalized_value
    );
    let digest = Sha256::digest(canonical.as_bytes());
    format!("v{IDENTITY_ENCODING_VERSION}:{}", hex::encode(digest))
}

pub fn allows_automatic_medium_recall(candidates: &[IdentityCandidate]) -> bool {
    candidates.iter().any(|candidate| {
        candidate.scope == IdentityScope::Medium
            && matches!(
                candidate.strength,
                IdentityStrength::HardwareStrong | IdentityStrength::HardwareReported
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_removes_transport_padding_without_synthesizing_identity() {
        assert_eq!(normalize_identifier("\0  sd  123 \0"), "SD 123");
        assert_eq!(normalize_identifier("   \0"), "");
    }

    #[test]
    fn topology_cannot_authorize_medium_recall() {
        let topology = IdentityCandidate::new(
            "windows.location-path",
            IdentityScope::Topology,
            "USBROOT(0)#USB(3)",
            IdentityStrength::Topology,
        )
        .expect("candidate");
        assert!(!allows_automatic_medium_recall(&[topology]));
    }

    #[test]
    fn canonical_key_is_stable_across_padding_and_case() {
        let first = IdentityCandidate::new(
            "vpd.naa",
            IdentityScope::Medium,
            " naa-123  ",
            IdentityStrength::HardwareStrong,
        )
        .expect("candidate");
        let second = IdentityCandidate::new(
            "VPD.NAA",
            IdentityScope::Medium,
            "NAA-123",
            IdentityStrength::HardwareStrong,
        )
        .expect("candidate");
        assert_eq!(
            derive_key(IdentityScope::Medium, &first),
            derive_key(IdentityScope::Medium, &second)
        );
    }
}
