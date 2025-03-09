mod competitionAttributes;
mod liveActivityApns;

use robotevents::client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::join;
use tokio::sync::RwLock;
use tokio::time::{sleep, sleep_until};
use warp::{http, Filter};

// add a constant for the bundle id
const BUNDLE_ID: &str = "net.dickhans.EchoPulse";

#[derive(Serialize, Deserialize, Debug, Clone)]
struct DeviceSubscription {
    competition_id: u32,
    division_id: u32,
    device_token: String,
    watch_team: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct DeviceSubscriptionChangeRequest {
    new_device_token: String,
    old_device_token: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Hash, Eq)]
struct CompetitionDivisionPair {
    competition_id: u32,
    division_id: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct TeamTokenPair {
    team_name: String,
    device_token: String,
}

impl CompetitionDivisionPair {
    fn new(competition_id: u32, division_id: u32) -> Self {
        Self {
            competition_id,
            division_id,
        }
    }

    fn from_device(device: &DeviceSubscription) -> Self {
        Self {
            competition_id: device.competition_id,
            division_id: device.division_id,
        }
    }
}

#[derive(Debug, Clone)]
struct StateStore {
    subscriptions: Arc<RwLock<HashMap<CompetitionDivisionPair, Vec<TeamTokenPair>>>>,
    matches: Arc<RwLock<HashMap<CompetitionDivisionPair, Vec<robotevents::schema::Match>>>>,
    apns_client: Arc<RwLock<liveActivityApns::LiveActivityClient>>,
}

impl StateStore {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let team_id = std::env::var("APPLE_TEAM_ID").expect("APPLE_TEAM_ID not set");
        let key_id = std::env::var("APPLE_KEY_ID").expect("APPLE_KEY_ID not set");
        let key_path = std::env::var("APPLE_KEY_PATH").expect("APPLE_KEY_PATH not set");

        let mut apns_client =
            liveActivityApns::LiveActivityClient::new(&team_id, &key_id, &key_path, BUNDLE_ID)?;

        Ok(Self {
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            matches: Arc::new(RwLock::new(HashMap::new())),
            apns_client: Arc::new(RwLock::new(apns_client)),
        })
    }

    async fn test_push_notifs(&self) {
        // use test method in liveActivityApns
        let mut apns_client = self.apns_client.write().await;

        // go through each device and update it with the liveActivityApns::test_live_activity() function
        // for each device in the subscriptions hashmap
        let subscriptions = self.subscriptions.read().await;

        for (competition_division, devices) in subscriptions.iter() {
            for TeamTokenPair {
                team_name,
                device_token,
            } in devices.iter()
            {
                liveActivityApns::test_live_activity(&mut apns_client, device_token)
                    .await
                    .expect("unable to send messages");
            }
        }
    }

    async fn add_subscription_from_device(&self, device: DeviceSubscription) {
        println!(
            "Adding subscription for competition {:?} and device {}",
            CompetitionDivisionPair::from_device(&device),
            device.device_token
        );
        let mut subscriptions = self.subscriptions.write().await;
        let entry = subscriptions
            .entry(CompetitionDivisionPair::from_device(&device))
            .or_insert(Vec::new());
        entry.push(TeamTokenPair {
            team_name: device.watch_team.clone(),
            device_token: device.device_token,
        });
    }

    async fn change_subscription_from_device(&self, device: &DeviceSubscriptionChangeRequest) {
        let mut subscriptions = self.subscriptions.write().await;

        let mut old_competition_division = None;
        let mut old_watch_team = None;

        for (competition_division, devices) in subscriptions.iter_mut() {
            for TeamTokenPair {
                team_name,
                device_token,
            } in devices.iter_mut()
            {
                if device_token == &device.old_device_token {
                    old_competition_division = Some(competition_division.clone());
                    old_watch_team = Some(team_name.clone());
                    devices.retain(
                        |TeamTokenPair {
                             team_name,
                             device_token,
                         }| device_token != &device.old_device_token,
                    );
                    break;
                }
            }
        }

        if let Some(old_competition_division) = old_competition_division {
            let new_subscriptions = subscriptions
                .entry(old_competition_division)
                .or_insert(Vec::new());
            new_subscriptions.push(TeamTokenPair {
                device_token: device.new_device_token.clone(),
                team_name: old_watch_team.unwrap(),
            });
        }
    }
}

async fn add_device(
    device: DeviceSubscription,
    state_store: StateStore,
) -> Result<impl warp::Reply, warp::Rejection> {
    // let r = competition.grocery_list.read();
    // Ok(warp::reply::json(&*r))
    state_store.add_subscription_from_device(device).await;
    Ok(warp::reply::with_status(
        "Added device",
        http::StatusCode::CREATED,
    ))
}

async fn change_device(
    device: DeviceSubscriptionChangeRequest,
    state_store: StateStore,
) -> Result<impl warp::Reply, warp::Rejection> {
    state_store.change_subscription_from_device(&device).await;
    Ok(warp::reply::with_status(
        "Changed device",
        http::StatusCode::ACCEPTED,
    ))
}

fn json_body_new_device(
) -> impl Filter<Extract = (DeviceSubscription,), Error = warp::Rejection> + Clone {
    // When accepting a body, we want a JSON body
    // (and to reject huge payloads)...
    warp::body::content_length_limit(1024 * 16).and(warp::body::json())
}

fn json_body_change_device(
) -> impl Filter<Extract = (DeviceSubscriptionChangeRequest,), Error = warp::Rejection> + Clone {
    // When accepting a body, we want a JSON body
    // (and to reject huge payloads)...
    warp::body::content_length_limit(1024 * 16).and(warp::body::json())
}

async fn poll(state_store: StateStore) {
    loop {
        // just print information about each subscription
        let subscriptions = state_store.subscriptions.read().await;

        println!("Subscriptions:");

        for (competition_id, devices) in subscriptions.iter() {
            println!("Competition {:?}: {:?}", competition_id, devices);
        }

        // test push notifications
        state_store.test_push_notifs().await;

        sleep_until(tokio::time::Instant::now() + tokio::time::Duration::from_secs(10)).await;
    }
}

#[tokio::main]
async fn main() {
    let token = std::env::var("ROBOTEVENTS_TOKEN").expect("ROBOTEVENTS_TOKEN not set");

    // let client = client::RobotEvents::new(token);

    let store = StateStore::new().unwrap();
    let cloned_store = store.clone();
    let store_filter = warp::any().map(move || cloned_store.clone());

    let add_items = warp::post()
        .and(warp::path("v1"))
        .and(warp::path("subscribe"))
        .and(warp::path::end())
        .and(json_body_new_device())
        .and(store_filter.clone())
        .and_then(add_device);

    let change_device = warp::post()
        .and(warp::path("v1"))
        .and(warp::path("change"))
        .and(warp::path::end())
        .and(json_body_change_device())
        .and(store_filter.clone())
        .and_then(change_device);

    join!(
        warp::serve(add_items.or(change_device)).run(([0, 0, 0, 0], 3030)),
        poll(store.clone()),
    );
}
