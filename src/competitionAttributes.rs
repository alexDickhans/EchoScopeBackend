use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use robotevents::schema::{AllianceColor, Match};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CompetitionAttributesContentState {
    pub last_match: Option<DisplayMatch>,
    pub next_match: Option<DisplayMatch>,
    pub team_next_match: Option<DisplayMatch>
}

impl From<(Vec<Match>, String)> for CompetitionAttributesContentState {
    fn from(value: (Vec<Match>, String)) -> Self {
        let (matches, team_name) = value;
        let team_name = team_name.to_uppercase();

        let mut last_scored_index = 0;
        let mut team_next_match = 0;
        let mut matches_skipped = true;

        for (index, m) in matches.iter().enumerate() {
            // Check if match has a score
            if m.scored {
                last_scored_index = index;
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
            .and_then(|s| datetime_from_string(s));

        let start_time = m.started.as_ref()
            .and_then(|s| datetime_from_string(s));

        // Find red alliance
        let red_alliance = m.alliances.iter()
            .find(|a| matches!(a.color, AllianceColor::Red));

        // Find blue alliance
        let blue_alliance = m.alliances.iter()
            .find(|a| matches!(a.color, AllianceColor::Blue));

        DisplayMatch {
            name: format!("{} {}", m.round.to_string(), m.instance),
            scheduled,
            start_time,
            red_alliance: Alliance {
                team1: red_alliance
                    .and_then(|a| a.teams.first())
                    .map_or_else(|| String::new(), |t| t.team.name.to_string()),
                team2: red_alliance
                    .and_then(|a| a.teams.get(1))
                    .map(|t| t.team.name.to_string()),
                score: red_alliance.map(|a| a.score),
            },
            blue_alliance: Alliance {
                team1: blue_alliance
                    .and_then(|a| a.teams.first())
                    .map_or_else(|| String::new(), |t| t.team.name.to_string()),
                team2: blue_alliance
                    .and_then(|a| a.teams.get(1))
                    .map(|t| t.team.name.to_string()),
                score: blue_alliance.map(|a| a.score),
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

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DisplayMatch {
    pub name: String,
    pub scheduled: Option<DateTime<Utc>>,
    pub start_time: Option<DateTime<Utc>>,
    pub red_alliance: Alliance,
    pub blue_alliance: Alliance,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Alliance {
    pub team1: String,
    pub team2: Option<String>,
    pub score: Option<i32>
}



