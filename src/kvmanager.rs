use headers_accept::Accept;
use hypertext::{Renderable, prelude::*, rsx};
use itertools::Itertools;
use mediatype::{
    MediaType,
    names::{HTML, TEXT},
};
use std::str::FromStr;
use worker::{Request, Response, Result, RouteContext};

pub async fn kv_list(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let kv = ctx.env.kv("VID_PLAYLIST_MANAGER_KV")?;
    let list = kv.list().execute().await?;
    let names = list.keys.into_iter().map(|x| x.name).collect_vec();

    let accept_header = req.headers().get("Accept")?.unwrap_or("text/plain".into());
    let accept =
        Accept::from_str(&accept_header).unwrap_or(Accept::from_str("text/plain").unwrap());

    let html_media = MediaType::new(TEXT, HTML);
    let as_html = accept.negotiate(&[html_media]).is_some();

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

    let accept_header = req.headers().get("Accept")?.unwrap_or("text/plain".into());
    let accept =
        Accept::from_str(&accept_header).unwrap_or(Accept::from_str("text/plain").unwrap());

    let html_media = MediaType::new(TEXT, HTML);
    let as_html = accept.negotiate(&[html_media]).is_some();

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
            <form action="/kv/new" method="post">
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

    // Invalidate the cache for the viewable GET path
    let mut cache_url = req.url()?;
    cache_url.set_path(&format!("/kv/{}", kvname));
    let cache = worker::Cache::default();
    let _ = cache.delete(cache_url.as_str(), false).await;

    Response::ok("KV set")
}

pub async fn kv_edit_get(_req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let kvname = if let Some(n) = ctx.param("id") {
        n
    } else {
        return Response::error("KV not found", 404);
    };

    let kv = ctx.env.kv("VID_PLAYLIST_MANAGER_KV")?;

    let kvvalue = match kv.get(kvname).text().await? {
        Some(s) => s,
        None => return Response::error("KV not found", 404),
    };

    let edit_action = format!("/kv/{kvname}/edit");
    let delete_action = format!("/kv/{kvname}/delete");

    Response::from_html(
        rsx! {
        <!DOCTYPE html><html>
        <head><title>edit kv</title></head>
            <body>
            <form action={edit_action} method="post">
                <input id="keyname" name="keyname" value={kvname} readonly /><br/>
                <textarea id="keyvalue" name="keyvalue" rows="6" cols="40" required>(kvvalue)</textarea><br/>
                <button type="submit">Save</button>
            </form>
            <form action={delete_action} method="post">
                <button type="submit">Delete</button>
            </form>
            </body>
        </html>
                }
        .render()
        .as_inner(),
    )
}

pub async fn kv_edit_post(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let kvname = if let Some(n) = ctx.param("id") {
        n
    } else {
        return Response::error("KV not found", 404);
    };

    let body = req.text().await?;
    let form: std::collections::HashMap<String, String> = form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();

    let kvvalue = if let Some(kvvalue) = form.get("keyvalue") {
        kvvalue
    } else {
        return Response::error("Missing 'keyvalue' field", 400);
    };

    let kv = ctx.env.kv("VID_PLAYLIST_MANAGER_KV")?;

    // Ensure the key already exists before allowing an "edit" to create one.
    if kv.get(kvname).text().await?.is_none() {
        return Response::error("KV not found", 404);
    }

    kv.put(kvname, kvvalue)?.execute().await?;

    // Invalidate the cache for the viewable GET path
    let mut cache_url = req.url()?;
    cache_url.set_path(&format!("/kv/{}", kvname));
    let cache = worker::Cache::default();
    let _ = cache.delete(cache_url.as_str(), false).await;

    // Also invalidate the edit page itself, in case it's cached
    let mut edit_url = req.url()?;
    edit_url.set_path(&format!("/kv/{}/edit", kvname));
    let _ = cache.delete(edit_url.as_str(), false).await;

    Response::ok("KV updated")
}

pub async fn kv_delete_get(_req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let kvname = if let Some(n) = ctx.param("id") {
        n
    } else {
        return Response::error("KV not found", 404);
    };

    let kv = ctx.env.kv("VID_PLAYLIST_MANAGER_KV")?;

    if kv.get(kvname).text().await?.is_none() {
        return Response::error("KV not found", 404);
    }

    let delete_action = format!("/kv/{kvname}/delete");
    let prompt = format!("Delete key {kvname}? This cannot be undone.");

    Response::from_html(
        rsx! {
        <!DOCTYPE html><html>
        <head><title>delete kv</title></head>
            <body>
            <p>(prompt)</p>
            <form action={delete_action} method="post">
                <button type="submit">Delete</button>
            </form>
            </body>
        </html>
                }
        .render()
        .as_inner(),
    )
}

pub async fn kv_delete_post(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let kvname = if let Some(n) = ctx.param("id") {
        n
    } else {
        return Response::error("KV not found", 404);
    };

    let kv = ctx.env.kv("VID_PLAYLIST_MANAGER_KV")?;

    if kv.get(kvname).text().await?.is_none() {
        return Response::error("KV not found", 404);
    }

    kv.delete(kvname).await?;

    // Invalidate the cache for the viewable GET path
    let mut cache_url = req.url()?;
    cache_url.set_path(&format!("/kv/{}", kvname));

    let cache = worker::Cache::default();
    let _ = cache.delete(cache_url.as_str(), false).await;

    Response::ok("KV deleted")
}
