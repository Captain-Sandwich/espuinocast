use feed_rs::parser;
use m3u;
use podcast_search::{search, Kind, PodcastSearchError};
use reqwest;
use std::io::{self, Write};
use configparser::ini::Ini;
use suppaftp::FtpStream;
use log::{info,warn};

use std::time::Duration;
use std::str;


#[tokio::main]
async fn main() -> () {
    println!("Hello, world!");
    let mut config = Ini::new();
    config.load("./config.ini").unwrap();
    println!("config: {:?}", config);

    let res = search_podcast("the adventure zone");
    let _ = res.await;
    let playlist = process_rss("https://feeds.simplecast.com/cYQVc__c").await.unwrap();
    write_playlist_to_tempfile(&playlist, "./adventurezone.m3u")?;

    let mut ftp_stream = FtpStream::connect("espuino.local:21").unwrap();
    ftp_stream.login("esp32", "esp32").unwrap();
    let mut ftp_stream = ftp_stream.active_mode(Duration::new(2,0)); // needs to set active mode. Passive mode is not supported in the esp library
    let _ = ftp_stream.cwd("/SD-Card/podcasts").unwrap();
    let pwd = ftp_stream.pwd().unwrap();
    println!("pwd: {}", pwd);
    let fnames = ftp_stream.feat().unwrap();
    println!("Fnames: {:?}", fnames);
    let cursor = ftp_stream.retr_as_buffer("config.ini").unwrap();
    println!("File: {:?}", str::from_utf8(&cursor.into_inner()).unwrap());
}


// async fn write_playlist_to_ftp(ftp: &mut FtpStream, playlist: &Vec<m3u::Entry>, fname: &str, path: &str) -> () {
//     let mut cursor = std::io::Cursor::new(vec![0; 1024*1024]); // 1 MB buffer
//     let mut writer = m3u::Writer::new(cursor);
//     for entry in playlist {
//         writer.write_entry(entry).unwrap();
//     }
//     cursor.set_position(0);
//     // ftp.

//     unimplemented!()
// }

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

fn write_playlist_to_tempfile(pls: &Vec<m3u::Entry>, fname: &str) -> io::Result<()> {
    let mut file = std::fs::File::create(fname).unwrap();
    let mut writer = m3u::Writer::new(&mut file);
    for entry in pls {
        writer.write_entry(entry).unwrap();
    }
    Ok(())
}

async fn process_rss(url: &str) -> reqwest::Result<Vec<m3u::Entry>> {
    let body = reqwest::get(url).await?.text().await?;
    let rss = parser::parse(body.as_bytes()).unwrap();
    println!("Processing {:?}", rss.title.unwrap().content);
    let episodes: Vec<_> = rss
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
    // println!("{:?}", episodes);
    Ok(episodes)
}
