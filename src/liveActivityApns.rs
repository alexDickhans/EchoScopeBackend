use hyper::{Body, Client, Method, Request};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use crate::competitionAttributes::{Alliance, CompetitionAttributesContentState, DisplayMatch};

pub enum LiveActivityAction {
    Start,
    Update,
    End,
}

#[derive(Debug)]
pub struct LiveActivityClient {
    client: Client<HttpsConnector<hyper::client::HttpConnector>>,
    team_id: String,
    key_id: String,
    private_key: Vec<u8>,
    token_expiration: Duration,
    current_token: Option<(String, SystemTime)>,
    bundle_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iss: String,
    iat: u64,
}

impl LiveActivityClient {
    pub fn new(
        team_id: &str,
        key_id: &str,
        key_path: &str,
        bundle_id: &str,
    ) -> Result<Self, Box<dyn Error>> {
        let private_key = fs::read(key_path)?;

        let https = HttpsConnectorBuilder::new()
            .with_native_roots()
            .https_only()
            .enable_http2()
            .build();

        let client = Client::builder()
            .http2_only(true)
            .build::<_, Body>(https);

        Ok(LiveActivityClient {
            client,
            team_id: team_id.to_string(),
            key_id: key_id.to_string(),
            private_key,
            token_expiration: Duration::from_secs(55 * 60), // 55 minutes
            current_token: None,
            bundle_id: bundle_id.to_string(),
        })
    }

    fn generate_token(&self) -> Result<String, Box<dyn Error>> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        // Use a simplified claims structure that matches Apple's requirements
        let claims = Claims {
            iss: self.team_id.clone(),
            iat: now,
        };

        // Configure the header with required fields for APNs
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.key_id.clone());
        // Remove typ field by using a custom header
        header.typ = None;

        // Apple uses PKCS#8 keys, so ensure proper reading
        let key_content = String::from_utf8_lossy(&self.private_key);

        // Use direct PEM key loading which handles the parsing correctly
        let encoding_key = EncodingKey::from_ec_pem(key_content.as_bytes())?;

        let token = encode(&header, &claims, &encoding_key)?;
        Ok(token)
    }

    pub fn get_token(&mut self) -> Result<String, Box<dyn Error>> {
        match &self.current_token {
            Some((token, created_at)) => {
                let now = SystemTime::now();
                if now.duration_since(*created_at)? < self.token_expiration {
                    return Ok(token.clone());
                }
            }
            None => {}
        }

        let token = self.generate_token()?;
        self.current_token = Some((token.clone(), SystemTime::now()));
        Ok(token)
    }

    pub async fn send_live_activity_notification(
        &mut self,
        device_token: &str,
        payload: &Value,
    ) -> Result<(), Box<dyn Error>> {
        let token = self.get_token()?;

        // Create the URI
        let uri = format!("https://api.sandbox.push.apple.com/3/device/{}", device_token);

        // Build the request
        let req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("authorization", format!("bearer {}", token))
            .header(
                "apns-topic",
                format!("{}.push-type.liveactivity", self.bundle_id),
            )
            .header("apns-push-type", "liveactivity")
            .header("apns-priority", "10")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(payload)?))?;

        let res = self.client.request(req).await?;

        println!("Response: {:?}", res.status());

        if !res.status().is_success() {
            let body_bytes = hyper::body::to_bytes(res.into_body()).await?;
            let body_str = String::from_utf8_lossy(&body_bytes);
            return Err(format!("APNs error: {}", body_str).into());
        }

        Ok(())
    }

    pub async fn update_match_activity(
        &mut self,
        device_token: &str,
        match_info: &Value,
        team_id: u32,
    ) -> Result<(), Box<dyn Error>> {
        let content_state = create_match_content_state(match_info, team_id);

        let payload = json!({
            "aps": {
                "content-state": content_state,
                "timestamp": SystemTime::now()
                    .duration_since(UNIX_EPOCH)?
                    .as_secs(),
                "event": "update"
            }
        });

        self.send_live_activity_notification(device_token, &payload)
            .await
    }
}

// Helper function to create content state for match updates
fn create_match_content_state(match_data: &Value, team_id: u32) -> Value {
    // Extract relevant information from match_data
    // This should be customized based on your match data structure
    json!({
        "matchName": match_data.get("name").unwrap_or(&json!("Unknown")),
        "teamId": team_id,
        "redScore": match_data.get("red_score").unwrap_or(&json!(0)),
        "blueScore": match_data.get("blue_score").unwrap_or(&json!(0)),
        "matchStatus": match_data.get("status").unwrap_or(&json!("unknown")),
        "scheduledTime": match_data.get("scheduled").unwrap_or(&json!(0))
    })
}

pub async fn test_live_activity(client: &mut LiveActivityClient, device_token: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Create test match data
    let test_red_alliance = Alliance {
        team1: "R1".to_string(),
        team2: Some("R2".to_string()),
        score: Some(0)
    };

    let test_blue_alliance = Alliance {
        team1: "B1".to_string(),
        team2: Some("B2".to_string()),
        score: Some(0)
    };

    let next_match = DisplayMatch {
        name: "Q5".to_string(),
        scheduled: Some(SystemTime::from(chrono::Utc::now() + chrono::Duration::minutes(15))),
        start_time: None,
        red_alliance: test_red_alliance.clone(),
        blue_alliance: test_blue_alliance.clone()
    };

    // Create initial content state
    let content_state = CompetitionAttributesContentState {
        last_match: None,
        next_match: Some(next_match.clone()),
        team_next_match: Some(next_match.clone())
    };

    // Start the activity
    let start_payload = json!({
        "aps": {
            "timestamp": chrono::Utc::now().timestamp(),
            "event": "update",
            "content-state": content_state
        }
    });

    client.send_live_activity_notification(
        device_token,
        &start_payload,
    ).await?;

    // Wait before updating
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

    Ok(())
}
