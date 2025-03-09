use std::time::SystemTime;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, TimestampSeconds};
use robotevents::schema::{AllianceColor, Match};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CompetitionAttributesContentState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_match: Option<DisplayMatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_match: Option<DisplayMatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_next_match: Option<DisplayMatch>
}

impl CompetitionAttributesContentState {
    pub fn from_matchlist(unsorted_matches: &[Match], team_name: &str) -> Self {
        let team_name = team_name.to_uppercase();

        let mut matches= unsorted_matches.clone().to_vec();

        matches.sort_by(|x, x1| {
            let mut round_sum_x = x.round as f32;
            round_sum_x = if round_sum_x == 6.0 { 2.5 } else { round_sum_x };
            round_sum_x = round_sum_x * 1000.0 + x.matchnum as f32;
            let mut round_sum_x1 = x1.round as f32;
            round_sum_x1 = if round_sum_x1 == 6.0 { 2.5 } else { round_sum_x1 };
            round_sum_x1 = round_sum_x1 * 1000.0 + x1.matchnum as f32;
            round_sum_x.total_cmp(&round_sum_x1)
        });


        let mut last_scored_index = 0;
        let mut team_next_match = 0;
        let mut matches_skipped = true;

        for (index, m) in matches.iter().enumerate() {
            // Check if match has a score
            if m.alliances[0].score != 0 || m.alliances[1].score != 0 {
                last_scored_index = index;
                matches_skipped = true;
            } else {
                matches_skipped = false;
            }

            // Check if team is in this match
            let team_in_match = m.alliances.iter()
                .flat_map(|a| &a.teams)
                .any(|team| team.team.name.to_string().to_uppercase() == team_name);

            if team_in_match {
                team_next_match = index;

                if !matches_skipped {
                    break;
                }
            }
        }

        // Get matches using safe indexing
        let last_match = matches.get(last_scored_index)
            .map(|m| DisplayMatch::from(m));

        let next_match = matches.get(last_scored_index + 1)
            .map(|m| DisplayMatch::from(m));

        let team_next_match = matches.get(team_next_match)
            .map(|m| DisplayMatch::from(m));

        CompetitionAttributesContentState {
            last_match,
            next_match,
            team_next_match,
        }
    }
}

impl From<&Match> for DisplayMatch {
    fn from(m: &Match) -> Self {
        // Parse date strings into DateTime<Utc>
        let scheduled = m.scheduled.as_ref()
            .and_then(|s| datetime_from_string(s).map(|dt| dt.into()));

        let start_time = m.started.as_ref()
            .and_then(|s| datetime_from_string(s).map(|dt| dt.into()));

        // Find red alliance
        let red_alliance = m.alliances.iter()
            .find(|a| matches!(a.color, AllianceColor::Red));

        // Find blue alliance
        let blue_alliance = m.alliances.iter()
            .find(|a| matches!(a.color, AllianceColor::Blue));

        let re = regex::Regex::new(r"[a-z#]").unwrap();
        let cleaned_name = re.replace_all(&m.name, "");

        // if both scores are zero set them to None
        let red_score = red_alliance.map(|a| a.score).unwrap_or(0);
        let blue_score = blue_alliance.map(|a| a.score).unwrap_or(0);

        let red_score_new = if red_score == 0 && blue_score == 0 { None } else { Some(red_score) };
        let blue_score_new = if red_score == 0 && blue_score == 0 { None } else { Some(blue_score) };

        DisplayMatch {
            name: cleaned_name.to_string(),
            scheduled,
            start_time,
            red_alliance: Alliance {
                team1: red_alliance
                    .and_then(|a| a.teams.first())
                    .map_or_else(|| String::new(), |t| t.team.name.to_string()),
                team2: red_alliance
                    .and_then(|a| a.teams.get(1))
                    .map(|t| t.team.name.to_string()),
                score: red_score_new,
            },
            blue_alliance: Alliance {
                team1: blue_alliance
                    .and_then(|a| a.teams.first())
                    .map_or_else(|| String::new(), |t| t.team.name.to_string()),
                team2: blue_alliance
                    .and_then(|a| a.teams.get(1))
                    .map(|t| t.team.name.to_string()),
                score: blue_score_new,
            },
        }
    }
}

// Helper function to parse date strings
fn datetime_from_string(date_str: &str) -> Option<DateTime<Utc>> {
    // Attempt to parse with different formats
    if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
        return Some(dt.with_timezone(&Utc));
    }

    // Try other common formats if needed
    None
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DisplayMatch {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde_as(as = "Option<TimestampSeconds<f64>>")]
    pub scheduled: Option<SystemTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde_as(as = "Option<TimestampSeconds<f64>>")]
    pub start_time: Option<SystemTime>,
    pub red_alliance: Alliance,
    pub blue_alliance: Alliance,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Alliance {
    pub team1: String,
    pub team2: Option<String>,
    pub score: Option<i32>
}
