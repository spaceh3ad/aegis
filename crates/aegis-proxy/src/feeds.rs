use std::{path::PathBuf, sync::Mutex, time::Duration};

use aegis_core::{AddressSets, Blocklist, BlocklistError};
use thiserror::Error;

use crate::config::FeedSpec;

pub const MAX_FEED_BYTES: usize = 32 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Error)]
pub enum FeedError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("feed body exceeds {MAX_FEED_BYTES} bytes")]
    TooLarge,
    #[error("feed body is not utf-8")]
    NotUtf8,
    #[error(transparent)]
    Parse(#[from] BlocklistError),
}

/// Owns every blocklist source (local file + operator-chosen feeds) and republishes their
/// union into the shared [`Blocklist`] whenever one changes. A failing source keeps its
/// last good contents, so a feed outage never silently empties the list.
pub struct SourceRegistry {
    blocklist: Blocklist,
    file_path: PathBuf,
    feeds: Vec<FeedSpec>,
    state: Mutex<Sources>,
}

struct Sources {
    file: AddressSets,
    feeds: Vec<Option<AddressSets>>,
}

impl SourceRegistry {
    pub fn new(
        blocklist: Blocklist,
        file_path: PathBuf,
        feeds: Vec<FeedSpec>,
    ) -> Result<Self, BlocklistError> {
        let file = AddressSets::parse("file", &std::fs::read_to_string(&file_path)?)?;
        let registry = Self {
            blocklist,
            file_path,
            state: Mutex::new(Sources {
                file,
                feeds: vec![None; feeds.len()],
            }),
            feeds,
        };
        registry.publish(&registry.state.lock().unwrap());
        Ok(registry)
    }

    pub fn feeds(&self) -> &[FeedSpec] {
        &self.feeds
    }

    /// Returns the merged entry count.
    pub fn reload_file(&self) -> Result<usize, BlocklistError> {
        let file = AddressSets::parse("file", &std::fs::read_to_string(&self.file_path)?)?;
        let mut state = self.state.lock().unwrap();
        state.file = file;
        Ok(self.publish(&state))
    }

    /// Fetches every feed; returns one result per feed (entry count on success).
    pub async fn refresh_feeds(&self, client: &reqwest::Client) -> Vec<Result<usize, FeedError>> {
        let mut results = Vec::with_capacity(self.feeds.len());
        for (i, feed) in self.feeds.iter().enumerate() {
            let fetched = fetch(client, feed).await;
            results.push(match fetched {
                Ok(sets) => {
                    let n = sets.len();
                    let mut state = self.state.lock().unwrap();
                    state.feeds[i] = Some(sets);
                    self.publish(&state);
                    Ok(n)
                }
                Err(e) => Err(e),
            });
        }
        results
    }

    fn publish(&self, state: &Sources) -> usize {
        let merged =
            AddressSets::merge(std::iter::once(&state.file).chain(state.feeds.iter().flatten()));
        let n = merged.len();
        self.blocklist.replace(merged);
        n
    }
}

async fn fetch(client: &reqwest::Client, feed: &FeedSpec) -> Result<AddressSets, FeedError> {
    let mut resp = client
        .get(&feed.url)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await?
        .error_for_status()?;
    if resp
        .content_length()
        .is_some_and(|n| n as usize > MAX_FEED_BYTES)
    {
        return Err(FeedError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if body.len() + chunk.len() > MAX_FEED_BYTES {
            return Err(FeedError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(body).map_err(|_| FeedError::NotUtf8)?;
    Ok(AddressSets::parse(&feed.name, &text)?)
}
