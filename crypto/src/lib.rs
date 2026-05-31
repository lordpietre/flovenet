pub mod error;

use argon2::Argon2;
use chacha20poly1305::{aead::AeadInPlace, ChaCha20Poly1305, KeyInit, Nonce};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand::{rngs::OsRng, Rng};
use serde::{Deserialize, Serialize};

pub use ed25519_dalek::{Signature, SECRET_KEY_LENGTH as ED25519_SEED_LEN};
pub use error::CryptoError;

pub type Result<T> = std::result::Result<T, CryptoError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedBlob {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub algorithm: String,
}

impl EncryptedBlob {
    pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Self> {
        let cipher = ChaCha20Poly1305::new(key.into());
        let nonce_bytes: [u8; 12] = OsRng.gen();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let mut buf = plaintext.to_vec();
        cipher
            .encrypt_in_place(nonce, &[], &mut buf)
            .map_err(|_| CryptoError::Encryption)?;
        Ok(EncryptedBlob {
            ciphertext: buf,
            nonce: nonce_bytes.to_vec(),
            algorithm: "ChaCha20-Poly1305".into(),
        })
    }

    pub fn decrypt(&self, key: &[u8; 32]) -> Result<Vec<u8>> {
        if self.algorithm != "ChaCha20-Poly1305" {
            return Err(CryptoError::UnsupportedAlgorithm(self.algorithm.clone()));
        }
        let cipher = ChaCha20Poly1305::new(key.into());
        let nonce = Nonce::from_slice(&self.nonce);
        let mut buf = self.ciphertext.clone();
        cipher
            .decrypt_in_place(nonce, &[], &mut buf)
            .map_err(|_| CryptoError::Decryption)?;
        Ok(buf)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedEnvelope {
    pub payload: Vec<u8>,
    pub signature: Vec<u8>,
    pub signer_public_key: Vec<u8>,
}

impl SignedEnvelope {
    pub fn sign(payload: &[u8], signing_key: &SigningKey) -> Self {
        let sig = signing_key.sign(payload);
        SignedEnvelope {
            payload: payload.to_vec(),
            signature: sig.to_bytes().to_vec(),
            signer_public_key: signing_key.verifying_key().to_bytes().to_vec(),
        }
    }

    pub fn verify(&self) -> Result<()> {
        let pub_key = VerifyingKey::from_bytes(
            self.signer_public_key
                .as_slice()
                .try_into()
                .map_err(|_| CryptoError::InvalidKey)?,
        )
        .map_err(|_| CryptoError::InvalidKey)?;
        let sig = Signature::from_bytes(
            self.signature
                .as_slice()
                .try_into()
                .map_err(|_| CryptoError::SignatureVerification)?,
        );
        pub_key
            .verify(&self.payload, &sig)
            .map_err(|_| CryptoError::SignatureVerification)
    }

    pub fn verify_with_key(&self, public_key: &VerifyingKey) -> Result<()> {
        let sig = Signature::from_bytes(
            self.signature
                .as_slice()
                .try_into()
                .map_err(|_| CryptoError::SignatureVerification)?,
        );
        public_key
            .verify(&self.payload, &sig)
            .map_err(|_| CryptoError::SignatureVerification)
    }
}

pub fn derive_key_from_password(password: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|_| CryptoError::Encryption)?;
    Ok(key)
}

pub fn generate_salt() -> [u8; 32] {
    OsRng.gen()
}

pub fn generate_keypair() -> SigningKey {
    let mut seed = [0u8; ED25519_SEED_LEN];
    OsRng.fill(&mut seed);
    SigningKey::from_bytes(&seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = [0x42u8; 32];
        let plaintext = b"hello flovenet";
        let blob = EncryptedBlob::encrypt(&key, plaintext).unwrap();
        let decrypted = blob.decrypt(&key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_wrong_key_fails() {
        let key_ok = [0x42u8; 32];
        let key_bad = [0xffu8; 32];
        let blob = EncryptedBlob::encrypt(&key_ok, b"test").unwrap();
        assert!(blob.decrypt(&key_bad).is_err());
    }

    #[test]
    fn test_sign_verify() {
        let signing_key = generate_keypair();
        let data = b"important message";
        let envelope = SignedEnvelope::sign(data, &signing_key);
        assert!(envelope.verify().is_ok());
    }

    #[test]
    fn test_sign_verify_tampered() {
        let signing_key = generate_keypair();
        let data = b"original";
        let mut envelope = SignedEnvelope::sign(data, &signing_key);
        envelope.payload = b"tampered".to_vec();
        assert!(envelope.verify().is_err());
    }

    #[test]
    fn test_verify_with_key() {
        let signing_key = generate_keypair();
        let verifying_key = signing_key.verifying_key();
        let data = b"verify with explicit key";
        let envelope = SignedEnvelope::sign(data, &signing_key);
        assert!(envelope.verify_with_key(&verifying_key).is_ok());
    }

    #[test]
    fn test_sign_verify_wrong_key() {
        let signing_key = generate_keypair();
        let wrong_key = generate_keypair().verifying_key();
        let data = b"wrong key test";
        let envelope = SignedEnvelope::sign(data, &signing_key);
        assert!(envelope.verify_with_key(&wrong_key).is_err());
    }

    #[test]
    fn test_password_derivation() {
        let password = "supersecret123";
        let salt = generate_salt();
        let key1 = derive_key_from_password(password, &salt).unwrap();
        let key2 = derive_key_from_password(password, &salt).unwrap();
        assert_eq!(key1, key2);

        let wrong_password = derive_key_from_password("wrong", &salt).unwrap();
        assert_ne!(key1, wrong_password);

        let different_salt = generate_salt();
        let diff_salt_key = derive_key_from_password(password, &different_salt).unwrap();
        assert_ne!(key1, diff_salt_key);
    }

    #[test]
    fn test_signed_envelope_serde() {
        let signing_key = generate_keypair();
        let data = b"serde roundtrip";
        let envelope = SignedEnvelope::sign(data, &signing_key);
        let json = serde_json::to_string(&envelope).unwrap();
        let deserialized: SignedEnvelope = serde_json::from_str(&json).unwrap();
        assert!(deserialized.verify().is_ok());
    }
}
