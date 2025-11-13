use std::sync::LazyLock;

use itertools::Itertools;
use serde::{Deserialize, Serialize};

use anyhow::Result;
use time::UtcDateTime;

const DISCORD_API: &str = "https://discord.com/api/v10";
const PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[derive(Clone)]
pub struct DiscordClient {
    fetcher: crate::fetcher::Client,
    kv: crate::kvcache::KvCache,
}

#[allow(dead_code)]
impl DiscordClient {
    pub fn new(token: impl AsRef<str>, kv: worker::KvStore) -> Result<Self> {
        let mut headers = http::HeaderMap::new();
        headers.append(
            "User-Agent",
            http::HeaderValue::from_str("DiscordOccasionalMsgFetcher (gloo-net, v0.1)")?,
        );
        headers.append(
            "Authorization",
            http::HeaderValue::from_str(token.as_ref())?,
        );

        Ok(Self {
            fetcher: crate::fetcher::Client::new(DISCORD_API).with_headers(headers),
            kv: crate::kvcache::KvCache::new(kv),
        })
    }

    /// Internal helper to send authorized GET requests and parse JSON
    async fn get_json<T>(&self, endpoint: &str) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        self.fetcher.get_json(endpoint).await
    }

    /// Internal helper to send authorized GET requests and parse JSON
    async fn get_json_cached<T>(&self, endpoint: &str) -> Result<T>
    where
        T: serde::de::DeserializeOwned + serde::Serialize,
    {
        let keyname = format!("{PKG_NAME}_discord_{endpoint}");
        let kv_key = urlencoding::encode(&keyname);
        if let Some(cached) = self.kv.get_json::<T>(&kv_key).await? {
            tracing::trace!("KV HIT for {endpoint}");
            return Ok(cached);
        };

        tracing::trace!("KV MISS for {endpoint}");

        let res = self.get_json::<T>(endpoint).await?;

        self.kv.set(&kv_key, &res, 604_800).await?;

        Ok(res)
    }

    /// Get channel info (returns name + guild_id)
    pub async fn get_channel(&self, channel_id: &str) -> Result<Channel> {
        self.get_json_cached::<Channel>(&format!("/channels/{channel_id}"))
            .await
    }

    /// Get guild info (returns name)
    pub async fn get_guild(&self, guild_id: &str) -> Result<Guild> {
        self.get_json_cached::<Guild>(&format!("/guilds/{guild_id}"))
            .await
    }

    /// Get the last N messages
    pub async fn get_messages(&self, channel_id: &str, limit: u8) -> Result<Vec<Message>> {
        if limit == 0 {
            panic!("get_messages limit should be non-zero")
        }

        self.get_json::<Vec<Message>>(&format!("/channels/{channel_id}/messages?limit={}", limit))
            .await
    }

    /// Get messages before a given Snowflake ID
    pub async fn get_messages_before(
        &self,
        channel_id: &str,
        before_id: &str,
        limit: u8,
    ) -> Result<Vec<Message>> {
        if limit == 0 {
            panic!("get_messages_before limit should be non-zero")
        }

        self.get_json::<Vec<Message>>(&format!(
            "/channels/{channel_id}/messages?before={}&limit={}",
            before_id, limit
        ))
        .await
    }

    pub async fn get_messages_range(
        &self,
        channel_id: &str,
        date_range: impl std::ops::RangeBounds<time::UtcDateTime>,
        limit: Option<usize>,
    ) -> Result<Vec<Message>> {
        let mut messages = Vec::<Message>::new();
        // let range = date_range.start_bound()
        // let before_id = utils::unix_ms_to_snowflake(timestamp_ms, worker_id, sequence)

        let filter_msg = |msgs: Vec<Message>| {
            // Limit messages to containing date_range
            // Messages sorted by newest, descending
            // Find middlepoint to split if any
            let split_idx = msgs.partition_point(|x| {
                if let Ok(t) = x.timestamp() {
                    date_range.contains(&t)
                } else {
                    false
                }
            });

            // If split_idx is anywhere below 100
            if split_idx < 100 {
                return msgs[..split_idx].to_vec();
            }

            msgs
        };

        // First round of message batch
        messages.append(&mut filter_msg(match date_range.end_bound() {
            std::ops::Bound::Included(&d) | std::ops::Bound::Excluded(&d) => {
                let before_id = utils::unix_ms_to_snowflake(d.unix_timestamp() * 1000, 0, 0)?;
                self.get_messages_before(
                    channel_id,
                    &before_id,
                    limit.unwrap_or(100).min(100) as u8,
                )
                .await?
            }
            std::ops::Bound::Unbounded => {
                self.get_messages(channel_id, limit.unwrap_or(100).min(100) as u8)
                    .await?
            }
        }));

        if messages.is_empty() {
            return Ok(vec![]);
        }

        if messages.len() < 100 {
            return Ok(messages);
        }

        tracing::info!("Msg more than 100. Fetching more...");

        // Safety measure in case of a runouts
        // Limit fetch loop to 5 min
        let timeout_now = web_time::Instant::now();
        let timeout_dur = web_time::Duration::from_secs(60 * 5);

        //The loop continues while all these are true:
        //  there’s no limit or if we’re under the limit.
        //  There is a last message,
        //  Its timestamp is valid,
        //  That timestamp is inside the date_range.
        //  Also within safety margin
        while limit.is_none_or(|limit| messages.len() <= limit)
            && let Some(lastmsg) = messages.last()
            && let Ok(x) = lastmsg.timestamp()
            && date_range.contains(&x)
            && timeout_now.elapsed() < timeout_dur
        {
            let cap = if let Some(l) = limit {
                (l - messages.len()).min(100)
            } else {
                100
            } as u8;

            let mut newmsg = filter_msg(
                self.get_messages_before(channel_id, &messages.last().unwrap().id, cap)
                    .await?,
            );

            if newmsg.is_empty() {
                break;
            }

            messages.append(&mut newmsg);
        }

        Ok(messages)
    }
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub guild_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug)]
pub struct Guild {
    pub id: String,
    pub name: String,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct User {
    pub id: String,
    pub username: String,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct Message {
    pub id: String,
    pub content: String,
    pub author: User,
}

impl Message {
    /// Return the timestamp of the message based on the Snowflake ID
    pub fn timestamp(&self) -> Result<UtcDateTime> {
        utils::snowflake_to_utc_datetime(&self.id)
    }
}

#[allow(dead_code)]
mod utils {
    use anyhow::*;
    use time::UtcDateTime;
    use time::{Date, Month};

    const DISCORD_EPOCH: i64 = 1420070400000; // milliseconds since unix epoch

    /// Extract the Unix timestamp in milliseconds from a Discord snowflake string.
    pub fn snowflake_to_unix_ms(s: &str) -> Result<i64> {
        let snowflake: u64 = s
            .parse()
            .map_err(|e| anyhow!("invalid snowflake '{}': {}", s, e))?;
        let ts_offset_ms = (snowflake >> 22) as i64;
        Ok(ts_offset_ms + DISCORD_EPOCH)
    }

    /// Extract the Discord timestamp as a `UtcDateTime`.
    pub fn snowflake_to_utc_datetime(s: &str) -> Result<UtcDateTime> {
        let ms = snowflake_to_unix_ms(s)?;
        // Matches the existing usage elsewhere in the codebase which constructs from ms.
        UtcDateTime::from_unix_timestamp(ms / 1000).map_err(|e| anyhow!("invalid timestamp: {}", e))
    }

    /// Construct a Discord snowflake from a Unix timestamp in milliseconds, plus a worker id and sequence.
    ///
    /// - `timestamp_ms`: unix milliseconds (must be >= DISCORD_EPOCH)
    /// - `worker_id`: 10-bit value (0..=1023)
    /// - `sequence`: 12-bit value (0..=4095)
    pub fn unix_ms_to_snowflake(
        timestamp_ms: i64,
        worker_id: u16,
        sequence: u16,
    ) -> Result<String> {
        if timestamp_ms < DISCORD_EPOCH {
            return Err(anyhow!("timestamp before Discord epoch"));
        }
        if worker_id > 0x3FF {
            return Err(anyhow!("worker_id must be <= 1023"));
        }
        if sequence > 0xFFF {
            return Err(anyhow!("sequence must be <= 4095"));
        }

        let offset = (timestamp_ms - DISCORD_EPOCH) as u64;
        // Ensure offset fits into 42 bits
        if offset >> 42 != 0 {
            return Err(anyhow!(
                "timestamp too large to encode in a Discord snowflake"
            ));
        }

        let snowflake = (offset << 22) | ((worker_id as u64) << 12) | (sequence as u64);
        Ok(snowflake.to_string())
    }

    pub fn parse_month(s: &str) -> Result<Date> {
        let s = s.replace('-', "");
        if s.len() != 6 {
            return Err(anyhow!("Invalid date format, expected yyyyMM"));
        }

        let year: i32 = s[0..4].parse()?;
        let month: u8 = s[4..6].parse()?;

        if !(1..=12).contains(&month) {
            return Err(anyhow!("Month must be between 1 and 12"));
        }

        Ok(Date::from_calendar_date(year, Month::try_from(month)?, 1)?)
    }
}

pub async fn mainfn(env: &worker::Env, sched_diff: i64) -> Result<()> {
    let token = env.secret("DISCORD_TOKEN")?;
    let channels = env.secret("DISCORD_CHANNEL_IDS")?.to_string();
    let channels = channels.split(",").collect::<Vec<_>>();

    let kv = env.kv("VID_PLAYLIST_MANAGER_KV")?;

    let client = DiscordClient::new(token.to_string(), env.kv("KVCACHE")?)?;

    let currtime = time::UtcDateTime::now();
    let prevtime = currtime.saturating_sub(time::Duration::minutes(sched_diff));

    {
        let timefmt = time::format_description::parse("[hour]:[minute]:[second]")?;
        let timestr = currtime.format(&timefmt)?;
        tracing::debug!("It is currently {timestr}");
    }

    let range = prevtime..currtime;
    tracing::debug!("{range:?}");

    let sem = std::sync::Arc::new(async_lock::Semaphore::new(8));

    let urls_getter = futures::future::join_all(
        channels
            .iter()
            .map(|x| (x, client.clone(), range.clone(), sem.clone()))
            .map(|(x, c, r, sem)| async move {
                let _permit = sem.acquire().await;
                ch_fetcher(&c, x, r).await
            }),
    )
    .await;

    let (urls, errs): (Vec<Vec<String>>, Vec<anyhow::Error>) =
        urls_getter.into_iter().partition_result();

    errs.iter()
        .for_each(|err| tracing::error!(?err, "Fetch failed"));

    let urls = urls.into_iter().flatten().collect_vec();

    if urls.is_empty() {
        let emfmt = time::format_description::parse("[hour]:[minute]:[second]")?;
        let emtime = prevtime.format(&emfmt)?;
        tracing::info!("No new links since {emtime}. Skipping sending to KV.");

        return Ok(());
    }

    let timefmt = time::format_description::parse("[year]-[month]")?;
    let timestr = prevtime.format(&timefmt)?;

    let kvname = format!("{timestr}_discord_merged");
    let kvvalue = &urls.join("\n");

    {
        tracing::debug!("Getting previous KV to append");
        let prev = kv
            .get(&kvname)
            .text()
            .await
            .expect("Failed prepping KV get")
            .unwrap_or("".into());
        let newval = prev + "\n" + kvvalue.as_ref();

        tracing::info!("Sending to KV");
        kv.put(&kvname, &newval)
            .expect("Failed prepping KV send")
            .execute()
            .await
            .expect("Failed sending KV");
        tracing::info!("Done!");
    }

    Ok(())
}

const EXCLUDED_PATTERNS: &[&str] = &[
    "cdn.",
    "tenor.",
    "redgifs.",
    "discordapp.",
    "redd.it",
    "media.tumblr.",
];

static FINDER: LazyLock<linkify::LinkFinder> = LazyLock::new(linkify::LinkFinder::new);
static EXCLUDER: LazyLock<aho_corasick::AhoCorasick> = LazyLock::new(|| {
    aho_corasick::AhoCorasick::builder()
        .ascii_case_insensitive(true)
        .build(EXCLUDED_PATTERNS)
        .expect("Failed to init filter")
});

#[tracing::instrument(skip(client, range))]
async fn ch_fetcher(
    client: &DiscordClient,
    ch_id: &str,
    range: impl std::ops::RangeBounds<UtcDateTime>,
) -> Result<Vec<String>> {
    let ch = client.get_channel(ch_id).await?;
    let chname = ch.name;
    let srv_id = ch
        .guild_id
        .expect("Failed to get Server ID (this shouldn't've been possible");
    let srvname = client.get_guild(&srv_id).await?.name;
    // let msg: Vec<Message> = client.get_messages(ch, 1).await?;
    let msg_res = client.get_messages_range(ch_id, range, None).await?;

    if let Some(m) = msg_res.first() {
        let snip = m.content.clone();
        let t_str = m
            .timestamp()?
            .format(&time::format_description::well_known::Rfc3339)?;
        tracing::debug!("First message snippet: [{t_str}] {snip}");
    }

    let msgcount = msg_res.len();
    tracing::trace!("msgcount: {msgcount}");

    let links = msg_res
        .into_iter()
        .map(|x| x.content)
        .flat_map(|x| {
            FINDER
                .links(&x)
                .map(|x| x.as_str().to_string())
                .collect_vec()
        })
        .collect_vec();

    let filtered_count = links.iter().filter(|x| EXCLUDER.is_match(x)).count();

    tracing::info!(
        "Fetched from {chname} ({srvname}): {} new message, {} new links, {} links excluded",
        if msgcount == 0 {
            "No"
        } else {
            &msgcount.to_string()
        },
        if links.is_empty() {
            "no"
        } else {
            &links.len().to_string()
        },
        if filtered_count == 0 {
            "no"
        } else {
            &filtered_count.to_string()
        }
    );

    Ok(links
        .into_iter()
        .filter(|x| !EXCLUDER.is_match(x))
        .collect_vec())
}
