use actix_files::NamedFile;
use actix_web::{
    error::{ErrorInternalServerError, ErrorNotFound},
    get,
    http::header::ContentDisposition,
    http::header::DispositionParam,
    http::header::DispositionType,
    web::{self, Json},
    App, Error, HttpServer,
};
use db::*;
use json::{object, JsonValue};
use lazy_static::lazy_static;
use rand::{distributions::WeightedIndex, prelude::Distribution, thread_rng};
use serde::Serialize;
use stable_eyre::eyre::{eyre, Context};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use update_manager::adb_update;
use walkdir::WalkDir;

mod db;
mod update_manager;

type Mylist = Arc<Mutex<Vec<u16>>>;

lazy_static! {
    static ref LAST_SONGS: Mylist = Arc::new(Mutex::new(Vec::new()));
}

const GL_PORT: i16 = 81i16;
const GL_RATING_BASE: u16 = 2u16;
const GL_DEFAULT_RATING_SCALE: f32 = 2.5f32;
const GL_DEBUG_SIZE: bool = false;
const GL_REPLAY_PROTECTION: usize = 15;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .service(net_update_files)
            .service(net_songlist)
            .service(net_get_random_id)
            .service(net_get_random_id_with_scale)
            .service(net_song_random)
            .service(net_song_by_id)
            .service(net_song_upvote_by_id)
            .service(net_song_downvote_by_id)
            .service(net_songdata_by_id)
            .service(net_songdata_pretty_by_id)
            .service(net_404)
            .service(net_ping)
    })
    .bind(format!(":{GL_PORT}"))?
    .bind(format!("localhost:{GL_PORT}"))?
    .bind(format!("0.0.0.0:{GL_PORT}"))?
    .run()
    .await
}

#[get("/ping")]
async fn net_ping() -> Result<String, Error> {
    Ok("pong".to_string())
}

#[get("/update")]
async fn net_update_files() -> Result<String, Error> {
    adb_update()?;

    let mut result: String = "".to_string();
    let mut size: u64 = 0;
    let mut values = String::with_capacity(10000 * 300);

    let files = WalkDir::new("E:\\Musik\\");
    files
        .into_iter()
        .filter_map(Result::ok)
        .filter(|f| {
            f.path()
                .to_str()
                .unwrap_or("")
                .to_lowercase()
                .ends_with(".mp3")
        })
        .for_each(|entry| {
            if GL_DEBUG_SIZE {
                size += match entry.metadata() {
                    Ok(ok) => ok.len(),
                    Err(_) => 0,
                };
            }
            let path = entry.path().display().to_string();
            let filename = entry.file_name().to_string_lossy();

            let songname;
            let artist;
            let album;

            if let Ok(tags) = audiotags::Tag::new()
                .with_tag_type(audiotags::TagType::Id3v2)
                .read_from_path(&path)
            {
                songname = tags.title().unwrap_or_default().to_owned();
                artist = tags.artist().unwrap_or_default().to_owned();
                album = tags.album_title().unwrap_or_default().to_owned();
            } else {
                songname = "".to_string();
                artist = "".to_string();
                album = "".to_string();
            };

            let seconds = get_songlength_secs(&path);
            let length = format_songlength(seconds);
            let rating = GL_RATING_BASE;
            let vote = 0;
            let deleted = 0;

            result = format!("{result}{path}\n");
            // Statement-Values aufbauen
            values = format!(
                "{}(\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\", {}, {}, {}, {}),",
                values,
                path.replace('\"', ""),
                filename.replace('\"', ""),
                songname.replace('\"', ""),
                artist.replace('\"', ""),
                album.replace('\"', ""),
                length.replace('\"', ""),
                seconds,
                rating,
                vote,
                deleted
            );
        });
    if GL_DEBUG_SIZE {
        println!("{size}");
    }

    if !values.is_empty() {
        adb_execute("UPDATE songs set deleted = 1")?;

        let statement = &format!(
            "INSERT INTO songs (path, filename, songname, artist, album, length, seconds, rating, vote, deleted)
            VALUES {}
            ON CONFLICT (path) DO UPDATE SET
            songname=excluded.songname,
            artist=excluded.artist,
            album=excluded.album,
            length=excluded.length,
            seconds=excluded.seconds,
            deleted=excluded.deleted",
            &values[..values.len() - 1]
        );

        std::fs::remove_file("log.txt").unwrap_or_default();
        std::fs::write("log.txt", statement).unwrap_or_default();

        adb_execute(statement)?;
    }

    Ok(result)
}

#[get("/random_id/{scale}")]
async fn net_get_random_id_with_scale(scale: web::Path<f32>) -> Result<String, Error> {
    adb_update()?;
    get_weighted_random_id(*scale)
}

#[get("/random_id")]
async fn net_get_random_id() -> Result<String, Error> {
    adb_update()?;
    get_weighted_random_id(GL_DEFAULT_RATING_SCALE)
}

#[derive(Serialize)]
struct Song {
    id: i32,
    path: String,
    filename: String,
    songname: String,
    artist: String,
    album: String,
    length: String,
    seconds: i32,
    rating: i32,
    vote: i32,
}

#[get("/songs")]
async fn net_songlist() -> Result<web::Json<Vec<Song>>, Error> {
    adb_update()?;
    let sql = "select * from songs";

    let c = adb_con()?;
    let mut stmt = errconv(c.prepare(sql).wrap_err("prepare"))?;
    let vec = stmt.query_map([], |row| {
        Ok(Song {
            id: row.get::<_, i32>(0)?,
            path: row.get::<_, String>(1)?,
            filename: row.get::<_, String>(2)?,
            songname: row.get::<_, String>(3)?,
            artist: row.get::<_, String>(4)?,
            album: row.get::<_, String>(5)?,
            length: row.get::<_, String>(6)?,
            seconds: row.get::<_, i32>(7)?,
            rating: row.get::<_, i32>(8)?,
            vote: row.get::<_, i32>(9)?,
        })
    });

    let vec = errconv(vec.wrap_err("convert MappedRows"))?;
    let vec = errconv(
        vec.into_iter()
            .collect::<Result<Vec<Song>, _>>()
            .wrap_err("collect Songs"),
    )?;

    Ok(Json(vec))
}

#[get("/songs/{id}")]
async fn net_song_by_id(id: web::Path<u32>) -> Result<NamedFile, Error> {
    adb_update()?;
    let id = id.into_inner();
    increase_times_played(id)?;
    let val = get_songpath_by_id(id)?;
    get_file_by_name(&val)
}

#[get("/songs/random")]
async fn net_song_random() -> Result<NamedFile, Error> {
    adb_update()?;
    let id = get_weighted_random_id(GL_DEFAULT_RATING_SCALE)?;
    let path = get_songpath_by_id(id.parse::<u32>().unwrap_or_default())?;
    get_file_by_name(&path)
}

#[get("/songdata/{id}")]
async fn net_songdata_by_id(id: web::Path<u32>) -> Result<String, Error> {
    adb_update()?;
    let id = id.into_inner();
    let song = get_songdata_json(id)?;
    Ok(song.dump())
}

#[get("/songdata_pretty/{id}")]
async fn net_songdata_pretty_by_id(id: web::Path<u32>) -> Result<String, Error> {
    adb_update()?;
    let id = id.into_inner();
    let song = get_songdata_json(id)?;
    Ok(format!("{song:#}"))
}

fn get_songdata_json(id: u32) -> Result<JsonValue, Error> {
    adb_select(
        "select * from songs where id = ?",
        [id],
        |row| -> Result<json::JsonValue, rusqlite::Error> {
            Ok(object! {
                id: row.get(0).unwrap_or(0),
                path: row.get(1).unwrap_or_else(|_a| "".to_string()),
                filename: row.get(2).unwrap_or_else(|_a| "".to_string()),
                songname: row.get(3).unwrap_or_else(|_a| "".to_string()),
                artist: row.get(4).unwrap_or_else(|_a| "".to_string()),
                album: row.get(5).unwrap_or_else(|_a| "".to_string()),
                length: row.get(6).unwrap_or_else(|_a| "".to_string()),
                seconds: row.get(7).unwrap_or(0),
                rating: row.get(8).unwrap_or(0),
                vote: row.get(9).unwrap_or(0),
                times_played: row.get(11).unwrap_or(0)
            })
        },
    )
}

#[get("/upvote/{id}")]
async fn net_song_upvote_by_id(id: web::Path<u32>) -> Result<String, Error> {
    adb_update()?;
    let id = id.into_inner();
    let mut val = adb_uint32_read(&format!("SELECT rating FROM songs WHERE id = {id}"))?;
    if val < 7 {
        val += 1;
        let i = &format!("Update songs set rating = {val} where id = {id}");
        adb_execute(i)?;
    }
    Ok(format!("Upvoted {id}. New Score: {val}"))
}

#[get("/downvote/{id}")]
async fn net_song_downvote_by_id(id: web::Path<u32>) -> Result<String, Error> {
    adb_update()?;
    let id = id.into_inner();
    let mut val = adb_uint32_read(&format!("SELECT rating FROM songs WHERE id = {id}"))?;
    if val > 0 {
        val -= 1;
        let i = &format!("Update songs set rating = {val} where id = {id}");
        adb_execute(i)?;
    }
    Ok(format!("Downvoted {id}. New Score: {val}"))
}

#[get("/*")]
async fn net_404() -> Result<String, Error> {
    Err(ErrorNotFound("No pages here."))
}

fn get_songpath_by_id(id: u32) -> Result<String, Error> {
    let i = &format!("SELECT path FROM songs WHERE id = {id}");
    adb_str_read(i)
}

fn get_songlength_secs(path: &str) -> u64 {
    let path = Path::new(path);
    let duration = mp3_duration::from_path(path).unwrap_or_default();
    duration.as_secs()
}

fn increase_times_played(id: u32) -> Result<(), Error> {
    let i = &format!("Update songs set times_played = times_played + 1 where id = {id}");
    adb_execute(i)
}

fn format_songlength(seconds: u64) -> String {
    let mins = seconds / 60;
    let secs = seconds % 60;
    if mins >= 60 {
        let hours = mins / 60;
        let mins = mins / 60;
        format!("{hours}:{mins:0>2}:{secs:0>2}")
    } else {
        format!("{mins:0>1}:{secs:0>2}")
    }
}

fn get_file_by_name(path: &str) -> Result<NamedFile, Error> {
    let p = Path::new(path);

    let file = NamedFile::open(p)?;

    let filename = p
        .file_name()
        .unwrap_or_default()
        .to_str()
        .unwrap_or_default();

    Ok(file
        .use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Inline,
            parameters: vec![DispositionParam::Filename(filename.to_owned())],
        }))
}

fn get_weighted_random_id(scale: f32) -> Result<String, Error> {
    let c = adb_con()?;

    let mut stmt = errconv(
        c.prepare("select rating, id from songs where deleted = 0 and rating > 0")
            .wrap_err("prepare query"),
    )?;

    let rows = errconv(
        stmt.query_map([], |row| {
            Ok((row.get::<usize, u32>(0), row.get::<usize, u16>(1)))
        })
        .wrap_err("convert query result"),
    )?
    .filter_map(|a| if let Ok(row) = a { Some(row) } else { None });

    let mut map: Vec<(u32, u16)> = Vec::new();

    for row in rows {
        if let Ok(a) = row.0 {
            if let Ok(b) = row.1 {
                let a = scale.powi((a - 1) as i32).round() as u32;
                map.push((a, b));
            } else {
                continue;
            };
        } else {
            continue;
        };
    }

    let mut c: u16;

    let lasts = LAST_SONGS.clone();
    let Ok(mut inner) = lasts.lock() else {
        errconv(Err(eyre!("Could not acquire mutex!")))?;
        unreachable!();
    };
    // println!("{inner:#?}");
    loop {
        c = rng(&map)?;

        if !inner.contains(&c) {
            if inner.len() >= GL_REPLAY_PROTECTION {
                inner.remove(0);
            }
            inner.push(c);
            break;
        } else {
            println!("{c} ist schon in der Liste!");
        }
    }
    drop(inner);

    Ok(c.to_string())
}

pub fn rng(map: &[(u32, u16)]) -> Result<u16, Error> {
    if let Ok(res) = WeightedIndex::new(map.iter().map(|item| item.0)) {
        Ok(map[res.sample(&mut thread_rng())].1)
    } else {
        Err(ErrorInternalServerError(
            "Error in random seletion".to_string(),
        ))
    }
}

pub fn errconv<T>(r: stable_eyre::Result<T>) -> Result<T, Error> {
    match r {
        Ok(o) => Ok(o),
        Err(e) => Err(ErrorInternalServerError(e)),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::rng;
    #[test]
    fn test_vec_rng() {
        let map = vec![
            (100u32, 100u16),
            (200u32, 200u16),
            (300u32, 300u16),
            (100u32, 400u16),
            (50u32, 500u16),
            (250u32, 600u16),
        ];

        let mut hash = HashMap::<u16, usize>::new();

        for _ in 0..1000000 {
            let out = rng(&map).unwrap_or_default();
            let val = *hash.get(&out).unwrap_or(&0);
            hash.insert(out, val + 1);
        }
        println!("{hash:#?}");
        assert!(*hash.get(&100).unwrap_or(&0) > 95_000);
        assert!(*hash.get(&100).unwrap_or(&0) < 105_000);

        assert!(*hash.get(&200).unwrap_or(&0) > 195_000);
        assert!(*hash.get(&200).unwrap_or(&0) < 205_000);

        assert!(*hash.get(&300).unwrap_or(&0) > 295_000);
        assert!(*hash.get(&300).unwrap_or(&0) < 305_000);

        assert!(*hash.get(&400).unwrap_or(&0) > 95_000);
        assert!(*hash.get(&400).unwrap_or(&0) < 105_000);

        assert!(*hash.get(&500).unwrap_or(&0) > 45_000);
        assert!(*hash.get(&500).unwrap_or(&0) < 55_000);

        assert!(*hash.get(&600).unwrap_or(&0) > 245_000);
        assert!(*hash.get(&600).unwrap_or(&0) < 255_000);
    }
}
