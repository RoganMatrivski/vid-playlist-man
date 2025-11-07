use std::str::FromStr;

use hypertext::{prelude::*, rsx};
use itertools::Itertools;

use worker::*;

mod cf_utils;
mod discord;
mod fetcher;
mod playlist;

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    tracing_worker::init(&env);

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
        .get_async("/kv", |req, ctx| async move {
            let kv = ctx.env.kv("VID_PLAYLIST_MANAGER_KV")?;
            let list = kv.list().execute().await?;
            let names = list.keys.into_iter().map(|x| x.name).collect_vec();

            let as_html = req
                .headers()
                .get("Accept")?
                .unwrap_or("".into())
                .contains("text/html");

            if !as_html {
                Response::ok(names.join("\n"))
            } else {
                Response::from_html(
                    rsx! {
                    <!DOCTYPE html><html>
                    <head><title>kv list</title></head>
                        <body><div>
                            @for s in &names {
                                <ul>
                                    <li><a href={"/kv/" s}>(s)</a></li>
                                </ul>
                            }
                        </div></body>
                    </html>
                            }
                    .render()
                    .as_inner(),
                )
            }
        })
        .get_async("/kv/new", |_, _| async move {
            Response::from_html(
                rsx! {
                <!DOCTYPE html><html>
                <head><title>new kv</title></head>
                    <body>
                    <form action="/kvnew" method="post">
                        <input id="keyname" name="keyname" /><br/>
                        <textarea id="keyvalue" name="keyvalue" rows="6" cols="40" required></textarea><br/>
                        <button type="submit">Submit</button>
                    </form>
                    </body>
                </html>
                        }
                .render()
                .as_inner(),
            )
        })
        .post_async("/kv/new", |mut req, ctx| async move {
            let body = req.text().await?;
            let form: std::collections::HashMap<String, String> =
                form_urlencoded::parse(body.as_bytes())
                    .into_owned()
                    .collect();

            let kvname = if let Some(kvname) = form.get("keyname") {
                kvname
            } else {
                return Response::error("Missing 'keyname' field", 400);
            };

            let kvvalue = if let Some(kvvalue) = form.get("keyvalue") {
                kvvalue
            } else {
                return Response::error("Missing 'keyvalue' field", 400);
            };

            let kv = ctx.env.kv("VID_PLAYLIST_MANAGER_KV")?;

            kv.put(kvname, kvvalue)?
                .execute()
                .await?;

            Response::ok("KV set")
        })

        .get_async("/kv/:keyname", |req, ctx| async move {
            let kvname = if let Some(n) = ctx.param("keyname") {
                n
            } else {
                return Response::error("KV not found", 404);
            };

            let as_html = req
                .headers()
                .get("Accept")?
                .unwrap_or("".into())
                .contains("text/html");

            let kv = ctx.env.kv("VID_PLAYLIST_MANAGER_KV")?;

            match kv.get(kvname).text().await? {
                Some(s) => {
                    if !as_html {
                        Response::ok(s)
                    } else {
                        Response::from_html(
                            rsx! {
                                <!DOCTYPE html><html>
                                <head><title>(kvname)</title></head>
                                    <body><p style="white-space: pre-wrap;">(s)</p></body>
                                </html>
                            }
                            .render()
                            .as_inner(),
                        )
                    }
                }
                None => Response::error("KV Empty", 404),
            }
        })
        // .get("*", |_, _| Response::error("Not found", 404))
        .run(req, env)
        .await
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
