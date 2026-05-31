use argon2::Argon2;
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Header, Validation};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub email: String,
    pub exp: usize,
    pub iat: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredUser {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub password_salt: Vec<u8>,
    pub public_key: Vec<u8>,
    pub display_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub struct AuthManager {
    jwt_secret: String,
    users: Arc<RwLock<HashMap<String, StoredUser>>>,
}

impl AuthManager {
    pub fn new(jwt_secret: impl Into<String>) -> Self {
        AuthManager {
            jwt_secret: jwt_secret.into(),
            users: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn register(
        &self,
        email: &str,
        password: &str,
        display_name: &str,
        public_key: Vec<u8>,
    ) -> Result<String, String> {
        let mut users = self.users.write().await;
        if users.contains_key(email) {
            return Err("email already registered".into());
        }

        let id = uuid::Uuid::new_v4().to_string();
        let salt: Vec<u8> = rand::thread_rng().gen::<[u8; 32]>().to_vec();
        let mut hash = [0u8; 32];
        Argon2::default()
            .hash_password_into(password.as_bytes(), &salt, &mut hash)
            .map_err(|e| format!("hashing error: {e}"))?;

        let user = StoredUser {
            id,
            email: email.to_string(),
            password_hash: hex::encode(hash),
            password_salt: salt,
            public_key,
            display_name: display_name.to_string(),
            created_at: Utc::now(),
        };

        let token = self.generate_token(&user)?;
        users.insert(email.to_string(), user);
        Ok(token)
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<(String, StoredUser), String> {
        let users = self.users.read().await;
        let user = users
            .get(email)
            .ok_or_else(|| "invalid email or password".to_string())?;

        let mut hash = [0u8; 32];
        Argon2::default()
            .hash_password_into(password.as_bytes(), &user.password_salt, &mut hash)
            .map_err(|e| format!("hashing error: {e}"))?;

        let computed = hex::encode(hash);
        if computed != user.password_hash {
            return Err("invalid email or password".into());
        }

        let token = self.generate_token(user)?;
        Ok((token, user.clone()))
    }

    pub async fn validate_token(&self, token: &str) -> Result<Claims, String> {
        let key = jsonwebtoken::DecodingKey::from_secret(self.jwt_secret.as_bytes());
        let token_data = decode::<Claims>(token, &key, &Validation::default())
            .map_err(|e| format!("invalid token: {e}"))?;
        Ok(token_data.claims)
    }

    fn generate_token(&self, user: &StoredUser) -> Result<String, String> {
        let now = Utc::now();
        let claims = Claims {
            sub: user.id.clone(),
            email: user.email.clone(),
            iat: now.timestamp() as usize,
            exp: (now + Duration::hours(24)).timestamp() as usize,
        };
        let key = jsonwebtoken::EncodingKey::from_secret(self.jwt_secret.as_bytes());
        encode(&Header::default(), &claims, &key)
            .map_err(|e| format!("token generation error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_auth() -> AuthManager {
        AuthManager::new("test-secret-1234567890")
    }

    #[tokio::test]
    async fn test_register_and_login() {
        let auth = make_auth();
        let token = auth
            .register("alice@test.com", "pass123", "Alice", vec![1, 2, 3])
            .await
            .unwrap();
        assert!(!token.is_empty());

        let (token2, user) = auth.login("alice@test.com", "pass123").await.unwrap();
        assert_eq!(user.email, "alice@test.com");
        assert_eq!(user.display_name, "Alice");
        assert!(!token2.is_empty());
    }

    #[tokio::test]
    async fn test_register_duplicate_email() {
        let auth = make_auth();
        auth.register("dup@test.com", "pass1", "Dup", vec![])
            .await
            .unwrap();
        let err = auth
            .register("dup@test.com", "pass2", "Dup2", vec![])
            .await
            .unwrap_err();
        assert_eq!(err, "email already registered");
    }

    #[tokio::test]
    async fn test_login_wrong_password() {
        let auth = make_auth();
        auth.register("bob@test.com", "correct", "Bob", vec![4, 5, 6])
            .await
            .unwrap();
        let err = auth.login("bob@test.com", "wrong").await.unwrap_err();
        assert_eq!(err, "invalid email or password");
    }

    #[tokio::test]
    async fn test_login_nonexistent_user() {
        let auth = make_auth();
        let err = auth.login("nobody@test.com", "x").await.unwrap_err();
        assert_eq!(err, "invalid email or password");
    }

    #[tokio::test]
    async fn test_validate_token() {
        let auth = make_auth();
        let token = auth
            .register("val@test.com", "pwd", "Val", vec![7, 8, 9])
            .await
            .unwrap();
        let claims = auth.validate_token(&token).await.unwrap();
        assert_eq!(claims.email, "val@test.com");
        assert!(claims.exp > claims.iat);
    }

    #[tokio::test]
    async fn test_validate_token_invalid() {
        let auth = make_auth();
        let err = auth.validate_token("badtoken").await.unwrap_err();
        assert!(err.contains("invalid token"));
    }

    #[tokio::test]
    async fn test_register_generates_different_tokens() {
        let auth = make_auth();
        let t1 = auth
            .register("a@test.com", "pwd", "A", vec![])
            .await
            .unwrap();
        let t2 = auth
            .register("b@test.com", "pwd", "B", vec![])
            .await
            .unwrap();
        assert_ne!(t1, t2);
    }
}
