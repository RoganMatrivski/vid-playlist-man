use std::str::FromStr;

use worker::*;

mod discord;
mod fetcher;
mod htmlgen;
mod kvcache;
mod playlist;

mod kvmanager;
mod playlistviewer;

fn get_envvar(env: &Env) -> worker::wasm_bindgen::JsValue {
    env.var("ENV")
        .unwrap_or(worker::Var::from(worker::wasm_bindgen::JsValue::from_str(
            "",
        )))
        .as_ref()
        .clone()
}

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    tracing_worker::init_tracing(if env.var("ENV").unwrap().as_ref() == "development" {
        tracing::Level::TRACE
    } else {
        tracing::Level::INFO
    });

    Router::new()
        .get("/", |_, _| Response::error("", 404))
        .get_async("/get", |req, _ctx| async move {
            let url = req.url()?;
            let mut query_pairs = url.query_pairs();

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
        })
        .get_async("/kv", kvmanager::kv_list)
        .get_async("/kv/new", kvmanager::kv_new_get)
        .post_async("/kv/new", kvmanager::kv_new_post)
        .get_async("/kv/:keyname", kvmanager::kv_get)
        .get_async("/playlist", playlistviewer::playlist_list)
        .get_async("/playlist/:name", playlistviewer::playlist_single)
        .get("/test", |_, _| {
            tracing::trace!("Testing trace");
            tracing::debug!("Testing debug");
            tracing::info!("Testing info");
            tracing::warn!("Testing warn");
            tracing::error!("Testing error");

            Response::ok("")
        })
        // .get("*", |_, _| Response::error("Not found", 404))
        .run(req, env)
        .await
}

#[event(scheduled)]
pub async fn cron_event(event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    tracing_worker::init_tracing(if get_envvar(&env) == "production" {
        tracing::Level::INFO
    } else {
        tracing::Level::TRACE
    });

    // Do whatever you want here – e.g., call an API, clean up KV, etc.
    tracing::info!("Running scheduled task: {:?}", event.cron());

    let t = event.schedule();
    let t_chrono = chrono::DateTime::from_timestamp_millis(t as i64).unwrap();
    let cron = croner::Cron::from_str(&event.cron()).unwrap();
    let two_cron = cron.iter_before(t_chrono).take(2).collect::<Vec<_>>();
    let crondiff = (two_cron[0] - two_cron[1]).num_minutes();
    tracing::debug!("cron description: {}", cron.describe());
    tracing::debug!("{crondiff} | {t_chrono} | {}", t as i64);

    if let Err(e) = discord::mainfn(&env, crondiff).await {
        tracing::error!("ERROR: {e}")
    }

    tracing::info!("Done running schedule task");

    // Ok(())
}
