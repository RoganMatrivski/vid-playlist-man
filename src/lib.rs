use std::str::FromStr;

use worker::*;

mod cf_utils;
mod discord;

#[event(fetch)]
pub async fn main(_req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    tracing_worker::init(&env);

    Response::ok("")
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
    console_log!("{crondiff} | {t_chrono} | {}", t as i64);

    if let Err(e) = discord::mainfn(&env, crondiff).await {
        console_error!("ERROR: {e}")
    }

    console_log!("Done running schedule task");

    // Ok(())
}
