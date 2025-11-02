use worker::*;

mod discord;

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    tracing_worker::init(&env);

    Response::ok("OK")
}

#[event(scheduled)]
pub async fn cron_event(event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    tracing_worker::init(&env);

    // Do whatever you want here – e.g., call an API, clean up KV, etc.
    console_log!("Running scheduled task: {:?}", event.cron());

    if let Err(e) = discord::mainfn(&env).await {
        console_error!("ERROR: {e}")
    }

    console_log!("Done running schedule task");

    // Ok(())
}
