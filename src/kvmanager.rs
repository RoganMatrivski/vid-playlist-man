use hypertext::{Renderable, prelude::*, rsx};
use itertools::Itertools;
use worker::{Request, Response, Result, RouteContext};

pub async fn kv_list(req: Request, ctx: RouteContext<()>) -> Result<Response> {
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
            crate::htmlgen::gen_linkpage(
                names
                    .into_iter()
                    .map(|x| crate::htmlgen::Nav::new(format!("kv/{x}"), &x))
                    .collect_vec(),
            )
            .expect("Failed render template"),
        )
    }
}

pub async fn kv_get(req: Request, ctx: RouteContext<()>) -> Result<Response> {
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
                    crate::htmlgen::gen_plaintext(s.trim()).expect("Failed render template"),
                )
            }
        }
        None => Response::error("KV Empty", 404),
    }
}

pub async fn kv_new_get(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
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
}

pub async fn kv_new_post(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let body = req.text().await?;
    let form: std::collections::HashMap<String, String> = form_urlencoded::parse(body.as_bytes())
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

    kv.put(kvname, kvvalue)?.execute().await?;

    Response::ok("KV set")
}
