use std::collections::HashMap;
use std::sync::Arc;
use a2::{DefaultNotificationBuilder, NotificationBuilder, NotificationOptions};
use a2::request::payload::APSAlert::Default;
use robotevents::client;
use robotevents::query::{DivisionMatchesQuery, EventsQuery, PaginatedQuery, TeamSkillsQuery, TeamsQuery};
use robotevents::schema::Division;
use tokio::sync::RwLock;
use warp::{http, Filter};
use serde::{Deserialize, Serialize};
use tokio::join;
use tokio::time::sleep_until;

// add a constant for the bundle id
const BUNDLE_ID: &str = "net.dickhans.EchoPulse";

#[derive(Serialize, Deserialize, Debug, Clone)]
struct DeviceSubscription {
    competition_id: u32,
    division_id: u32,
    device_token: String,
    watch_team: u32,
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
    subscriptions: Arc<RwLock<HashMap<CompetitionDivisionPair, Vec<(String, u32)>>>>,
    matches: Arc<RwLock<HashMap<CompetitionDivisionPair, Vec<robotevents::schema::Match>>>>,
}

impl StateStore {
    fn new() -> Self {
        Self {
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            matches: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn add_subscription_from_device(&self, device: DeviceSubscription) {
        println!("Adding subscription for competition {:?} and device {}", CompetitionDivisionPair::from_device(&device), device.device_token);
        let mut subscriptions = self.subscriptions.write().await;
        let entry = subscriptions.entry(CompetitionDivisionPair::from_device(&device)).or_insert(Vec::new());
        entry.push((device.device_token, device.watch_team));
    }

    async fn change_subscription_from_device(&self, device: &DeviceSubscriptionChangeRequest) {
        // find the old subscription and remove it, storing where it was and what team was associated with it
        let mut subscriptions = self.subscriptions.write().await;

        let mut old_competition_division = None;
        let mut old_watch_team = None;

        for (competition_division, devices) in subscriptions.iter_mut() {
            for (device_token, watch_team) in devices.iter_mut() {
                if device_token == &device.old_device_token {
                    old_competition_division = Some(competition_division.clone());
                    old_watch_team = Some(*watch_team);
                    devices.retain(|(token, _)| token != &device.old_device_token);
                    break;
                }
            }
        }

        if let Some(old_competition_division) = old_competition_division {
            let mut new_subscriptions = subscriptions.entry(old_competition_division).or_insert(Vec::new());
            new_subscriptions.push((device.new_device_token.clone(), old_watch_team.unwrap()));
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

fn json_body_new_device() -> impl Filter<Extract = (DeviceSubscription,), Error = warp::Rejection> + Clone {
    // When accepting a body, we want a JSON body
    // (and to reject huge payloads)...
    warp::body::content_length_limit(1024 * 16).and(warp::body::json())
}

fn json_body_change_device() -> impl Filter<Extract = (DeviceSubscriptionChangeRequest,), Error = warp::Rejection> + Clone {
    // When accepting a body, we want a JSON body
    // (and to reject huge payloads)...
    warp::body::content_length_limit(1024 * 16).and(warp::body::json())
}

async fn poll(state_store: StateStore) {
    loop {
        sleep_until(tokio::time::Instant::now() + tokio::time::Duration::from_secs(10)).await;

        // just print information about each subscription
        let subscriptions = state_store.subscriptions.read().await;
        for (competition_id, devices) in subscriptions.iter() {
            println!("Competition {:?}: {:?}", competition_id, devices);
        }
    }
}

#[tokio::main]
async fn main() {
    let token = std::env::var("ROBOTEVENTS_TOKEN").expect("ROBOTEVENTS_TOKEN not set");

    let client = client::RobotEvents::new(token);

    let store = StateStore::new();
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

    let mut builder = DefaultNotificationBuilder::new();
    let payload = "test";

    let mut notificationBuilder = NotificationOptions{
        apns_priority: Some(Priority::Normal),
        ..Default::default()
    };


    builder.build("device-token-from-the-user", Default::default());

    join!(
        warp::serve(add_items.or(change_device))
            .run(([0, 0, 0, 0], 3030)),
        poll(store.clone())
    );
}
