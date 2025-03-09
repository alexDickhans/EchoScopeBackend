mod competitionAttributes;
mod liveActivityApns;

use robotevents::{client, RobotEvents};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use robotevents::query::{DivisionMatchesQuery, PaginatedQuery};
use serde_json::json;
use tokio::join;
use tokio::sync::RwLock;
use tokio::time::{sleep, sleep_until};
use warp::{http, Filter};
use crate::competitionAttributes::CompetitionAttributesContentState;

// add a constant for the bundle id
const BUNDLE_ID: &str = "net.dickhans.EchoPulse";

#[derive(Serialize, Deserialize, Debug, Clone)]
struct DeviceSubscription {
    competition_id: i32,
    division_id: i32,
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
    competition_id: i32,
    division_id: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct TeamTokenPair {
    team_name: String,
    device_token: String,
}

impl CompetitionDivisionPair {
    fn new(competition_id: i32, division_id: i32) -> Self {
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
    robot_events_client: Arc<RwLock<RobotEvents>>,
}

impl StateStore {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let team_id = std::env::var("APPLE_TEAM_ID").expect("APPLE_TEAM_ID not set");
        let key_id = std::env::var("APPLE_KEY_ID").expect("APPLE_KEY_ID not set");
        let key_path = std::env::var("APPLE_KEY_PATH").expect("APPLE_KEY_PATH not set");

        println!("Creating APNS client with team_id {}, key_id {}, key_path {}", team_id, key_id, key_path);

        let mut apns_client =
            liveActivityApns::LiveActivityClient::new(&team_id, &key_id, &key_path, BUNDLE_ID).expect("Unable to create APNS client");

        Ok(Self {
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            matches: Arc::new(RwLock::new(HashMap::new())),
            apns_client: Arc::new(RwLock::new(apns_client)),
            robot_events_client: Arc::new(RwLock::new(client::RobotEvents::new(
                std::env::var("ROBOTEVENTS_TOKEN").expect("ROBOTEVENTS_TOKEN not set"),
            ))),
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

        if device.new_device_token.is_empty() {
            println!("Removing device with token {}", device.old_device_token);
            Self::remove_empty_subscriptions(&mut *self.subscriptions.write().await);
            return;
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

    fn remove_empty_subscriptions(subscriptions: &mut HashMap<CompetitionDivisionPair, Vec<TeamTokenPair>>) {
        subscriptions.retain(|_, v| !v.is_empty());
    }

    async fn update_all_subscriptions(&self) {
        // mutably get the current match hash map
        let mut matches = self.matches.write().await;

        // get the current subscriptions hash map
        let subscriptions = self.subscriptions.read().await;

        // mutably get the robot events client
        let robot_events_client = self.robot_events_client.write().await;

        // mutably get the apns client
        let mut apns_client = self.apns_client.write().await;

        println!("updating all subscriptions");

        // for each competition division pair in the subscriptions hash map
        for (competition_division, devices) in subscriptions.iter() {
            // get the matches for the competition division pair
            if let Some(new_matches) = get_matches(competition_division, &robot_events_client).await {
                // if the matches don't match what is in the matches hash map, update the matches hash map and send a notification
                if new_matches != *matches.get(competition_division).unwrap_or(&Vec::new()) {
                    matches.insert(competition_division.clone(), new_matches.clone());

                    // for each device in the devices vector
                    for TeamTokenPair {
                        team_name,
                        device_token,
                    } in devices.iter()
                    {
                        let content_state = CompetitionAttributesContentState::from_matchlist(&new_matches, team_name);

                        let payload = json!({
                        "aps": {
                            "timestamp": chrono::Utc::now().timestamp(),
                            "event": "update",
                            "content-state": content_state
                        }
                    });

                        println!("Sending notification to device {}, with payload {}", device_token, payload);

                        // send a notification to the device
                        apns_client.send_live_activity_notification(device_token, &payload).await.expect("Unable to send notification");
                    }
                } else {
                    println!("No new matches found for competition division pair {:?}", competition_division);
                }
            } else {
                println!("ERROR: No matches found for competition division pair {:?}", competition_division);
            }
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

async fn remove_device(
    device: DeviceSubscription,
    state_store: StateStore,
) -> Result<impl warp::Reply, warp::Rejection> {
    Ok(warp::reply::with_status(
        "Removed device",
        http::StatusCode::OK,
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

/// get all the matches from a competition division pair
async fn get_matches(
    competition_division: &CompetitionDivisionPair,
    robot_events_client: &RobotEvents
) -> Option<Vec<robotevents::schema::Match>> {
    let matches = robot_events_client.event_division_matches(competition_division.competition_id, competition_division.division_id, DivisionMatchesQuery::new().per_page(250)).await;

    Some(matches.ok()?.data)
}

async fn poll(state_store: StateStore) {
    loop {
        // just print information about each subscription
        let start_time = tokio::time::Instant::now();

        state_store.update_all_subscriptions().await;

        sleep_until(start_time + tokio::time::Duration::from_secs(30)).await;
    }
}

#[tokio::main]
async fn main() {
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
        warp::serve(add_items.or(change_device)).run(([0, 0, 0, 0], std::env::var("PORT").expect("PORT not set").parse().unwrap())),
        poll(store.clone()),
    );
}
