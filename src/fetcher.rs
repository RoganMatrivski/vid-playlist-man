use std::str::FromStr;

use anyhow::{Result, anyhow};
use backon::{ExponentialBuilder, Retryable};
use gloo_net::http::{Headers, Request, Response};
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use worker::console_error;

#[derive(Debug, Clone)]
pub struct Client {
    base_url: String,
    headers: HeaderMap,
}

pub struct GlooHeaders(pub Headers);

impl From<&HeaderMap> for GlooHeaders {
    fn from(map: &HeaderMap) -> Self {
        let headers = Headers::new();

        for (key, value) in map.iter() {
            if let (name, Ok(value_str)) = (key, value.to_str()) {
                headers.append(name.as_str(), value_str);
            }
        }

        GlooHeaders(headers)
    }
}

impl From<GlooHeaders> for Headers {
    fn from(wrapper: GlooHeaders) -> Self {
        wrapper.0
    }
}

impl TryFrom<GlooHeaders> for HeaderMap {
    type Error = anyhow::Error;

    fn try_from(wrapper: GlooHeaders) -> Result<Self> {
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
        }
    }

    pub fn with_headers(&self, headers: HeaderMap) -> Self {
        Self {
            headers,
            ..self.clone()
        }
    }

    pub async fn fetch(&self, endpoint: &str) -> Result<Vec<u8>> {
        let url = format!("{}{endpoint}", &self.base_url);
        let fetchcall = || async {
            let res: Response = Request::get(&url)
                .headers(GlooHeaders::from(&self.headers).into())
                .send()
                .await
                .map_err(|e| anyhow!("Network error: {}", e))?;

            if res.status() != StatusCode::OK {
                let src = HttpError {
                    status: res.status(),
                    headers: GlooHeaders(res.headers()).try_into()?,
                    message: format!("Request failed with status {}", res.status()),
                };
                return Err(anyhow::Error::new(src));
            }

            Ok(res.binary().await?)
        };

        let res = fetchcall
            .retry(ExponentialBuilder::default().with_jitter())
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
                            console_error!("Retry-After returns duration more than 15 minutes ({retry_after}). Cancelling...");
                            return None
                        }

                        Some(std::time::Duration::from_secs(retry_after))
                    } else {
                        dur
                    }.max(Some(std::time::Duration::from_secs(1)))
                }
                None => dur,
            })
            .notify(|err, dur| {
                worker::console_warn!("retrying {:?} after {:?}", err, dur);
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
