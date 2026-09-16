//! Icecast/Shoutcast HTTP(S) stations. No HLS, no Blitz, no JS.
use anyhow::{Result, ensure};
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Station {
    pub name: &'static str,
    pub url: &'static str,
}

pub fn bundled() -> &'static [Station] {
    &[
        Station {
            name: "SomaFM Groove Salad",
            url: "https://ice1.somafm.com/groovesalad-128-mp3",
        },
        Station {
            name: "SomaFM Drone Zone",
            url: "https://ice1.somafm.com/dronezone-128-mp3",
        },
        Station {
            name: "Radio Paradise",
            url: "https://stream.radioparadise.com/mp3-128",
        },
        Station {
            name: "SomaFM Indie Pop Rocks",
            url: "https://ice1.somafm.com/indiepop-128-mp3",
        },
    ]
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
    fn bundled_stations_are_https() {
        for station in bundled() {
            let url = parse_stream_url(station.url).unwrap();
            assert!(url.starts_with("https://"), "{url}");
        }
    }
}
