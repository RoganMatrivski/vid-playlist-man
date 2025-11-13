use anyhow::Result;
use itertools::Itertools;
use scraper::Selector;
use url::Url;

fn get_page_links(document: &scraper::html::Html) -> Vec<String> {
    let selector = Selector::parse("a").unwrap();

    document
        .select(&selector)
        .filter_map(|element| element.value().attr("href").map(|href| href.to_string()))
        .filter(|href| href.starts_with("page") && href.ends_with(".html"))
        .collect()
}

/// Extracts all links starting with a given prefix, removes query parameters
fn get_video_links(document: &scraper::html::Html, starts_with: &str) -> Vec<String> {
    let selector = Selector::parse("a").unwrap();

    document
        .select(&selector)
        .filter_map(|element| element.value().attr("href").map(|href| href.to_string()))
        .filter(|href| href.starts_with(starts_with))
        .filter_map(|href| {
            // Parse URL and strip query parameters
            if let Ok(mut parsed) = Url::parse(&href) {
                parsed.set_query(None);
                Some(parsed.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn get_baseurl(rawurl: &str) -> String {
    // Ensure the input has a scheme
    let mut url_input = rawurl.to_string();
    if !url_input.contains("://") {
        url_input = format!("http://{}", url_input);
    }

    // Parse using the `url` crate
    let parsed =
        Url::parse(&url_input).unwrap_or_else(|e| panic!("Failed to parse URL {}: {}", rawurl, e));

    // Construct base URL as "[protocol]://[hostname]"
    format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""))
}

const PKG_NAME: &str = env!("CARGO_PKG_NAME");

pub struct PlaylistFetcher {
    fetcher: crate::fetcher::Client,
    kv: crate::kvcache::KvCache,
}

impl PlaylistFetcher {
    pub fn new(kv: worker::KvStore) -> Self {
        Self {
            fetcher: crate::fetcher::Client::new(""),
            kv: crate::kvcache::KvCache::new(kv),
        }
    }
    async fn get_text_cached(&self, endpoint: &str) -> Result<String> {
        let keyname = format!("{PKG_NAME}_discord_{endpoint}");
        let kv_key = urlencoding::encode(&keyname);
        if let Some(cached) = self.kv.get_text(&kv_key).await? {
            tracing::trace!("KV HIT for {endpoint}");
            return Ok(cached);
        };

        tracing::trace!("KV MISS for {endpoint}");

        let res = self.fetcher.get_text(endpoint).await?;

        self.kv.set(&kv_key, &res, 60 * 30).await?;

        Ok(res)
    }

    pub async fn get(&self, url: &str) -> Result<String> {
        let vid_baseurl = get_baseurl(url) + "/video/";

        let res = self.get_text_cached(url).await?;
        let doc = scraper::Html::parse_document(&res);
        let pagelinks = get_page_links(&doc).into_iter().dedup().collect_vec();
        let vidlinks = get_video_links(&doc, &vid_baseurl);

        let pagenum: Vec<u32> = pagelinks
            .iter()
            .map(|x| {
                x[4..x.len() - 5]
                    .parse::<u32>()
                    .map_err(|e| anyhow::anyhow!("Failed to parse {x}: {e}"))
            })
            .try_collect()?;

        let maxpage = pagenum.into_iter().max().unwrap_or(1);
        let sem = std::sync::Arc::new(async_lock::Semaphore::new(8));

        let pagelinks = (2..(maxpage + 1))
            .map(|x| {
                let endpoint = format!("{url}/page{}.html", x);
                let vid_baseurl = vid_baseurl.clone();
                let sem = sem.clone();

                async move {
                    let _permit = sem.acquire().await;
                    tracing::trace!("Fetching page {x}");

                    let res = self.get_text_cached(&endpoint).await?;
                    let doc = scraper::Html::parse_document(&res);
                    let links = get_video_links(&doc, &vid_baseurl);

                    anyhow::Ok(links)
                }
            })
            .collect_vec();

        let links = futures::future::try_join_all(pagelinks).await?;

        let links = std::iter::once(vidlinks)
            .chain(links)
            .flatten()
            .collect_vec();

        Ok(links.join("\n"))
    }
}
