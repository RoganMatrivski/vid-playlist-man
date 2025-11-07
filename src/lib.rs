use std::str::FromStr;

use worker::*;

mod cf_utils;
mod discord;
mod fetcher;
mod playlist;

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    tracing_worker::init(&env);

    let path = req.path();

    let url = req.url()?;
    let mut query_pairs = url.query_pairs();

    match path.as_str() {
        "/" => Response::ok(""),
        "/get" => {
            let url = query_pairs
                .find(|(key, _)| key == "url")
                .map(|(_, value)| value.to_string());

            if let Some(u) = url {
                match playlist::mainfn_single(&u).await {
                    Ok(x) => Response::ok(x),
                    Err(e) => Response::error(format!("GET request failed. {e}"), 500),
                }
            } else {
                Response::error("url key empty", 400)
            }
        }
        _ => Response::error("Not found", 404),
    }
}

#[event(scheduled)]
pub async fn cron_event(event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    tracing_worker::init(&env);

    // Do whatever you want here – e.g., call an API, clean up KV, etc.
    console_log!("Running scheduled task: {:?}", event.cron());

    let t = event.schedule();
    let t_chrono = chrono::DateTime::from_timestamp_millis(t as i64).unwrap();
    let cron = croner::Cron::from_str(&event.cron()).unwrap();
    let two_cron = cron.iter_before(t_chrono).take(2).collect::<Vec<_>>();
    let crondiff = (two_cron[0] - two_cron[1]).num_minutes();
    console_log!("cron description: {}", cron.describe());
    console_log!("{crondiff} | {t_chrono} | {}", t as i64);

    if let Err(e) = discord::mainfn(&env, crondiff).await {
        console_error!("ERROR: {e}")
    }

    if let Err(e) = playlist::mainfn(&env).await {
        console_error!("ERROR: {e}")
    }

    console_log!("Done running schedule task");

    // Ok(())
}
