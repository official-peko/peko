//! A rule database that came over the network, and how to refuse it.
//!
//! The binary ships with a database and can fetch a newer one. That fetch is
//! the dangerous part: whoever controls the response controls what the tool
//! reports. A rule that vanishes reports a pass. A rule that appears reports a
//! failure on working code. Neither looks like an attack.
//!
//! So a fetched database is used only when an Ed25519 signature over its exact
//! bytes verifies against the key compiled into this binary. There is no
//! setting to skip it and no fallback to unverified bytes. A database that
//! fails any check is dropped, and the one that shipped with the binary is
//! used instead.
//!
//! Three checks, and each one exists because skipping it is exploitable.
//!
//! The signature must verify. Without that anybody on the path writes the
//! rules.
//!
//! The version must not go backwards. A signature stays valid forever, so an
//! attacker who cannot forge one can still replay a real database from a year
//! ago and take away every rule added since.
//!
//! The bytes verified must be the bytes parsed. Verifying one buffer and
//! parsing another is the oldest way to pass a signature check and still run
//! something else.

use crate::db::RuleDatabase;
use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};

/// The key that signs a published rule database.
///
/// The private half never leaves the machine that publishes. Replacing this
/// constant is a release of a new binary, on purpose: a key that can be
/// changed at runtime is a key an attacker can change.
pub const PUBLIC_KEY_HEX: &str = "7a2060079191a158181f892964c34bba4eaec92ed0c60f9b6111adb07eac63b1";

/// Why a fetched database was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The signature is not 64 bytes, or the key is not 32.
    Malformed(String),
    /// The signature does not match these bytes and this key.
    BadSignature,
    /// The bytes verified, and they are not a rule database.
    NotADatabase(String),
    /// This binary was built before a signing key existed.
    ///
    /// It refuses every fetched database rather than accept one nobody signed.
    NoKeyConfigured,
    /// The database is older than the one already in hand.
    ///
    /// A signature does not expire, so an old database stays verifiable after
    /// every rule it lacks was added.
    Older { offered: String, held: String },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(why) => write!(formatter, "the signature or key is malformed: {why}"),
            Self::NoKeyConfigured => write!(
                formatter,
                "this build carries no signing key, so it trusts no fetched database"
            ),
            Self::BadSignature => write!(
                formatter,
                "the signature does not match. The database was not published by this key"
            ),
            Self::NotADatabase(why) => {
                write!(formatter, "the signed bytes are not a rule database: {why}")
            }
            Self::Older { offered, held } => write!(
                formatter,
                "the offered database is {offered} and the one in hand is {held}. \
                 A signature stays valid forever, so an older one may be a replay"
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// Read the key this binary trusts.
///
/// # Errors
///
/// Returns a refusal when the compiled key is not 32 hex-encoded bytes.
pub fn public_key() -> Result<VerifyingKey, Refusal> {
    // An all zero key is a valid curve point, so parsing it succeeds and
    // proves nothing. The placeholder has to be recognised by name, or a build
    // made before the key existed would accept whatever it was handed.
    if PUBLIC_KEY_HEX.trim().bytes().all(|b| b == b'0') {
        return Err(Refusal::NoKeyConfigured);
    }
    key_from_hex(PUBLIC_KEY_HEX)
}

/// Read a verifying key from hex.
///
/// # Errors
///
/// Returns a refusal when the text is not 32 hex-encoded bytes.
pub fn key_from_hex(text: &str) -> Result<VerifyingKey, Refusal> {
    let bytes = hex::decode(text.trim())
        .map_err(|error| Refusal::Malformed(format!("the key is not hex: {error}")))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| Refusal::Malformed("the key is not 32 bytes".to_string()))?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|error| Refusal::Malformed(format!("the key is not a point: {error}")))
}

/// Verify a signature over `payload`, and build the database it holds.
///
/// `held` is the version already in hand, usually the one compiled into the
/// binary. The offered database must not be older.
///
/// # Errors
///
/// Returns a refusal when the signature fails, the payload is not a database,
/// or the offered version goes backwards.
pub fn verify(
    payload: &[u8],
    signature: &[u8],
    key: &VerifyingKey,
    held: &semver::Version,
) -> Result<RuleDatabase, Refusal> {
    let signature: [u8; 64] = signature
        .try_into()
        .map_err(|_| Refusal::Malformed("the signature is not 64 bytes".to_string()))?;
    let signature = Signature::from_bytes(&signature);
    key.verify(payload, &signature)
        .map_err(|_| Refusal::BadSignature)?;

    // Parse the same buffer that was verified. Reading the file again here, or
    // taking a caller's already parsed value, is how a signature check ends up
    // covering bytes that nothing runs.
    let bundle: Bundle = serde_json::from_slice(payload)
        .map_err(|error| Refusal::NotADatabase(error.to_string()))?;
    let database = RuleDatabase::new(bundle.manifest, bundle.rules)
        .map_err(|error| Refusal::NotADatabase(error.to_string()))?;

    if database.version() < held {
        return Err(Refusal::Older {
            offered: database.version().to_string(),
            held: held.to_string(),
        });
    }
    Ok(database)
}

/// What a published database file holds.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Bundle {
    pub manifest: crate::db::DatabaseManifest,
    pub rules: Vec<crate::schema::Rule>,
}

impl Bundle {
    /// The bytes that get signed.
    ///
    /// The signature covers exactly what this returns, so a caller must sign
    /// and publish the same buffer. Re-serialising between the two steps can
    /// reorder a map and break every signature.
    ///
    /// # Errors
    ///
    /// Returns an error when the bundle will not serialise.
    pub fn payload(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self).map(|mut bytes| {
            bytes.push(b'\n');
            bytes
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    fn keypair() -> SigningKey {
        // A fixed seed, so the test signs and verifies the same way every run.
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn bundle(version: &str) -> Bundle {
        let manifest: crate::db::DatabaseManifest = serde_json::from_value(serde_json::json!({
            "database_version": version,
            "schema_version": 1,
            "updated_at": "2026-09-04T00:00:00Z",
            "description": "a test database",
        }))
        .expect("the manifest parses");
        let rules: Vec<crate::schema::Rule> =
            serde_json::from_str(crate::embedded::RULE_FILES[0].1).expect("rules parse");
        Bundle { manifest, rules }
    }

    fn signed(version: &str) -> (Vec<u8>, Vec<u8>, VerifyingKey) {
        let key = keypair();
        let payload = bundle(version).payload().expect("serialises");
        let signature = key.sign(&payload).to_bytes().to_vec();
        (payload, signature, key.verifying_key())
    }

    fn held(version: &str) -> semver::Version {
        semver::Version::parse(version).expect("a version")
    }

    #[test]
    fn a_signed_database_verifies() {
        let (payload, signature, key) = signed("1.0.0");
        let database = verify(&payload, &signature, &key, &held("1.0.0")).expect("verifies");
        assert!(!database.is_empty());
    }

    #[test]
    fn one_changed_byte_fails() {
        // Whoever controls the response controls what the tool reports. A rule
        // that vanishes reports a pass on a project that is not passing.
        let (mut payload, signature, key) = signed("1.0.0");
        let last = payload.len() - 2;
        payload[last] ^= 0x01;
        assert_eq!(
            verify(&payload, &signature, &key, &held("1.0.0")).unwrap_err(),
            Refusal::BadSignature
        );
    }

    #[test]
    fn another_key_fails() {
        let (payload, signature, _) = signed("1.0.0");
        let other = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
        assert_eq!(
            verify(&payload, &signature, &other, &held("1.0.0")).unwrap_err(),
            Refusal::BadSignature
        );
    }

    #[test]
    fn an_older_database_is_refused_even_with_a_good_signature() {
        // This is the one a signature alone does not catch. A signature stays
        // valid forever, so an attacker who cannot forge one can still replay
        // a real database from a year ago and take away every rule since.
        let (payload, signature, key) = signed("1.0.0");
        let error = verify(&payload, &signature, &key, &held("2.0.0")).unwrap_err();
        assert!(matches!(error, Refusal::Older { .. }), "{error:?}");
    }

    #[test]
    fn the_same_version_is_allowed() {
        // A re-publish at the same version is normal, and refusing it would
        // make a client stick on a database it already has for no reason.
        let (payload, signature, key) = signed("1.2.3");
        assert!(verify(&payload, &signature, &key, &held("1.2.3")).is_ok());
    }

    #[test]
    fn a_signature_of_the_wrong_length_is_refused_before_anything_else() {
        let (payload, _, key) = signed("1.0.0");
        let error = verify(&payload, &[0u8; 8], &key, &held("1.0.0")).unwrap_err();
        assert!(matches!(error, Refusal::Malformed(_)), "{error:?}");
    }

    #[test]
    fn signed_bytes_that_are_not_a_database_are_refused() {
        // A valid signature over rubbish is still rubbish. The check is on the
        // bytes, so the parse has to happen after it and on the same buffer.
        let key = keypair();
        let payload = b"{\"not\":\"a database\"}".to_vec();
        let signature = key.sign(&payload).to_bytes().to_vec();
        let error = verify(&payload, &signature, &key.verifying_key(), &held("0.0.1")).unwrap_err();
        assert!(matches!(error, Refusal::NotADatabase(_)), "{error:?}");
    }

    #[test]
    fn the_compiled_key_is_read_as_hex() {
        // An all zero key is a valid curve point, so parsing it succeeds and
        // proves nothing. A build made before the key existed would trust
        // whatever it was handed, which is why the placeholder is recognised
        // by name rather than by failing to parse.
        //
        // This build carries a real key, so the check is that it reads.
        public_key().expect("the compiled key reads");
        assert!(
            !PUBLIC_KEY_HEX.bytes().all(|b| b == b'0'),
            "this build still carries the placeholder key"
        );
    }

    #[test]
    fn a_key_that_is_not_hex_is_refused() {
        assert!(matches!(
            key_from_hex("not hex at all").unwrap_err(),
            Refusal::Malformed(_)
        ));
        assert!(matches!(
            key_from_hex("aabb").unwrap_err(),
            Refusal::Malformed(_)
        ));
    }

    #[test]
    fn the_payload_is_stable_across_calls() {
        // The signature covers exactly these bytes. If serialising twice gave
        // two buffers, every signature would break for no visible reason.
        let one = bundle("1.0.0").payload().expect("serialises");
        let two = bundle("1.0.0").payload().expect("serialises");
        assert_eq!(one, two);
    }
}
