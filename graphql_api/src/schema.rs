use async_graphql::{Context, Object, SimpleObject, Subscription};
use chrono::{DateTime, Utc};
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Serialize, Deserialize, SimpleObject, Clone)]
pub struct AuthPayload {
    pub token: String,
    pub profile: Profile,
}

#[derive(Debug, Serialize, Deserialize, SimpleObject, Clone)]
pub struct Profile {
    pub peer_id: String,
    pub display_name: String,
    pub bio: Option<String>,
    pub avatar_cid: Option<String>,
    pub follower_count: u32,
    pub following_count: u32,
    pub post_count: u32,
    pub reputation: Option<f64>,
}

#[derive(Serialize, Deserialize, SimpleObject, Clone)]
pub struct Post {
    pub cid: String,
    pub author: String,
    pub content: String,
    pub media: Vec<String>,
    pub parent: Option<String>,
    pub reply_count: u32,
    pub like_count: u32,
    pub timestamp: DateTime<Utc>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize, SimpleObject, Clone)]
pub struct GatewayInfo {
    pub peer_id: String,
    pub api_url: String,
    pub region: String,
    pub roles: Vec<String>,
    pub reputation_score: f64,
    pub latency_ms: Option<u64>,
}

#[derive(Serialize, Deserialize, SimpleObject, Clone)]
pub struct FeedItem {
    pub post: Post,
    pub author: Profile,
    pub score: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, SimpleObject, Clone)]
pub struct GpuInfo {
    pub vram_gb: f64,
    pub model: String,
    pub slots_total: u32,
    pub slots_available: u32,
}

#[derive(Debug, Serialize, Deserialize, SimpleObject, Clone)]
pub struct NodeResources {
    pub cpu_cores: u32,
    pub cpu_freq_mhz: f64,
    pub ram_total_gb: f64,
    pub ram_available_gb: f64,
    pub disk_total_gb: f64,
    pub disk_available_gb: f64,
    pub gpu: Option<GpuInfo>,
    pub uptime_secs: u64,
    pub platform: String,
}

pub struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn available_gateways(
        &self,
        _ctx: &Context<'_>,
        _region: Option<String>,
    ) -> Vec<GatewayInfo> {
        Vec::new()
    }

    async fn profile(&self, ctx: &Context<'_>, peer_id: String) -> Option<Profile> {
        let state = ctx.data_unchecked::<AppState>();
        state.get_profile(&peer_id).await
    }

    async fn post(&self, ctx: &Context<'_>, cid: String) -> Option<Post> {
        let state = ctx.data_unchecked::<AppState>();
        state.get_post(&cid).await
    }

    async fn feed(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> Vec<FeedItem> {
        let state = ctx.data_unchecked::<AppState>();
        state
            .get_feed(limit.unwrap_or(20) as usize, offset.unwrap_or(0) as usize)
            .await
    }

    async fn search_profiles(&self, ctx: &Context<'_>, query: String) -> Vec<Profile> {
        let state = ctx.data_unchecked::<AppState>();
        state.search_profiles(&query).await
    }

    async fn search_posts(&self, ctx: &Context<'_>, query: String) -> Vec<Post> {
        let state = ctx.data_unchecked::<AppState>();
        state.search_posts(&query).await
    }

    async fn followers(
        &self,
        ctx: &Context<'_>,
        peer_id: String,
        limit: Option<i32>,
    ) -> Vec<Profile> {
        let state = ctx.data_unchecked::<AppState>();
        state
            .get_followers(&peer_id, limit.unwrap_or(50) as usize)
            .await
    }

    async fn following(
        &self,
        ctx: &Context<'_>,
        peer_id: String,
        limit: Option<i32>,
    ) -> Vec<Profile> {
        let state = ctx.data_unchecked::<AppState>();
        state
            .get_following(&peer_id, limit.unwrap_or(50) as usize)
            .await
    }

    async fn node_resources(&self, _ctx: &Context<'_>) -> NodeResources {
        let res = resource_manager::NodeResources::detect();
        let gpu = match (res.gpu_vram_gb, res.gpu_model) {
            (Some(vram), Some(model)) => {
                let slots = resource_manager::gpu::GpuSlot::create_slots(vram, &model);
                Some(GpuInfo {
                    vram_gb: vram,
                    model,
                    slots_total: slots.len() as u32,
                    slots_available: slots.iter().filter(|s| s.available).count() as u32,
                })
            }
            _ => None,
        };
        NodeResources {
            cpu_cores: res.cpu_cores,
            cpu_freq_mhz: res.cpu_freq_mhz,
            ram_total_gb: res.ram_total_gb,
            ram_available_gb: res.ram_available_gb,
            disk_total_gb: res.disk_total_gb,
            disk_available_gb: res.disk_available_gb,
            gpu,
            uptime_secs: res.uptime_secs,
            platform: format!("{:?}", res.platform),
        }
    }
}

pub struct MutationRoot;

#[Object]
impl MutationRoot {
    async fn register(
        &self,
        ctx: &Context<'_>,
        email: String,
        password: String,
        display_name: String,
    ) -> async_graphql::Result<AuthPayload> {
        let state = ctx.data_unchecked::<AppState>();
        state.register_user(&email, &password, &display_name).await
    }

    async fn login(
        &self,
        ctx: &Context<'_>,
        email: String,
        password: String,
    ) -> async_graphql::Result<AuthPayload> {
        let state = ctx.data_unchecked::<AppState>();
        state.login_user(&email, &password).await
    }

    async fn create_post(
        &self,
        ctx: &Context<'_>,
        content: String,
        media: Option<Vec<String>>,
        parent: Option<String>,
    ) -> async_graphql::Result<Post> {
        let state = ctx.data_unchecked::<AppState>();
        state
            .create_post("user", &content, media.unwrap_or_default(), parent)
            .await
    }

    async fn delete_post(&self, ctx: &Context<'_>, cid: String) -> async_graphql::Result<bool> {
        let state = ctx.data_unchecked::<AppState>();
        state.delete_post(&cid).await
    }

    async fn follow(&self, ctx: &Context<'_>, peer_id: String) -> async_graphql::Result<bool> {
        let state = ctx.data_unchecked::<AppState>();
        state.follow_user("user", &peer_id).await
    }

    async fn unfollow(&self, ctx: &Context<'_>, peer_id: String) -> async_graphql::Result<bool> {
        let state = ctx.data_unchecked::<AppState>();
        state.unfollow_user("user", &peer_id).await
    }

    async fn update_profile(
        &self,
        ctx: &Context<'_>,
        display_name: Option<String>,
        bio: Option<String>,
        avatar_cid: Option<String>,
    ) -> async_graphql::Result<Profile> {
        let state = ctx.data_unchecked::<AppState>();
        state
            .update_profile("user", display_name, bio, avatar_cid)
            .await
    }
}

pub struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {
    async fn new_posts(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = false)] _following: bool,
    ) -> impl Stream<Item = Post> {
        let state = ctx.data_unchecked::<AppState>();
        let mut rx = state.event_tx.subscribe();
        async_stream::stream! {
            while let Ok(event) = rx.recv().await {
                let GatewayEvent::NewPost(post) = event;
                yield post;
            }
        }
    }
}

#[derive(Clone)]
pub enum GatewayEvent {
    NewPost(Post),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_payload_serde() {
        let p = Profile {
            peer_id: "peer1".into(),
            display_name: "Alice".into(),
            bio: Some("hello".into()),
            avatar_cid: None,
            follower_count: 5,
            following_count: 3,
            post_count: 10,
            reputation: Some(100.0),
        };
        let payload = AuthPayload {
            token: "jwt-token".into(),
            profile: p,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let decoded: AuthPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.token, "jwt-token");
        assert_eq!(decoded.profile.display_name, "Alice");
        assert_eq!(decoded.profile.follower_count, 5);
    }

    #[test]
    fn test_feed_item_serde() {
        let post = Post {
            cid: "cid-1".into(),
            author: "author-1".into(),
            content: "content".into(),
            media: vec!["img.jpg".into()],
            parent: None,
            reply_count: 2,
            like_count: 3,
            timestamp: chrono::Utc::now(),
            signature: vec![1, 2, 3],
        };
        let author = Profile {
            peer_id: "author-1".into(),
            display_name: "Author".into(),
            bio: None,
            avatar_cid: None,
            follower_count: 0,
            following_count: 0,
            post_count: 1,
            reputation: None,
        };
        let item = FeedItem {
            post,
            author,
            score: Some(42.5),
        };
        let json = serde_json::to_string(&item).unwrap();
        let decoded: FeedItem = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.post.cid, "cid-1");
        assert_eq!(decoded.author.display_name, "Author");
        assert_eq!(decoded.score, Some(42.5));
    }

    #[test]
    fn test_gateway_info_serde() {
        let info = GatewayInfo {
            peer_id: "pid".into(),
            api_url: "http://gw:8080".into(),
            region: "us-east".into(),
            roles: vec!["compute".into()],
            reputation_score: 80.0,
            latency_ms: Some(15),
        };
        let json = serde_json::to_string(&info).unwrap();
        let decoded: GatewayInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.latency_ms, Some(15));
        assert_eq!(decoded.roles.len(), 1);
    }

    #[test]
    fn test_gateway_event_new_post() {
        let post = Post {
            cid: "cid-evt".into(),
            author: "a".into(),
            content: "event content".into(),
            media: vec![],
            parent: None,
            reply_count: 0,
            like_count: 0,
            timestamp: chrono::Utc::now(),
            signature: vec![],
        };
        let event = GatewayEvent::NewPost(post);
        match event {
            GatewayEvent::NewPost(p) => assert_eq!(p.cid, "cid-evt"),
        }
    }
}
