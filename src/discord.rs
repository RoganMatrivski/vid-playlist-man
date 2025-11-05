use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gloo_net::http::{Request, Response};
use itertools::Itertools;
use serde::Deserialize;

use anyhow::{Result, anyhow};
use time::UtcDateTime;
use worker::console_log;

const DISCORD_API: &str = "https://discord.com/api/v10";

#[derive(Clone)]
pub struct DiscordClient {
    token: String,
    retry_after: Rc<RefCell<Option<Instant>>>,
}

#[allow(dead_code)]
impl DiscordClient {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            retry_after: Rc::new(RefCell::new(None)),
        }
    }

    /// Internal helper to send authorized GET requests and parse JSON
    #[async_recursion::async_recursion(?Send)]
    async fn get_json<T>(&self, endpoint: &str) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let retry_after = *self.retry_after.borrow();
        if let Some(when) = retry_after {
            let now = Instant::now();
            if now < when {
                let wait = when - now;
                wasmtimer::tokio::sleep(wait).await;
            }
        }

        let url = format!("{DISCORD_API}{endpoint}");
        let res: Response = Request::get(&url)
            .header("Authorization", &self.token)
            .header("User-Agent", "DiscordOccasionalMsgFetcher (gloo-net, v0.1)")
            .send()
            .await
            .map_err(|e| anyhow!("Network error: {}", e))?;

        // Check for rate limit (429)
        if res.status() == 429 {
            let retry_after_secs = res
                .headers()
                .get("Retry-After")
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(1.0);

            let duration = std::time::Duration::from_secs_f64(retry_after_secs);
            let next_time = Instant::now() + duration;
            *self.retry_after.borrow_mut() = Some(next_time);

            // Wait, then retry once
            wasmtimer::tokio::sleep(duration).await;
            return self.get_json(endpoint).await;
        }

        // On success, clear cooldown (some minor random jitter could be added here)
        *self.retry_after.borrow_mut() = None;

        if !res.ok() {
            return Err(anyhow!(
                "Discord API error {} at {}",
                res.status(),
                endpoint
            ));
        }

        Ok(res.json::<T>().await?)
    }

    /// Get channel info (returns name + guild_id)
    pub async fn get_channel(&self, channel_id: &str) -> Result<Channel> {
        self.get_json::<Channel>(&format!("/channels/{channel_id}"))
            .await
    }

    /// Get guild info (returns name)
    pub async fn get_guild(&self, guild_id: &str) -> Result<Guild> {
        self.get_json::<Guild>(&format!("/guilds/{guild_id}")).await
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
                // console_log!("{d:?} | {}", d.unix_timestamp());
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

        console_log!("Msg more than 100. Fetching more...");

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

            // console_log!("{limit:?} | {}", messages.len());
            // console_log!("Fetching more {cap} messages");

            let mut newmsg = filter_msg(
                self.get_messages_before(channel_id, &messages.last().unwrap().id, cap)
                    .await?,
            );

            if newmsg.is_empty() {
                break;
            }

            messages.append(&mut newmsg);

            // console_log!("lastmsg: {:?}", messages.last());
            // console_log!("lastdate: {:?}", messages.last().map(|x| x.timestamp()));
            // console_log!(
            //     "range: {:?} -- {:?}",
            //     date_range.start_bound(),
            //     date_range.end_bound()
            // );
            // console_log!(
            //     "inrange: {:?}\n",
            //     date_range.contains(&messages.last().unwrap().timestamp()?)
            // );
            // console_log!("firstmsg: {:?}", messages.first());
            // console_log!("firstdate: {:?}", messages.first().map(|x| x.timestamp()));
            // console_log!(
            //     "range: {:?} -- {:?}",
            //     date_range.start_bound(),
            //     date_range.end_bound()
            // );
            // console_log!(
            //     "inrange: {:?}",
            //     date_range.contains(&messages.first().unwrap().timestamp()?)
            // );
            // console_log!("limit: {limit:?}\n");
        }

        Ok(messages)
    }
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub guild_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
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

const EXCLUDED_PATTERNS: &[&str] = &[
    "cdn.",
    "tenor.",
    "redgifs.",
    "discordapp.",
    "redd.it",
    "media.tumblr.",
];

// TODO: New strat. Make this run each hour or less, store in temp_[yyyyMM]_*
// TODO: Each month, process raw urls and concat.
pub async fn mainfn(env: &worker::Env, sched_diff: i64) -> Result<()> {
    let token = env.secret("DISCORD_TOKEN")?;
    let channels = env.secret("DISCORD_CHANNEL_IDS")?.to_string();
    let channels = channels.split(",").collect::<Vec<_>>();

    let kv = env.kv("VID_PLAYLIST_MANAGER_KV")?;

    let client = DiscordClient::new(token.to_string());

    let currtime = time::UtcDateTime::now();
    let prevtime = currtime.saturating_sub(time::Duration::minutes(sched_diff));

    {
        let timefmt = time::format_description::parse("[hour]:[minute]:[second]")?;
        let timestr = currtime.format(&timefmt)?;
        console_log!("It is currently {timestr}");
    }

    let range = prevtime..currtime;
    console_log!("{range:?}");

    let finder = linkify::LinkFinder::new();
    // finder.url_must_have_scheme(false);
    let excluder = aho_corasick::AhoCorasick::builder()
        .ascii_case_insensitive(true)
        .build(EXCLUDED_PATTERNS)?;
    let mut urls = vec![];

    for ch_id in channels {
        let ch = client.get_channel(ch_id).await?;
        let chname = ch.name;
        let srv_id = ch
            .guild_id
            .expect("Failed to get Server ID (this shouldn't've been possible");
        let srvname = client.get_guild(&srv_id).await?.name;
        // let msg: Vec<Message> = client.get_messages(ch, 1).await?;
        let msg_res = client
            .get_messages_range(ch_id, range.clone(), None)
            .await?;

        if let Some(m) = msg_res.first() {
            let snip = m.content.clone();
            let t_str = m
                .timestamp()?
                .format(&time::format_description::well_known::Rfc3339)?;
            console_log!("First message snippet: [{t_str}] {snip}");
        }

        let msgcount = msg_res.len();
        console_log!("msgcount: {msgcount}");

        let links = msg_res
            .into_iter()
            .map(|x| x.content)
            .flat_map(|x| {
                finder
                    .links(&x)
                    .map(|x| x.as_str().to_string())
                    .collect_vec()
            })
            .collect_vec();

        let filtered_count = links.iter().filter(|x| excluder.is_match(x)).count();

        console_log!(
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

        let mut filtered_links = links
            .into_iter()
            .filter(|x| !excluder.is_match(x))
            .collect_vec();

        urls.append(&mut filtered_links);
    }

    if urls.is_empty() {
        let emfmt = time::format_description::parse("[hour]:[minute]:[second]")?;
        let emtime = prevtime.format(&emfmt)?;
        console_log!("No new links since {emtime}. Skipping sending to KV.");

        return Ok(());
    }

    // let msgs = msgs
    //     .iter()
    //     .sorted_by_key(|x| x.url.clone())
    //     .dedup_by(|a, b| a.url == b.url)
    //     .collect::<Vec<_>>();

    let timefmt = time::format_description::parse("[year]-[month]")?;
    let timestr = prevtime.format(&timefmt)?;

    // let metadata = format!("// METADATA: {{\"created_at\":\"{}\"}}\n\n", currtime);

    let kvname = format!("{timestr}_discord_merged");
    let kvvalue = &urls.join("\n");

    // kv.put(&kvname, kvvalue)
    //     .expect("Failed prepping KV send")
    //     .execute()
    //     .await
    //     .expect("Failed sending KV");

    // crate::cf_utils::kv_append(&kv, &kvname, format!("\n{kvvalue}")).await?;
    {
        console_log!("Getting previous KV to append");
        let prev = kv
            .get(&kvname)
            .text()
            .await
            .expect("Failed prepping KV get")
            .unwrap_or("".into());
        let newval = prev + "\n" + kvvalue.as_ref();

        console_log!("Sending to KV");
        kv.put(&kvname, &newval)
            .expect("Failed prepping KV send")
            .execute()
            .await
            .expect("Failed sending KV");
        console_log!("Done!");
    }

    Ok(())
}
