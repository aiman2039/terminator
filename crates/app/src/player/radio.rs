//! Icecast/Shoutcast HTTP(S) stations. No HLS, no Blitz, no JS.
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::sync::OnceLock;
use url::Url;

const CATALOG_JSON: &str = include_str!("../../assets/radio/stations.json");

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Station {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub icon: String,
}

#[derive(Deserialize)]
struct CatalogFile {
    stations: Vec<Station>,
}

pub fn catalog() -> &'static [Station] {
    static CATALOG: OnceLock<Vec<Station>> = OnceLock::new();
    CATALOG.get_or_init(load_catalog)
}

fn load_catalog() -> Vec<Station> {
    serde_json::from_str::<CatalogFile>(CATALOG_JSON)
        .map(|file| file.stations)
        .unwrap_or_default()
}

pub fn categories() -> Vec<String> {
    let mut out = Vec::new();
    for station in catalog() {
        if !station.category.is_empty() && !out.iter().any(|item| item == &station.category) {
            out.push(station.category.clone());
        }
    }
    out.sort();
    out
}

pub fn matches_filter(station: &Station, query: &str, category: &str) -> bool {
    if !category.is_empty() && station.category != category {
        return false;
    }
    if query.is_empty() {
        return true;
    }
    station.name.to_lowercase().contains(query)
        || station.category.to_lowercase().contains(query)
        || station.country.to_lowercase().contains(query)
        || station.language.to_lowercase().contains(query)
}

pub fn parse_stream_url(raw: &str) -> Result<String> {
    let url = Url::parse(raw.trim())?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "Only HTTP(S) radio streams"
    );
    ensure!(url.host_str().is_some(), "Stream URL needs a host");
    ensure!(
        !matches!(url.scheme(), "file" | "javascript" | "data"),
        "Blocked URL scheme"
    );
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_and_https_streams_are_accepted() {
        assert!(parse_stream_url("https://ice1.somafm.com/groovesalad-128-mp3").is_ok());
        assert!(parse_stream_url("http://example.com:8000/stream").is_ok());
    }

    #[test]
    fn file_and_script_urls_are_rejected() {
        assert!(parse_stream_url("file:///tmp/song.mp3").is_err());
        assert!(parse_stream_url("javascript:alert(1)").is_err());
        assert!(parse_stream_url("data:audio/mp3;base64,xx").is_err());
        assert!(parse_stream_url("not a url").is_err());
    }

    #[test]
    fn bundled_catalog_is_https_and_unique() {
        let stations = catalog();
        assert!(stations.len() > 100);
        let mut urls = std::collections::HashSet::new();
        for station in stations {
            let url = parse_stream_url(&station.url).unwrap();
            assert!(url.starts_with("https://"), "{url}");
            assert!(urls.insert(url), "duplicate {}", station.url);
            assert!(!station.name.is_empty());
        }
    }

    #[test]
    fn filter_matches_name_and_category() {
        let groove = catalog()
            .iter()
            .find(|station| station.name.contains("Groove Salad"))
            .expect("groove");
        assert!(matches_filter(groove, "salad", ""));
        assert!(matches_filter(groove, "", "ambient"));
        assert!(!matches_filter(groove, "jazz", ""));
        assert!(!matches_filter(groove, "", "jazz"));
    }
}
