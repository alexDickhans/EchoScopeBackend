use hyper::{Body, Client, Method, Request};
use hyper_tls::HttpsConnector;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        let https = HttpsConnector::new();
        let client = Client::builder().build::<_, Body>(https);

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
        action: LiveActivityAction,
    ) -> Result<(), Box<dyn Error>> {
        let token = self.get_token()?;

        // Determine push type based on the activity action
        let push_type = match action {
            LiveActivityAction::Start => "activity",
            LiveActivityAction::Update => "activity.update",
            LiveActivityAction::End => "activity.end",
        };

        // Create the URI
        let uri = format!("https://api.push.apple.com/3/device/{}", device_token);

        // Build the request
        let req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("authorization", format!("bearer {}", token))
            .header(
                "apns-topic",
                format!("{}.push-type.{}", self.bundle_id, push_type),
            )
            .header("apns-push-type", push_type)
            .header("apns-priority", "10")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(payload)?))?;

        let res = self.client.request(req).await?;

        if !res.status().is_success() {
            let body_bytes = hyper::body::to_bytes(res.into_body()).await?;
            let body_str = String::from_utf8_lossy(&body_bytes);
            return Err(format!("APNs error: {}", body_str).into());
        }

        Ok(())
    }

    // Helper methods for Live Activity operations
    pub async fn start_match_activity(
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
                "event": "start"
            }
        });

        self.send_live_activity_notification(device_token, &payload, LiveActivityAction::Start)
            .await
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

        self.send_live_activity_notification(device_token, &payload, LiveActivityAction::Update)
            .await
    }

    pub async fn end_match_activity(
        &mut self,
        device_token: &str,
        match_info: &Value,
        team_id: u32,
        dismissal_delay: Option<u64>,
    ) -> Result<(), Box<dyn Error>> {
        let content_state = create_match_content_state(match_info, team_id);

        let mut aps = json!({
            "content-state": content_state,
            "timestamp": SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_secs(),
            "event": "end"
        });

        // Add dismissal date if provided (seconds to keep notification after ending)
        if let Some(delay) = dismissal_delay {
            let dismissal = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + delay;

            aps.as_object_mut()
                .unwrap()
                .insert("dismissal-date".to_string(), json!(dismissal));
        }

        let payload = json!({ "aps": aps });

        self.send_live_activity_notification(device_token, &payload, LiveActivityAction::End)
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
