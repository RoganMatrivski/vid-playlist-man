use std::{rc::Rc, str::FromStr};

use anyhow::{Result, anyhow};
use backon::{ExponentialBuilder, Retryable};
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use worker::{Cache, Fetch, Headers, RequestInit};

#[derive(Debug, Clone)]
pub struct Client {
    base_url: String,
    headers: HeaderMap,

    cache: Rc<Cache>,
    cache_ttl: usize,
}

pub struct RequestHeaders(pub Headers);

impl From<&HeaderMap> for RequestHeaders {
    fn from(map: &HeaderMap) -> Self {
        let headers = Headers::new();

        for (key, value) in map.iter() {
            if let (name, Ok(value_str)) = (key, value.to_str()) {
                headers.append(name.as_str(), value_str);
            }
        }

        RequestHeaders(headers)
    }
}

impl From<RequestHeaders> for Headers {
    fn from(wrapper: RequestHeaders) -> Self {
        wrapper.0
    }
}

impl TryFrom<RequestHeaders> for HeaderMap {
    type Error = anyhow::Error;

    fn try_from(wrapper: RequestHeaders) -> Result<Self> {
        let value = wrapper.0;

        let mut headers = HeaderMap::new();

        for (key, value) in value.entries() {
            headers.append(HeaderName::from_str(&key)?, HeaderValue::from_str(&value)?);
        }

        Ok(headers)
    }
}

#[derive(Debug)]
struct HttpError {
    status: u16,
    headers: HeaderMap,
    message: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP error {}: {}", self.status, self.message)
    }
}

impl std::error::Error for HttpError {}

impl Client {
    pub fn new(base_url: impl ToString) -> Self {
        Self {
            base_url: base_url.to_string(),
            headers: HeaderMap::new(),

            cache: Rc::new(Cache::default()),
            cache_ttl: 60,
        }
    }

    pub fn with_headers(self, headers: HeaderMap) -> Self {
        Self { headers, ..self }
    }

    pub fn with_cache_ttl(self, ttl: usize) -> Self {
        Self {
            cache_ttl: ttl,
            ..self
        }
    }

    pub async fn fetch(&self, endpoint: &str) -> Result<Vec<u8>> {
        let url = format!("{}{endpoint}", &self.base_url);
        let fetchcall = || async {
            let mut res = if let Some(cached) = self.cache.get(&url, false).await? {
                tracing::trace!("Cache HIT for {url}");
                cached
            } else {
                tracing::trace!("Cache MISS for {url}");
                let req = worker::Request::new_with_init(
                    &url,
                    RequestInit::new().with_headers(self.headers.clone().into()),
                )?;
                let mut res = Fetch::Request(req).send().await?;
                let mut cloned_res = res.cloned()?;

                cloned_res.headers_mut().set(
                    "Cache-Control",
                    &format!("private=Set-Cookie,max-age={}", self.cache_ttl),
                )?;
                self.cache.put(&url, cloned_res.cloned()?).await?;

                res
            };

            if res.status_code() != StatusCode::OK {
                let src = HttpError {
                    status: res.status_code(),
                    headers: RequestHeaders(res.headers().clone()).try_into()?,
                    message: format!("Request failed with status {}", res.status_code()),
                };
                return Err(anyhow::Error::new(src));
            }

            Ok(res.bytes().await?)
        };

        let res = fetchcall
            .retry(ExponentialBuilder::default().with_jitter().with_max_times(5).with_min_delay(std::time::Duration::from_secs(1)))
            .adjust(|err, dur| match err.downcast_ref::<HttpError>() {
                Some(v) => {
                    if v.status == StatusCode::TOO_MANY_REQUESTS {
                        let retry_after = if let Some(retry_after) = v.headers.get("Retry-After") {
                            // Parse the Retry-After header and adjust the backoff
                            let retry_after = retry_after.to_str().unwrap_or("30");
                            retry_after.parse::<u64>().unwrap_or(30)
                        } else {
                            30u64
                        };

                        if retry_after > 60 * 15 {
                            // Retry after is more than 15 mins. Maybe abort
                            tracing::error!("Retry-After returns duration more than 15 minutes ({retry_after}). Cancelling...");
                            return None
                        }

                        Some(std::time::Duration::from_secs(retry_after))
                    } else {
                        dur
                    }
                }
                None => dur,
            })
            .notify(|err, dur| {
                tracing::warn!("retrying {:?} after {:?}", err, dur);
            })
            .await?;

        Ok(res)
    }

    /// Internal helper to send authorized GET requests and parse JSON
    pub async fn get_json<T>(&self, endpoint: &str) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let res = self.fetch(endpoint).await?;
        Ok(serde_json::from_slice(&res)?)
    }

    pub async fn get_text(&self, endpoint: &str) -> Result<String> {
        Ok(String::from_utf8(self.fetch(endpoint).await?)?)
    }
}
