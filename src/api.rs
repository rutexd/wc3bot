use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

const GAMELIST_URL: &str = "https://api.wc3stats.com/gamelist";

#[derive(Debug, Clone, Deserialize)]
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
}

impl Game {
    pub fn server_display(&self) -> String {
        self.server.to_uppercase()
    }

    pub fn notification_text(&self) -> String {
        format!(
            "🎮 Новая игра!\n\n\
             🗺️ Карта: {}\n\
             🏠 Хост: {}\n\
             📛 Название: {}\n\
             👥 Игроки: {}/{}\n\
             🌍 Сервер: {}\n\
             🆔 ID игры: {}",
            self.map,
            self.host,
            if self.name.trim().is_empty() { "—" } else { &self.name },
            self.slots_taken,
            self.slots_total,
            self.server_display(),
            self.id
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
