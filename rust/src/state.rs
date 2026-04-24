use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    pub agents: BTreeMap<String, Entry>,
}

impl Store {
    pub fn empty() -> Self {
        Self {
            agents: BTreeMap::new(),
        }
    }

    pub fn from_json(json: &str) -> Result<Self> {
        if json.trim().is_empty() {
            return Ok(Self::empty());
        }
        let mut store: Self = serde_json::from_str(json).context("parse state json")?;
        if store.agents.is_empty() {
            store.agents = BTreeMap::new();
        }
        Ok(store)
    }

    pub fn load(path: &str) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(content) => Self::from_json(&content),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::empty()),
            Err(err) => Err(err).with_context(|| format!("read {path}")),
        }
    }

    pub fn get(&self, agent_id: &str) -> Option<&Entry> {
        self.agents.get(agent_id)
    }

    pub fn put(&mut self, agent_id: impl Into<String>, mut entry: Entry) {
        entry.last_seen = Some(current_utc_rfc3339());
        self.agents.insert(agent_id.into(), entry);
    }

    pub fn to_json_pretty(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("marshal state json")
    }

    pub fn save(&self, path: &str) -> Result<()> {
        let content = self.to_json_pretty()?;
        let tmp = format!("{path}.tmp");
        std::fs::write(&tmp, content).with_context(|| format!("write {tmp}"))?;
        std::fs::rename(&tmp, path).with_context(|| format!("rename {tmp} to {path}"))
    }
}

pub(crate) fn current_utc_rfc3339() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    unix_seconds_to_utc_rfc3339(timestamp)
}

fn unix_seconds_to_utc_rfc3339(seconds: i64) -> String {
    const SECONDS_PER_DAY: i64 = 86_400;
    let days = seconds.div_euclid(SECONDS_PER_DAY);
    let seconds_of_day = seconds.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::unix_seconds_to_utc_rfc3339;

    #[test]
    fn formats_unix_seconds_as_utc_rfc3339() {
        assert_eq!(unix_seconds_to_utc_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            unix_seconds_to_utc_rfc3339(1_776_989_696),
            "2026-04-24T00:14:56Z"
        );
    }
}
