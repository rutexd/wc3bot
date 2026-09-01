use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

const GAMELIST_URL: &str = "https://api.wc3stats.com/gamelist";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub id: i64,
    #[serde(default)]
    pub name: String,
    pub host: String,
    pub map: String,
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub slots_taken: i32,
    #[serde(default)]
    pub slots_total: i32,
    #[serde(default)]
    pub created: i64,
}

use crate::loc::{tr, Lang};

impl Game {
    pub fn notification_text(&self, lang: Lang) -> String {
        let t = tr(lang);
        let name = if self.name.trim().is_empty() { t.dash } else { &self.name };
        format!(
            "{map}\n{name}\n{host}\n{slots}",
            map = t.ping_map.replace("{map}", &self.map),
            name = t.ping_name.replace("{name}", name).replace("{server}", &self.server.to_uppercase()),
            host = t.ping_host.replace("{host}", &self.host),
            slots = t.ping_slots.replace("{taken}", &self.slots_taken.to_string()).replace("{total}", &self.slots_total.to_string()),
        )
    }
}

pub async fn fetch_gamelist(client: &reqwest::Client) -> Result<Vec<Game>> {
    let resp = client
        .get(GAMELIST_URL)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .context("gamelist request failed")?;

    #[derive(Deserialize)]
    struct Resp {
        body: Vec<Game>,
    }

    let parsed: Resp = resp.json().await.context("gamelist json parse failed")?;
    Ok(parsed.body)
}
