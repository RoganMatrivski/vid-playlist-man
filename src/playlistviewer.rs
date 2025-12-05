use std::collections::HashMap;

use itertools::Itertools;
use worker::{Request, Response, Result, RouteContext};

pub async fn playlist_list(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let kv = ctx.env.kv("VID_PLAYLIST_MANAGER_KV")?;

    let as_html = req
        .headers()
        .get("Accept")?
        .unwrap_or("".into())
        .contains("text/html");

    let tomlstr = kv.get("config_playlist").text().await?.unwrap_or("".into());
    let tomlval = toml::from_str::<toml::Value>(&tomlstr).expect("Failed to parse toml");

    let src = tomlval
        .get("playlist_sources")
        .and_then(|x| x.as_array())
        .expect("No sources found");

    let names = src
        .iter()
        .map(|x| {
            x.get("name")
                .map(|x| x.as_str().expect("`name` value is not a string"))
                .expect("`name` field missing")
        })
        .collect::<Vec<_>>();

    if as_html {
        Response::from_html(
            crate::htmlgen::gen_linkpage(
                names
                    .into_iter()
                    .map(|x| crate::htmlgen::Nav::new(format!("playlist/{x}"), x))
                    .collect_vec(),
            )
            .expect("Failed render template"),
        )
    } else {
        Response::ok(names.join("\n"))
    }
}

pub async fn playlist_single(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let kv = ctx.env.kv("VID_PLAYLIST_MANAGER_KV")?;

    let as_html = req
        .headers()
        .get("Accept")?
        .unwrap_or("".into())
        .contains("text/html");

    let reversed = {
        let url = req.url()?;
        url.query_pairs().any(|(k, _)| k == "reversed")
    };

    let tomlstr = kv.get("config_playlist").text().await?.unwrap_or("".into());
    let tomlval = toml::from_str::<toml::Value>(&tomlstr).expect("Failed to parse toml");

    let src = tomlval
        .get("playlist_sources")
        .and_then(|x| x.as_array())
        .expect("No sources found");

    let nameurlpair = src
        .iter()
        .map(|x| {
            (
                x.get("name")
                    .map(|x| x.as_str().expect("`name` value is not a string"))
                    .expect("`name` field missing"),
                x.get("url")
                    .map(|x| x.as_str().expect("`url` value is not a string"))
                    .expect("`url` field missing"),
            )
        })
        .collect::<HashMap<_, _>>();

    let playlistname = if let Some(n) = ctx.param("name") {
        n
    } else {
        return Response::error("Playlist not found", 404);
    };

    let url = nameurlpair
        .get(playlistname.as_str())
        .unwrap_or_else(|| panic!("Cannot get url for name {playlistname}"));

    let playlist_urls = crate::playlist::PlaylistFetcher::new()
        .get(url)
        .await
        .unwrap_or_else(|_| panic!("Failed getting urls for {playlistname}"));

    let mut playlist_urls: Vec<&str> = playlist_urls.lines().map(str::trim).collect();

    if reversed {
        playlist_urls.reverse();
    }

    let playlist_urls = playlist_urls.join("\n");

    if as_html {
        Response::from_html(
            crate::htmlgen::gen_plaintext(playlist_urls).expect("Failed render template"),
        )
    } else {
        Response::ok(playlist_urls)
    }
}
