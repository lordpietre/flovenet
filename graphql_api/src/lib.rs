pub mod auth;
pub mod schema;

use std::collections::HashMap;
use std::sync::Arc;

use async_graphql::{Request, Response, Schema};
use axum::{
    extract::Extension,
    http::{Method, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use crate::schema::{
    AuthPayload, GatewayEvent, MutationRoot, Post, Profile, QueryRoot, SubscriptionRoot,
};
use crypto::generate_keypair;
use identity::PeerId;

pub type GatewaySchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;

#[derive(Clone, Default)]
pub struct InMemoryStore {
    profiles: Arc<RwLock<HashMap<String, Profile>>>,
    posts: Arc<RwLock<HashMap<String, Post>>>,
    followers: Arc<RwLock<HashMap<String, Vec<String>>>>,
    following: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

impl InMemoryStore {
    pub async fn save_profile(&self, peer_id: &str, profile: Profile) {
        let mut profiles = self.profiles.write().await;
        profiles.insert(peer_id.to_string(), profile);
    }

    pub async fn get_profile(&self, peer_id: &str) -> Option<Profile> {
        let profiles = self.profiles.read().await;
        profiles.get(peer_id).cloned()
    }

    pub async fn get_profiles(&self) -> Vec<Profile> {
        let profiles = self.profiles.read().await;
        profiles.values().cloned().collect()
    }

    pub async fn save_post(&self, cid: &str, post: Post) {
        let mut posts = self.posts.write().await;
        posts.insert(cid.to_string(), post);
    }

    pub async fn get_post(&self, cid: &str) -> Option<Post> {
        let posts = self.posts.read().await;
        posts.get(cid).cloned()
    }

    pub async fn get_posts(&self) -> Vec<Post> {
        let posts = self.posts.read().await;
        let mut all: Vec<Post> = posts.values().cloned().collect();
        all.sort_by_key(|b| std::cmp::Reverse(b.timestamp));
        all
    }

    pub async fn delete_post(&self, cid: &str) -> bool {
        let mut posts = self.posts.write().await;
        posts.remove(cid).is_some()
    }

    pub async fn follow(&self, follower: &str, followee: &str) {
        {
            let mut follow = self.followers.write().await;
            follow
                .entry(followee.to_string())
                .or_default()
                .push(follower.to_string());
        }
        {
            let mut follow = self.following.write().await;
            follow
                .entry(follower.to_string())
                .or_default()
                .push(followee.to_string());
        }
    }

    pub async fn unfollow(&self, follower: &str, followee: &str) {
        {
            let mut follow = self.followers.write().await;
            if let Some(f) = follow.get_mut(followee) {
                f.retain(|x| x != follower);
            }
        }
        {
            let mut follow = self.following.write().await;
            if let Some(f) = follow.get_mut(follower) {
                f.retain(|x| x != followee);
            }
        }
    }

    pub async fn get_followers(&self, peer_id: &str) -> Vec<String> {
        let follow = self.followers.read().await;
        follow.get(peer_id).cloned().unwrap_or_default()
    }

    pub async fn get_following(&self, peer_id: &str) -> Vec<String> {
        let follow = self.following.read().await;
        follow.get(peer_id).cloned().unwrap_or_default()
    }
}

pub struct AppState {
    pub auth: auth::AuthManager,
    pub event_tx: broadcast::Sender<GatewayEvent>,
    pub store: InMemoryStore,
}

impl AppState {
    pub async fn get_profile(&self, peer_id: &str) -> Option<Profile> {
        self.store.get_profile(peer_id).await
    }

    pub async fn get_post(&self, cid: &str) -> Option<Post> {
        self.store.get_post(cid).await
    }

    pub async fn get_feed(&self, limit: usize, offset: usize) -> Vec<schema::FeedItem> {
        let posts = self.store.get_posts().await;
        let mut items = Vec::with_capacity(limit);
        for post in posts.into_iter().skip(offset).take(limit) {
            let author = self
                .store
                .get_profile(&post.author)
                .await
                .unwrap_or_else(|| Profile {
                    peer_id: post.author.clone(),
                    display_name: post.author.clone(),
                    bio: None,
                    avatar_cid: None,
                    follower_count: 0,
                    following_count: 0,
                    post_count: 0,
                    reputation: None,
                });
            items.push(schema::FeedItem {
                post,
                author,
                score: None,
            });
        }
        items
    }

    pub async fn search_profiles(&self, query: &str) -> Vec<Profile> {
        let profiles = self.store.get_profiles().await;
        let q = query.to_lowercase();
        profiles
            .into_iter()
            .filter(|p| {
                p.display_name.to_lowercase().contains(&q) || p.peer_id.to_lowercase().contains(&q)
            })
            .collect()
    }

    pub async fn search_posts(&self, query: &str) -> Vec<Post> {
        let posts = self.store.get_posts().await;
        let q = query.to_lowercase();
        posts
            .into_iter()
            .filter(|p| p.content.to_lowercase().contains(&q))
            .collect()
    }

    pub async fn get_followers(&self, peer_id: &str, _limit: usize) -> Vec<Profile> {
        let peer_ids = self.store.get_followers(peer_id).await;
        let mut profiles = Vec::new();
        for pid in peer_ids {
            if let Some(p) = self.store.get_profile(&pid).await {
                profiles.push(p);
            }
        }
        profiles
    }

    pub async fn get_following(&self, peer_id: &str, _limit: usize) -> Vec<Profile> {
        let mut peer_ids = self.store.get_following(peer_id).await;
        // Fallback to "user" key (all mutations use "user" as follower_id)
        if peer_ids.is_empty() && peer_id != "user" {
            peer_ids = self.store.get_following("user").await;
        }
        let mut profiles = Vec::new();
        for pid in peer_ids {
            if let Some(p) = self.store.get_profile(&pid).await {
                profiles.push(p);
            }
        }
        profiles
    }

    pub async fn register_user(
        &self,
        email: &str,
        password: &str,
        display_name: &str,
    ) -> async_graphql::Result<AuthPayload> {
        let kp = generate_keypair();
        let pk_bytes = kp.verifying_key().to_bytes().to_vec();
        let token = self
            .auth
            .register(email, password, display_name, pk_bytes.clone())
            .await
            .map_err(async_graphql::Error::new)?;

        let peer_id = PeerId::from_public_key_bytes(&pk_bytes);
        let profile = Profile {
            peer_id: peer_id.0.clone(),
            display_name: display_name.to_string(),
            bio: None,
            avatar_cid: None,
            follower_count: 0,
            following_count: 0,
            post_count: 0,
            reputation: None,
        };
        self.store.save_profile(&peer_id.0, profile.clone()).await;
        // Also save under "user" key for mutations that reference it
        self.store.save_profile("user", profile.clone()).await;
        Ok(AuthPayload { token, profile })
    }

    pub async fn login_user(
        &self,
        email: &str,
        password: &str,
    ) -> async_graphql::Result<AuthPayload> {
        let (token, user) = self
            .auth
            .login(email, password)
            .await
            .map_err(async_graphql::Error::new)?;

        let peer_id = PeerId::from_public_key_bytes(&user.public_key);
        let profile = self
            .store
            .get_profile(&peer_id.0)
            .await
            .unwrap_or_else(|| Profile {
                peer_id: peer_id.0,
                display_name: user.display_name.clone(),
                bio: None,
                avatar_cid: None,
                follower_count: 0,
                following_count: 0,
                post_count: 0,
                reputation: None,
            });
        Ok(AuthPayload { token, profile })
    }

    pub async fn create_post(
        &self,
        user_id: &str,
        content: &str,
        media: Vec<String>,
        parent: Option<String>,
    ) -> async_graphql::Result<Post> {
        let cid = format!("post-{}", Uuid::new_v4());
        let post = Post {
            cid: cid.clone(),
            author: user_id.to_string(),
            content: content.to_string(),
            media,
            parent,
            reply_count: 0,
            like_count: 0,
            timestamp: chrono::Utc::now(),
            signature: Vec::new(),
        };
        self.store.save_post(&cid, post.clone()).await;
        let _ = self.event_tx.send(GatewayEvent::NewPost(post.clone()));
        Ok(post)
    }

    pub async fn delete_post(&self, cid: &str) -> async_graphql::Result<bool> {
        Ok(self.store.delete_post(cid).await)
    }

    pub async fn follow_user(
        &self,
        follower_id: &str,
        followee_id: &str,
    ) -> async_graphql::Result<bool> {
        self.store.follow(follower_id, followee_id).await;
        Ok(true)
    }

    pub async fn unfollow_user(
        &self,
        follower_id: &str,
        followee_id: &str,
    ) -> async_graphql::Result<bool> {
        self.store.unfollow(follower_id, followee_id).await;
        Ok(true)
    }

    pub async fn update_profile(
        &self,
        user_id: &str,
        display_name: Option<String>,
        bio: Option<String>,
        avatar_cid: Option<String>,
    ) -> async_graphql::Result<Profile> {
        let mut profile = self
            .store
            .get_profile(user_id)
            .await
            .ok_or_else(|| async_graphql::Error::new("profile not found"))?;
        if let Some(name) = display_name {
            profile.display_name = name;
        }
        if let Some(b) = bio {
            profile.bio = Some(b);
        }
        if let Some(cid) = avatar_cid {
            profile.avatar_cid = Some(cid);
        }
        self.store.save_profile(user_id, profile.clone()).await;
        Ok(profile)
    }
}

pub fn build_schema(state: AppState) -> GatewaySchema {
    Schema::build(QueryRoot, MutationRoot, SubscriptionRoot)
        .data(state)
        .finish()
}

pub async fn run_gateway(config: GatewayConfig, state: AppState) -> anyhow::Result<()> {
    let schema = build_schema(state);

    let app = Router::new()
        .route("/graphql", get(playground).post(graphql_handler))
        .layer(Extension(schema))
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([Method::GET, Method::POST]),
        );

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("GraphQL gateway at http://{addr}/graphql");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn playground() -> impl IntoResponse {
    axum::response::Html(async_graphql::http::playground_source(
        async_graphql::http::GraphQLPlaygroundConfig::new("/graphql"),
    ))
}

async fn graphql_handler(
    Extension(schema): Extension<GatewaySchema>,
    body: String,
) -> impl IntoResponse {
    let req: Request = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::response::Json(serde_json::json!({"error": format!("{e}")})),
            )
        }
    };
    let resp: Response = schema.execute(req).await;
    let status = if resp.is_err() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    (
        status,
        axum::response::Json(serde_json::to_value(&resp).unwrap_or_default()),
    )
}

#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub port: u16,
    pub peer_id: String,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            peer_id: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_profile(peer_id: &str, name: &str) -> Profile {
        Profile {
            peer_id: peer_id.to_string(),
            display_name: name.to_string(),
            bio: None,
            avatar_cid: None,
            follower_count: 0,
            following_count: 0,
            post_count: 0,
            reputation: None,
        }
    }

    fn make_post(author: &str, content: &str) -> Post {
        Post {
            cid: format!("cid-{}", Uuid::new_v4()),
            author: author.to_string(),
            content: content.to_string(),
            media: vec![],
            parent: None,
            reply_count: 0,
            like_count: 0,
            timestamp: chrono::Utc::now(),
            signature: vec![],
        }
    }

    // --- InMemoryStore tests ---

    #[tokio::test]
    async fn test_store_save_get_profile() {
        let store = InMemoryStore::default();
        let p = make_profile("peer1", "Alice");
        store.save_profile("peer1", p.clone()).await;
        let got = store.get_profile("peer1").await.unwrap();
        assert_eq!(got.display_name, "Alice");
        assert_eq!(got.peer_id, "peer1");
    }

    #[tokio::test]
    async fn test_store_get_profile_missing() {
        let store = InMemoryStore::default();
        assert!(store.get_profile("nobody").await.is_none());
    }

    #[tokio::test]
    async fn test_store_get_profiles() {
        let store = InMemoryStore::default();
        store.save_profile("p1", make_profile("p1", "A")).await;
        store.save_profile("p2", make_profile("p2", "B")).await;
        let all = store.get_profiles().await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_store_save_get_post() {
        let store = InMemoryStore::default();
        let post = make_post("alice", "hello");
        let cid = post.cid.clone();
        store.save_post(&cid, post.clone()).await;
        let got = store.get_post(&cid).await.unwrap();
        assert_eq!(got.content, "hello");
        assert_eq!(got.author, "alice");
    }

    #[tokio::test]
    async fn test_store_get_posts_sorted() {
        let store = InMemoryStore::default();
        let mut p1 = make_post("a", "first");
        p1.timestamp = chrono::Utc::now() - chrono::Duration::hours(2);
        let cid1 = p1.cid.clone();
        store.save_post(&cid1, p1.clone()).await;

        let p2 = make_post("b", "second");
        let cid2 = p2.cid.clone();
        store.save_post(&cid2, p2.clone()).await;

        let sorted = store.get_posts().await;
        assert_eq!(sorted[0].content, "second"); // newest first
        assert_eq!(sorted[1].content, "first");
    }

    #[tokio::test]
    async fn test_store_delete_post() {
        let store = InMemoryStore::default();
        let post = make_post("alice", "delete me");
        let cid = post.cid.clone();
        store.save_post(&cid, post).await;
        assert!(store.delete_post(&cid).await);
        assert!(store.get_post(&cid).await.is_none());
    }

    #[tokio::test]
    async fn test_store_delete_missing() {
        let store = InMemoryStore::default();
        assert!(!store.delete_post("nonexistent").await);
    }

    #[tokio::test]
    async fn test_store_follow_unfollow() {
        let store = InMemoryStore::default();
        store.follow("alice", "bob").await;
        let bob_followers = store.get_followers("bob").await;
        assert_eq!(bob_followers, vec!["alice"]);
        let alice_following = store.get_following("alice").await;
        assert_eq!(alice_following, vec!["bob"]);

        store.unfollow("alice", "bob").await;
        assert!(store.get_followers("bob").await.is_empty());
        assert!(store.get_following("alice").await.is_empty());
    }

    #[tokio::test]
    async fn test_store_followers_empty() {
        let store = InMemoryStore::default();
        assert!(store.get_followers("nobody").await.is_empty());
        assert!(store.get_following("nobody").await.is_empty());
    }

    // --- AppState tests ---

    fn make_state() -> AppState {
        let (tx, _) = broadcast::channel(16);
        AppState {
            auth: auth::AuthManager::new("test-secret"),
            event_tx: tx,
            store: InMemoryStore::default(),
        }
    }

    #[tokio::test]
    async fn test_appstate_search_profiles() {
        let state = make_state();
        state
            .store
            .save_profile("peer1", make_profile("peer1", "Alice"))
            .await;
        state
            .store
            .save_profile("peer2", make_profile("peer2", "Bob"))
            .await;
        let results = state.search_profiles("ali").await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].display_name, "Alice");
    }

    #[tokio::test]
    async fn test_appstate_search_profiles_by_peer_id() {
        let state = make_state();
        state
            .store
            .save_profile("abc-123", make_profile("abc-123", "X"))
            .await;
        let results = state.search_profiles("ABC").await;
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_appstate_search_posts() {
        let state = make_state();
        state
            .store
            .save_post("p1", make_post("alice", "hello world"))
            .await;
        state
            .store
            .save_post("p2", make_post("bob", "goodbye"))
            .await;
        let results = state.search_posts("hello").await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].author, "alice");
    }

    #[tokio::test]
    async fn test_appstate_search_posts_empty_query() {
        let state = make_state();
        state.store.save_post("p1", make_post("a", "hello")).await;
        // empty-like query "h" still works
        let results = state.search_posts("z").await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_appstate_get_feed() {
        let state = make_state();
        state
            .store
            .save_profile("alice", make_profile("alice", "Alice"))
            .await;
        state
            .store
            .save_post(
                "p1",
                Post {
                    cid: "p1".into(),
                    author: "alice".into(),
                    content: "post1".into(),
                    media: vec![],
                    parent: None,
                    reply_count: 0,
                    like_count: 0,
                    timestamp: chrono::Utc::now(),
                    signature: vec![],
                },
            )
            .await;
        let feed = state.get_feed(10, 0).await;
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].post.content, "post1");
        assert_eq!(feed[0].author.display_name, "Alice");
    }

    #[tokio::test]
    async fn test_appstate_get_feed_fallback_profile() {
        let state = make_state();
        state
            .store
            .save_post(
                "p1",
                Post {
                    cid: "p1".into(),
                    author: "unknown".into(),
                    content: "orphan".into(),
                    media: vec![],
                    parent: None,
                    reply_count: 0,
                    like_count: 0,
                    timestamp: chrono::Utc::now(),
                    signature: vec![],
                },
            )
            .await;
        let feed = state.get_feed(10, 0).await;
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].author.peer_id, "unknown");
        assert_eq!(feed[0].author.display_name, "unknown");
    }

    #[tokio::test]
    async fn test_appstate_get_feed_pagination() {
        let state = make_state();
        for i in 0..5 {
            let mut post = make_post("alice", &format!("post{}", i));
            // ensure different timestamps
            post.timestamp = chrono::Utc::now() - chrono::Duration::hours(i);
            let cid = post.cid.clone();
            state.store.save_post(&cid, post).await;
        }
        assert_eq!(state.get_feed(3, 0).await.len(), 3);
        assert_eq!(state.get_feed(10, 3).await.len(), 2);
    }

    #[tokio::test]
    async fn test_appstate_create_post() {
        let state = make_state();
        let post = state
            .create_post("alice", "new post", vec![], None)
            .await
            .unwrap();
        assert_eq!(post.author, "alice");
        assert_eq!(post.content, "new post");
        // verify stored
        let stored = state.store.get_post(&post.cid).await.unwrap();
        assert_eq!(stored.content, "new post");
    }

    #[tokio::test]
    async fn test_appstate_delete_post() {
        let state = make_state();
        let post = state
            .create_post("alice", "delete", vec![], None)
            .await
            .unwrap();
        assert!(state.delete_post(&post.cid).await.unwrap());
        assert!(!state.delete_post("nonexistent").await.unwrap());
    }

    #[tokio::test]
    async fn test_appstate_follow_unfollow() {
        let state = make_state();
        state
            .store
            .save_profile("alice", make_profile("alice", "Alice"))
            .await;
        state
            .store
            .save_profile("bob", make_profile("bob", "Bob"))
            .await;

        assert!(state.follow_user("alice", "bob").await.unwrap());
        let following = state.get_following("alice", 10).await;
        assert_eq!(following.len(), 1);
        assert_eq!(following[0].display_name, "Bob");

        assert!(state.unfollow_user("alice", "bob").await.unwrap());
        assert!(state.get_following("alice", 10).await.is_empty());
    }

    #[tokio::test]
    async fn test_appstate_update_profile() {
        let state = make_state();
        let p = make_profile("alice", "Alice");
        state.store.save_profile("alice", p).await;
        let updated = state
            .update_profile(
                "alice",
                Some("Alice Updated".into()),
                Some("bio here".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(updated.display_name, "Alice Updated");
        assert_eq!(updated.bio, Some("bio here".into()));
        assert_eq!(updated.avatar_cid, None);

        let stored = state.store.get_profile("alice").await.unwrap();
        assert_eq!(stored.display_name, "Alice Updated");
    }

    #[tokio::test]
    async fn test_appstate_update_profile_missing() {
        let state = make_state();
        let err = state
            .update_profile("nobody", Some("X".into()), None, None)
            .await
            .unwrap_err();
        assert_eq!(err.message, "profile not found");
    }

    #[tokio::test]
    async fn test_gateway_config_default() {
        let cfg = GatewayConfig::default();
        assert_eq!(cfg.port, 8080);
        assert!(cfg.peer_id.is_empty());
    }

    // --- HTTP integration tests ---

    use axum::{
        body::Body,
        http::{Method, Request, StatusCode},
    };
    use tower::ServiceExt;

    fn build_test_app() -> Router {
        let (tx, _) = broadcast::channel(16);
        let state = AppState {
            auth: auth::AuthManager::new("test-secret"),
            event_tx: tx,
            store: InMemoryStore::default(),
        };
        let schema = build_schema(state);
        Router::new()
            .route("/graphql", get(playground).post(graphql_handler))
            .layer(Extension(schema))
    }

    #[tokio::test]
    async fn test_http_graphql_playground() {
        let app = build_test_app();
        let req = Request::builder()
            .method(Method::GET)
            .uri("/graphql")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_http_graphql_register_mutation() {
        let app = build_test_app();
        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"mutation { register(email: \"alice@test.com\", password: \"pass123\", displayName: \"Alice\") { token profile { displayName } } }"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["data"]["register"]["profile"]["displayName"].as_str() == Some("Alice"));
        assert!(json["data"]["register"]["token"].as_str().unwrap().len() > 10);
    }

    #[tokio::test]
    async fn test_http_graphql_login() {
        let app = build_test_app();
        // First register
        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"mutation { register(email: \"bob@test.com\", password: \"secret\", displayName: \"Bob\") { token } }"}"#,
            ))
            .unwrap();
        app.clone().oneshot(req).await.unwrap();

        // Then login
        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"mutation { login(email: \"bob@test.com\", password: \"secret\") { profile { displayName } } }"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"]["login"]["profile"]["displayName"], "Bob");
    }

    #[tokio::test]
    async fn test_http_graphql_create_post_and_feed() {
        let app = build_test_app();
        // Register a user
        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"mutation { register(email: \"carol@test.com\", password: \"pwd\", displayName: \"Carol\") { token } }"}"#,
            ))
            .unwrap();
        app.clone().oneshot(req).await.unwrap();

        // Create a post
        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"mutation { createPost(content: \"Hello from HTTP test!\") { cid content author } }"}"#,
            ))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["data"]["createPost"]["content"],
            "Hello from HTTP test!"
        );

        // Query the feed
        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"{ feed { post { content } author { displayName } } }"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(!json["data"]["feed"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_http_graphql_search_profiles() {
        let app = build_test_app();
        // Register
        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"mutation { register(email: \"dave@test.com\", password: \"x\", displayName: \"Dave\") { token } }"}"#,
            ))
            .unwrap();
        app.clone().oneshot(req).await.unwrap();

        // Search
        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"{ searchProfiles(query: \"dave\") { displayName } }"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"]["searchProfiles"][0]["displayName"], "Dave");
    }

    #[tokio::test]
    async fn test_http_graphql_invalid_query() {
        let app = build_test_app();
        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"query":"invalid syntax"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Invalid GraphQL syntax still returns 200 with error in body
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(!json["errors"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_http_graphql_follow_and_search() {
        let app = build_test_app();
        // Register two users
        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"mutation { register(email: \"eve@test.com\", password: \"x\", displayName: \"Eve\") { token } }"}"#,
            ))
            .unwrap();
        app.clone().oneshot(req).await.unwrap();

        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"mutation { register(email: \"frank@test.com\", password: \"x\", displayName: \"Frank\") { token profile { peerId } } }"}"#,
            ))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let frank_peer_id = json["data"]["register"]["profile"]["peerId"]
            .as_str()
            .unwrap()
            .to_string();

        // Follow Frank
        let query = format!(r#"{{"query":"mutation {{ follow(peerId: \"{frank_peer_id}\") }}"}}"#,);
        let req = Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(query))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
