use feed_rs::parser;
use m3u;
use reqwest::{self, Url};
use configparser::ini::Ini;
// use log::{info,warn};
use serde::Deserialize;
use clap::Parser;
use std::collections::HashMap;
use std::path::Path;
use std::borrow::Cow;

/// ESPuino podcast helper
/// Converts podcast feeds to m3u playlists and writes them to an ESPuino.
#[derive(Parser, Debug)]
#[command(version, about, long_about=None)]
struct Args {
    /// Configuration file
    #[arg(short, long, default_value_t = String::from("./config.ini"), value_name = "FILE")]
    config: String,

    /// ESPuino host, name or IP, overrides configuration 
    #[arg(short, long)]
    address:Option<String>,

   /// forces playlists to files for all podcasts
   #[arg(short, long)]
   force_write: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    //parse arguments
    let args = Args::parse();

    //load config, override with cli args if necessary
    let mut config = Ini::new();
    config.load(args.config)?;
    let host = match args.address {
        Some(x) => x,
        None => config.get("espuino", "host").unwrap_or(String::from("espuino.local"))
    };
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
                eprintln!("Section {} is missing a url entry", section);
                continue},
            Some(url) => match Url::parse(&url) {
                Err(err) => {
                    eprintln!("Error parsing url for \"{}\": {}", section, err);
                    continue},
                Ok(url) => url
            }
        };

        let truncate = config.getuint(section, "num").unwrap_or(None).map(|x| x as usize);
        let reverse = config.getboolcoerce(&section, "reverse").unwrap_or(Some(false)).unwrap_or(false);
        let to_file = 
            if args.force_write
                { Some(format!("./{}.m3u", name)) } else
                {config.get(section, "file")};

        // get and process podcast
        let playlist = match process_rss(url.clone(), truncate, reverse).await {
            Err(err) => {
                eprintln!("Error processing content from {}: {}", url.as_str(), err);
                continue},
            Ok(pls) => pls
        };


        if let Some(fname) = to_file { write_m3u(&fname, &playlist)? }

        println!("Finished processing podcast \"{}\" ({} elements)", name, playlist.len());
        playlists.insert(name, playlist);
    }
    println!("Finished processing podcast feeds.");


    // prepare upload to ESPuino
    let mut api_url = Url::parse(&format!("http://{}", host))?;
    api_url.set_path("/explorer");

    println!("Starting uploads to ESPuino");
    // build reqwest client
    let client = match proxy_url {
        Some(proxy) => reqwest::Client::builder()
                                    .proxy(reqwest::Proxy::http(proxy)?) // useful for debugging
                                    .build()?,
        None => reqwest::Client::new()
    };

    // First check, if necessary directories exist
    let parent = match podcast_path.parent() {
        Some(p) => p,
        None => Path::new("/")
    };
    let basename = podcast_path.components().last().unwrap().as_os_str().to_str().unwrap();
    let params = [("path", parent)];
    let response = client.get(api_url.clone())
        .query(&params)
        .send()
        .await?;
    let dirtree: Vec<TreeEntry> = response.json().await?;
    let directory_exists = dirtree.iter().any(|item| (item.name == basename) && (item.dir == Some(true)));
    if !directory_exists {
        println!("Directory does not exist. Creating {:?}", podcast_path.as_os_str());
        let params = [("path", podcast_path)];
        client.put(api_url.clone())
            .query(&params)
            .send()
            .await?;
    }
    for (name, playlist) in playlists {
        // let path = podcast_path.with_file_name(name).with_extension("m3u");
        let path = podcast_path.join(name).with_extension("m3u");
        write_playlist_to_server(&client, &api_url
            , &playlist
            , path.to_str().unwrap()).await?;
    }

    // list podcast directory
    let params = [("path", podcast_path)];
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

    let path = Path::new(path);
    let directory = path.parent().unwrap();
    let fname = path.file_name().unwrap().to_owned().into_string().unwrap();

    // prepare multipart form
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(fname)
        .mime_str("application/octet-stream")?;

    let form = reqwest::multipart::Form::new()
        .part("file", part);

    let params = [("path", directory)]; // prepare query parameters

    client.post(url)
        .query(&params)
        .multipart(form)
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
        
fn write_m3u(path: &str, playlist: &Vec<m3u::Entry>) -> std::io::Result<()> {
            let mut file = std::fs::File::create(path)?;
            let mut writer = m3u::Writer::new(&mut file);
            for entry in playlist.iter().cloned() {
                writer.write_entry(&entry).unwrap();
            }
            Ok(())
}