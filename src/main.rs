use feed_rs::parser;
use m3u;
use podcast_search::{search, Kind, PodcastSearchError};
use reqwest::{self, Url};
use configparser::ini::Ini;
use log::{info,warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

use std::time::Duration;
use std::str;
use std::borrow::Cow;


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    //load config
    let mut config = Ini::new();
    // config.load("./config.ini")?;
    let host = config.get("espuino", "host").unwrap_or(String::from("espuino.local"));
    let proxy_url = config.get("espuino", "host");
    let mut directory = config.get("espuino", "path").unwrap_or(String::from("/podcasts/"));
    if !directory.ends_with("/") {directory.push('/')};
    let podcast_path = Path::new(&directory);

    // process podcast config
    let mut playlists = HashMap::new();
    for section in config.get_map_ref().keys().filter(|s| s.starts_with("podcast.")) {
        let name = section.strip_prefix("podcast.").unwrap();
        println!("Processing podcast \"{}\"", name);
        let url = match config.get(section, "url") {
            None => {
                println!("Section {} is missing a url entry", section);
                continue},
            Some(url) => match Url::parse(&url) {
                Err(err) => {
                    println!("Error parsing url for \"{}\": {}", section, err);
                    continue},
                Ok(url) => url
            }
        };

        let truncate = config.getuint(section, "num").unwrap_or(None).map(|x| x as usize);
        let reverse = config.getboolcoerce(&section, "reverse").unwrap_or(Some(false)).unwrap_or(false);
        let to_file = config.get(section, "file");

        let playlist = match process_rss(url.clone(), truncate, reverse).await {
            Err(err) => {
                println!("Error processing content from {}: {}", url.as_str(), err);
                continue},
            Ok(pls) => pls
        };

        if let Some(fname) = to_file {
            println!("Writing playlist file: {}", fname);
            let mut file = std::fs::File::create(fname)?;
            let mut writer = m3u::Writer::new(&mut file);
            for entry in playlist.iter().cloned() {
                writer.write_entry(&entry).unwrap();
            }
        }

        println!("Finished processing podcast \"{}\" ({} elements)", name, playlist.len());
        playlists.insert(name, playlist);
    }
    println!("Finished processing podcast feeds.");


    // prepare upload to ESPuino
    let mut api_url = Url::parse(&format!("http://{}", host))?;
    api_url.set_path("/explorer");

    // build reqwest client
    let client = match proxy_url {
        Some(proxy) => reqwest::Client::builder()
                                    .proxy(reqwest::Proxy::http(proxy)?) // useful for debugging
                                    .build()?,
        None => reqwest::Client::new()
    };
        .send()
        .await?;


    for (name, playlist) in playlists {
        // let path = podcast_path.with_file_name(name).with_extension("m3u");
        let path = podcast_path.join(name).with_extension("m3u");
        println!("path: {}", path.display());
        write_playlist_to_server(&client, &api_url
            , &playlist
            , path.to_str().unwrap()).await?;
    }

    // Test API call to /explorer
    let params = [("path", "/")];
    let mut url = api_url.clone();
    url.set_path("/explorer");
    let response = client.get(url)
        .query(&params)
        .send()
        .await?;
    let dirtree: Vec<TreeEntry> = response.json().await?;
    println!("Dirtree:\n{:?}", dirtree);

    let params = [("path", "/podcasts")];
    let mut url = api_url.clone();
    url.set_path("/explorer");
    let response = client.get(url)
        .query(&params)
        .send()
        .await?;
    let dirtree: Vec<TreeEntry> = response.json().await?;
    println!("Dirtree:\n{:?}", dirtree);



    Ok(())
}

#[derive(Deserialize, Debug)]
struct TreeEntry {
    name: String,
    dir: Option<bool>
}

#[derive(Debug)]
struct Podcast {
    name: String,
    url: Url,
    truncate: Option<usize>,
    reverse: bool,
}

async fn upload_file<T>(client: &reqwest::Client, base_url: &Url, path: &str, bytes: T) -> reqwest::Result<()>
where T: Into<Cow<'static, [u8]>>,
{
    let mut url = base_url.clone();
    url.set_path("/explorer");
    // prepare multipart form
    let part = reqwest::multipart::Part::bytes(bytes);
    let file = reqwest::multipart::Form::new()
        .part("file", part);

    let params = [("path", path)]; // prepare query parameters

    client.post(url)
        .query(&params)
        .multipart(file) 
        .send()
        .await?;
    Ok(())
}

async fn write_playlist_to_server(client: &reqwest::Client, base_url: &Url, playlist: &Vec<m3u::Entry>, path: &str) -> reqwest::Result<()> {
    // first write m3u file to in-memory buffer
    let buf: Vec<u8> = Vec::with_capacity(1024*1024);
    let mut cursor = std::io::Cursor::new(buf); // 1 MB buffer
    {
        // borrow cursor and give it to m3u writer
        let mut writer = m3u::Writer::new(&mut cursor);
        for entry in playlist {
            writer.write_entry(entry).unwrap();
        }
        // writer goes out of scope, cursor can be used again.
    }

    // second, send file from buffer
    upload_file(client, base_url, path, cursor.into_inner()).await?;
    Ok(())
}

async fn search_podcast(terms: &str) -> Result<(), PodcastSearchError> {
    let search_results_future = search(terms);
    println!("Searching for \"{}\"", terms);

    // execute:
    let search_results = search_results_future.await?;
    println!("Found {} results:", search_results.result_count);
    for (i, res) in search_results
        .results
        .iter()
        .filter(|&r| r.kind == Some(Kind::Podcast))
        .enumerate()
    {
        println!(
            "{}:\t{}\n\t{}",
            i + 1,
            res.collection_name.clone().unwrap(),
            res.feed_url.clone().unwrap()
        );
    }
    Ok(())
}

async fn process_rss(url: Url, truncate: Option<usize>, reverse: bool) -> reqwest::Result<Vec<m3u::Entry>> {
    let body = reqwest::get(url).await?.text().await?;
    let rss = parser::parse(body.as_bytes()).unwrap();

    let mut episodes: Vec<_> = rss
        .entries
        .iter()
        .map(|e| {
            // println!("Title: {:?}", e.title);
            // println!("Content: {:?}", e.links);
            let mut url = e
                .media
                .iter()
                .next()
                .unwrap()
                .content
                .iter()
                .next()
                .unwrap()
                .url
                .clone()
                .unwrap(); // just stupidly get first media enclosure
            if url.scheme() == "https" { // try to change https to http due to ESP limitations
                url.set_scheme("http").unwrap()
            }
            url.set_query(None); // try to remove url parameters to
                                 // println!("Media: {:?}", url.to_string());
                                 // e.title.clone().unwrap()
            // TODO: extend m3u url entry with title
            url
        })
        .map(|url| m3u::url_entry(url.as_str()).unwrap())
        .collect();

    if reverse {episodes.reverse()};
    if let Some(n) = truncate {episodes.truncate(n as usize)};

    Ok(episodes)
}
