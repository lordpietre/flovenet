use crypto::{
    derive_key_from_password, generate_keypair, generate_salt, CryptoError, EncryptedBlob,
    SignedEnvelope,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::PeerId;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeyStoreData {
    salt: Vec<u8>,
    encrypted_seed: EncryptedBlob,
    public_key: Vec<u8>,
}

pub struct KeyStore {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
    peer_id: PeerId,
    password: String,
    salt: Vec<u8>,
    path: Option<std::path::PathBuf>,
}

impl KeyStore {
    pub fn new(password: &str) -> Self {
        let signing_key = generate_keypair();
        let verifying_key = signing_key.verifying_key();
        let pub_key_bytes = verifying_key.to_bytes().to_vec();
        let peer_id = PeerId::from_public_key_bytes(&pub_key_bytes);
        let salt = generate_salt().to_vec();

        KeyStore {
            signing_key,
            verifying_key,
            peer_id,
            password: password.to_string(),
            salt,
            path: None,
        }
    }

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    pub fn public_key(&self) -> &VerifyingKey {
        &self.verifying_key
    }

    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.verifying_key.to_bytes().to_vec()
    }

    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    pub fn sign(&self, data: &[u8]) -> SignedEnvelope {
        SignedEnvelope::sign(data, &self.signing_key)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), CryptoError> {
        let key = derive_key_from_password(&self.password, &self.salt)
            .map_err(|_| CryptoError::Encryption)?;
        let seed_bytes = self.signing_key.to_bytes();
        let encrypted = EncryptedBlob::encrypt(&key, &seed_bytes)?;

        let data = KeyStoreData {
            salt: self.salt.clone(),
            encrypted_seed: encrypted,
            public_key: self.verifying_key.to_bytes().to_vec(),
        };

        let json = serde_json::to_string(&data).map_err(|_| CryptoError::Encryption)?;
        std::fs::write(path.as_ref(), json).map_err(|_| CryptoError::Encryption)?;
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>, password: &str) -> Result<Self, CryptoError> {
        let json = std::fs::read_to_string(path.as_ref()).map_err(|_| CryptoError::KeyNotFound)?;
        let data: KeyStoreData =
            serde_json::from_str(&json).map_err(|_| CryptoError::Decryption)?;

        let key = derive_key_from_password(password, &data.salt)?;
        let seed_bytes = data.encrypted_seed.decrypt(&key)?;

        let seed_array: [u8; 32] = seed_bytes
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::InvalidKey)?;
        let signing_key = SigningKey::from_bytes(&seed_array);
        let verifying_key = signing_key.verifying_key();

        let expected_pub = verifying_key.to_bytes();
        if expected_pub.as_slice() != data.public_key.as_slice() {
            return Err(CryptoError::SignatureVerification);
        }

        let peer_id = PeerId::from_public_key_bytes(&expected_pub);

        Ok(KeyStore {
            signing_key,
            verifying_key,
            peer_id,
            password: password.to_string(),
            salt: data.salt,
            path: Some(path.as_ref().to_path_buf()),
        })
    }

    pub fn change_password(&mut self, new_password: &str) {
        self.password = new_password.to_string();
    }

    pub fn has_path(&self) -> bool {
        self.path.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keystore_create_and_peer_id() {
        let ks = KeyStore::new("test-password");
        assert!(!ks.peer_id().0.is_empty());
        assert!(ks.peer_id().0.starts_with("bafk"));
    }

    #[test]
    fn test_keystore_sign_and_verify() {
        let ks = KeyStore::new("test-password");
        let envelope = ks.sign(b"hello world");
        assert!(envelope.verify_with_key(ks.public_key()).is_ok());
    }

    #[test]
    fn test_keystore_save_and_load() {
        let tmp = std::env::temp_dir().join("test_keystore.json");
        let password = "correct-horse-battery-staple";

        {
            let ks = KeyStore::new(password);
            ks.save(&tmp).unwrap();
        }

        let loaded = KeyStore::load(&tmp, password).unwrap();
        assert!(!loaded.peer_id().0.is_empty());

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_keystore_wrong_password_fails() {
        let tmp = std::env::temp_dir().join("test_keystore_wrong_pw.json");

        {
            let ks = KeyStore::new("real-password");
            ks.save(&tmp).unwrap();
        }

        let result = KeyStore::load(&tmp, "wrong-password");
        assert!(result.is_err());

        let _ = std::fs::remove_file(&tmp);
    }
}
